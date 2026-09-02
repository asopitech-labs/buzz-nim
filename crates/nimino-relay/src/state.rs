//! Shared application state — Arc-wrapped, shared across all connections.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::ws::{Message as WsMessage, Utf8Bytes as WsUtf8Bytes};
use dashmap::DashMap;
use futures_util::future::join_all;
use tokio::sync::{mpsc, watch, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use nimino_audit::AuditService;
use nimino_auth::{
    AuthService, Nip98ReplayGuard, RateLimitResult, RateLimiter, DEFAULT_REPLAY_TTL_SECS,
    MAX_REPLAY_TTL_SECS,
};
use nimino_core::tenant::TenantContext;
use nimino_core::CommunityId;
use nimino_db::Db;
#[cfg(test)]
use nimino_local_delivery::rate_limiter::LocalRateLimiter;
use nimino_local_delivery::LocalDelivery;
#[cfg(test)]
use nimino_local_delivery::LocalReplayGuard;
use nimino_media::MediaStorage;
use nimino_search::SearchService;
use nimino_workflow::WorkflowEngine;

use crate::audio::AudioRoomManager;
use crate::cluster_runtime::RelayDomainAdapters;

static AUTH_INVALIDATION_SEQUENCE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

struct ClusterReplayGuard {
    admission: nimino_control::AdmissionClient,
}

impl Nip98ReplayGuard for ClusterReplayGuard {
    fn try_mark_in_scope<'a>(
        &'a self,
        scope: &'a str,
        event_id: &'a nostr::EventId,
        ttl_secs: u64,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<bool, nimino_auth::AuthError>> + Send + 'a>,
    > {
        Box::pin(async move {
            self.admission
                .claim_replay(
                    scope,
                    event_id.to_hex(),
                    ttl_secs.clamp(DEFAULT_REPLAY_TTL_SECS, MAX_REPLAY_TTL_SECS),
                )
                .await
                .map_err(|error| nimino_auth::AuthError::Internal(error.to_string()))
        })
    }
}

pub(crate) enum AdmissionRateLimiter {
    Cluster(nimino_control::AdmissionClient),
    #[cfg(test)]
    Local(LocalRateLimiter),
}

impl RateLimiter for AdmissionRateLimiter {
    async fn check_and_increment(
        &self,
        context: &TenantContext,
        pubkey: &nostr::PublicKey,
        limit_type: nimino_auth::LimitType,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateLimitResult, nimino_auth::AuthError> {
        match self {
            Self::Cluster(admission) => admission
                .consume_rate(
                    "principal",
                    nimino_auth::rate_limit::rate_limit_key(context, pubkey, &limit_type),
                    window_secs,
                    limit,
                )
                .await
                .map(|result| RateLimitResult {
                    allowed: result.allowed,
                    current: result.current,
                    limit: result.limit,
                    reset_in_secs: result.reset_in_secs,
                })
                .map_err(|error| nimino_auth::AuthError::Internal(error.to_string())),
            #[cfg(test)]
            Self::Local(local) => {
                local
                    .check_and_increment(context, pubkey, limit_type, window_secs, limit)
                    .await
            }
        }
    }

    async fn check_ip_connection(
        &self,
        ip: &std::net::IpAddr,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateLimitResult, nimino_auth::AuthError> {
        match self {
            Self::Cluster(admission) => admission
                .consume_rate(
                    "ip",
                    nimino_auth::rate_limit::ip_rate_limit_key(ip),
                    window_secs,
                    limit,
                )
                .await
                .map(|result| RateLimitResult {
                    allowed: result.allowed,
                    current: result.current,
                    limit: result.limit,
                    reset_in_secs: result.reset_in_secs,
                })
                .map_err(|error| nimino_auth::AuthError::Internal(error.to_string())),
            #[cfg(test)]
            Self::Local(local) => local.check_ip_connection(ip, window_secs, limit).await,
        }
    }
}
use crate::config::Config;
use crate::connection::{ConnectionSubscriptions, RestartClose};
use crate::subscription::SubscriptionRegistry;

pub(crate) type ScopedPubkeyKey = (CommunityId, [u8; 32]);

/// Why a community-bound socket is being asked to stop.
///
/// Only deletion is externally attributed today. Ordinary lifecycle exits keep
/// using cancellation alone and therefore retain the existing bare-close
/// behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CommunityDisconnectReason {
    CommunityDeleted,
}

impl CommunityDisconnectReason {
    pub(crate) fn close_message(self) -> WsMessage {
        match self {
            Self::CommunityDeleted => WsMessage::Close(Some(axum::extract::ws::CloseFrame {
                code: axum::extract::ws::close_code::POLICY,
                reason: WsUtf8Bytes::from_static("community deleted"),
            })),
        }
    }
}

/// Per-socket lifecycle controls shared by the registry and the writer.
#[derive(Clone)]
pub(crate) struct CommunityConnectionControl {
    cancel: CancellationToken,
    reason_tx: watch::Sender<Option<CommunityDisconnectReason>>,
}

impl CommunityConnectionControl {
    pub(crate) fn new(cancel: CancellationToken) -> Self {
        let (reason_tx, _reason_rx) = watch::channel(None);
        Self { cancel, reason_tx }
    }

    pub(crate) fn cancellation_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub(crate) fn disconnect_reason(&self) -> watch::Receiver<Option<CommunityDisconnectReason>> {
        self.reason_tx.subscribe()
    }

    fn disconnect_community(&self) {
        self.reason_tx
            .send_replace(Some(CommunityDisconnectReason::CommunityDeleted));
        self.cancel.cancel();
    }
}

/// Leaves headroom under the process-wide drain deadline for a stalled writer.
const RESTART_CLOSE_ACK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
type SlidingWindowCounter = (u32, Instant);
type ScopedRateLimiter = DashMap<ScopedPubkeyKey, SlidingWindowCounter>;

/// Per-connection entry in the connection manager.
struct ConnEntry {
    tx: mpsc::Sender<WsMessage>,
    /// Control-frame sender, drained ahead of data and before cancel wins in
    /// the send loop. Used to deliver a ban-disconnect frame that must reach
    /// the client before the socket is closed (see [`ConnectionManager::disconnect_pubkey`]).
    ctrl_tx: mpsc::Sender<WsMessage>,
    restart_tx: Option<mpsc::Sender<RestartClose>>,
    cancel: CancellationToken,
    /// Community resolved from the connection host at handshake. This is the
    /// receiver-side tenant label fan-out must compare against the event label.
    community_id: CommunityId,
    /// Shared with `ConnectionState` — both direct sends and fan-out
    /// broadcasts track the same consecutive-full counter.
    backpressure_count: Arc<AtomicU8>,
    subscriptions: ConnectionSubscriptions,
    authenticated_pubkey: Arc<std::sync::RwLock<Option<Vec<u8>>>>,
    authenticated_owner_pubkey: Arc<std::sync::RwLock<Option<Vec<u8>>>>,
    grace_limit: u8,
}

/// Community-scoped lifecycle registry shared by every long-lived socket type.
///
/// A handler registers before durable active-state revalidation. Archival after
/// registration cancels the token; archival before registration is observed by
/// the revalidation. The returned guard removes the entry on every exit path.
pub struct CommunityConnectionRegistry {
    connections: Arc<DashMap<Uuid, (CommunityId, CommunityConnectionControl)>>,
}

impl Default for CommunityConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl CommunityConnectionRegistry {
    /// Creates an empty lifecycle registry.
    pub fn new() -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
        }
    }

    /// Registers one socket and returns a guard that deregisters it on drop.
    pub(crate) fn register(
        &self,
        connection_id: Uuid,
        community_id: CommunityId,
        control: CommunityConnectionControl,
    ) -> CommunityConnectionGuard {
        self.connections
            .insert(connection_id, (community_id, control));
        CommunityConnectionGuard {
            connection_id,
            connections: Arc::clone(&self.connections),
        }
    }

    /// Disconnects every socket type currently bound to `community_id` and
    /// attributes the close to community deletion.
    pub fn disconnect_community(&self, community_id: CommunityId) -> usize {
        let mut closed = 0;
        for entry in self.connections.iter() {
            if entry.value().0 == community_id {
                entry.value().1.disconnect_community();
                closed += 1;
            }
        }
        closed
    }

    /// Returns the distinct communities with live sockets on this pod.
    pub fn bound_communities(&self) -> HashSet<CommunityId> {
        self.connections
            .iter()
            .map(|entry| entry.value().0)
            .collect()
    }
}

/// Removes a socket lifecycle registration on every handler exit path.
pub struct CommunityConnectionGuard {
    connection_id: Uuid,
    connections: Arc<DashMap<Uuid, (CommunityId, CommunityConnectionControl)>>,
}

impl Drop for CommunityConnectionGuard {
    fn drop(&mut self) {
        self.connections.remove(&self.connection_id);
    }
}

/// Registers a socket, durably revalidates its community, then runs it.
///
/// The ordering is the archival admission invariant: archive-before-query is
/// observed by the query, while archive-after-registration sees the token.
pub(crate) async fn run_registered_community_connection<Check, CheckFuture, Run, RunFuture>(
    registry: &CommunityConnectionRegistry,
    connection_id: Uuid,
    community_id: CommunityId,
    control: CommunityConnectionControl,
    check_active: Check,
    run: Run,
) where
    Check: FnOnce() -> CheckFuture,
    CheckFuture: Future<Output = Result<bool, nimino_db::DbError>>,
    Run: FnOnce(CommunityConnectionControl) -> RunFuture,
    RunFuture: Future<Output = ()>,
{
    let cancel = control.cancel.clone();
    let _guard = registry.register(connection_id, community_id, control.clone());
    if !matches!(check_active().await, Ok(true)) {
        cancel.cancel();
        return;
    }
    if cancel.is_cancelled() {
        return;
    }
    run(control).await;
    cancel.cancel();
}

async fn revalidate_registered_communities<Check, CheckFuture>(
    registry: &CommunityConnectionRegistry,
    mut check_active: Check,
) -> (usize, Vec<(CommunityId, nimino_db::DbError)>)
where
    Check: FnMut(CommunityId) -> CheckFuture,
    CheckFuture: Future<Output = Result<bool, nimino_db::DbError>>,
{
    let communities = registry.bound_communities();
    let mut closed = 0;
    let mut failures = Vec::new();
    for community_id in communities {
        match check_active(community_id).await {
            Ok(false) => closed += registry.disconnect_community(community_id),
            Ok(true) => {}
            Err(error) => failures.push((community_id, error)),
        }
    }
    (closed, failures)
}

/// Tracks active Nostr WebSocket connections and provides message routing by connection ID.
pub struct ConnectionManager {
    connections: DashMap<Uuid, ConnEntry>,
    /// Sticky drain flag set by [`Self::drain_all`]. Registrations that land
    /// after the drain snapshot self-signal, so no upgrade-vs-shutdown
    /// interleaving can produce a connection that misses the restart close.
    draining: AtomicBool,
}

impl ConnectionManager {
    /// Creates a new, empty connection manager.
    pub fn new() -> Self {
        Self {
            connections: DashMap::new(),
            draining: AtomicBool::new(false),
        }
    }

    /// Registers a connection with its outbound sender, cancellation token,
    /// server-resolved community, shared backpressure counter, mutable
    /// subscription map, and grace limit.
    // Each argument is a distinct per-connection attribute stored verbatim in
    // `ConnEntry`; a params struct would only relocate the same fields.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn register(
        &self,
        conn_id: Uuid,
        tx: mpsc::Sender<WsMessage>,
        ctrl_tx: mpsc::Sender<WsMessage>,
        restart_tx: Option<mpsc::Sender<RestartClose>>,
        cancel: CancellationToken,
        community_id: CommunityId,
        backpressure_count: Arc<AtomicU8>,
        subscriptions: ConnectionSubscriptions,
        grace_limit: u8,
    ) {
        let drain_ctrl_tx = ctrl_tx.clone();
        let drain_cancel = cancel.clone();
        self.connections.insert(
            conn_id,
            ConnEntry {
                tx,
                ctrl_tx,
                restart_tx,
                cancel,
                community_id,
                backpressure_count,
                subscriptions,
                authenticated_pubkey: Arc::new(std::sync::RwLock::new(None)),
                authenticated_owner_pubkey: Arc::new(std::sync::RwLock::new(None)),
                grace_limit,
            },
        );
        // Insert-then-check pairs with drain_all's store-then-iterate: either
        // the drain iteration sees this entry, or this check sees the flag.
        // A registration that raced past the snapshot self-signals here, so
        // no connection can outlive graceful shutdown unclosed. A client that
        // arrives mid-shutdown should be closed at once, so the self-signal
        // always uses the immediate control-frame + cancel path regardless of
        // whether jittered drain is enabled — jitter smears the sockets that
        // were already established, not late arrivals.
        if self.draining.load(Ordering::SeqCst) {
            let _ = drain_ctrl_tx.try_send(Self::restart_close_frame());
            drain_cancel.cancel();
        }
    }

    /// Removes a connection from the registry.
    pub fn deregister(&self, conn_id: Uuid) {
        self.connections.remove(&conn_id);
    }

    /// Record the authenticated pubkey for a connection after NIP-42 succeeds.
    pub fn set_authenticated_pubkey(&self, conn_id: Uuid, pubkey_bytes: Vec<u8>) {
        self.set_authenticated_identity(conn_id, pubkey_bytes, None);
    }

    /// Records the authenticated principal and optional NIP-OA owner.
    pub fn set_authenticated_identity(
        &self,
        conn_id: Uuid,
        pubkey_bytes: Vec<u8>,
        owner_pubkey_bytes: Option<Vec<u8>>,
    ) {
        if let Some(entry) = self.connections.get(&conn_id) {
            if let Ok(mut slot) = entry.authenticated_pubkey.write() {
                *slot = Some(pubkey_bytes);
            }
            if let Ok(mut slot) = entry.authenticated_owner_pubkey.write() {
                *slot = owner_pubkey_bytes;
            }
        }
    }

    /// Return live connection IDs authenticated as `pubkey_bytes` in one community.
    ///
    /// The same Nostr key may be connected to multiple communities at once.
    /// Callers use this for tenant-visible cleanup such as presence clearing and
    /// subscription eviction, so a connection in B must not keep A's derived
    /// state alive.
    pub fn connection_ids_for_pubkey_in_community(
        &self,
        community_id: CommunityId,
        pubkey_bytes: &[u8],
    ) -> Vec<Uuid> {
        self.connections
            .iter()
            .filter_map(|entry| {
                let matches = entry.community_id == community_id
                    && entry
                        .authenticated_pubkey
                        .read()
                        .ok()
                        .and_then(|value| {
                            value
                                .as_ref()
                                .map(|stored| stored.as_slice() == pubkey_bytes)
                        })
                        .unwrap_or(false);
                matches.then_some(*entry.key())
            })
            .collect()
    }

    /// Return the authenticated pubkey recorded for a connection, if any.
    pub fn pubkey_for_conn(&self, conn_id: Uuid) -> Option<Vec<u8>> {
        self.connections
            .get(&conn_id)
            .and_then(|entry| entry.authenticated_pubkey.read().ok()?.clone())
    }

    /// Snapshots distinct authenticated identities with their resolved community.
    pub fn authenticated_identities(&self) -> Vec<(CommunityId, Vec<u8>, Option<Vec<u8>>)> {
        self.connections
            .iter()
            .filter_map(|entry| {
                entry
                    .authenticated_pubkey
                    .read()
                    .ok()
                    .and_then(|pubkey| pubkey.clone())
                    .map(|pubkey| {
                        let owner = entry
                            .authenticated_owner_pubkey
                            .read()
                            .ok()
                            .and_then(|owner| owner.clone());
                        (entry.community_id, pubkey, owner)
                    })
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    }

    /// Disconnect every live connection authenticated as `pubkey`, or as its
    /// NIP-OA agent, **in `community`**. Delivers a final `OK false` frame
    /// carrying `reason` before closing.
    ///
    /// Used for live ban enforcement (COMMUNITY_MODERATION_PLAN.md §0 decision
    /// 4): a ban must take effect immediately on existing sessions, not just at
    /// the next auth. The frame is sent on the control channel, which the send
    /// loop drains ahead of both queued data and the biased cancel branch, so
    /// the client learns *why* it was dropped. `event_id` labels the `OK` (the
    /// ban has no triggering client event, so a synthetic all-zero id is used).
    ///
    /// The `community` filter is the tenant fence: one pod holds sockets for
    /// many communities, and the same pubkey may be live in several. A ban in
    /// community A must close only A's sockets, never a session the member holds
    /// in community B ("authority stays inside the tenant fence").
    ///
    /// Returns the number of process-local connections closed.
    pub fn disconnect_pubkey(
        &self,
        community: CommunityId,
        pubkey: &[u8],
        event_id: &str,
        reason: &str,
    ) -> usize {
        let frame = crate::protocol::RelayMessage::ok(event_id, false, reason);
        let mut closed = 0usize;
        let conn_ids = self
            .connections
            .iter()
            .filter_map(|entry| {
                if entry.community_id != community {
                    return None;
                }
                let principal_matches = entry
                    .authenticated_pubkey
                    .read()
                    .ok()
                    .and_then(|value| value.as_ref().map(|stored| stored.as_slice() == pubkey))
                    .unwrap_or(false);
                let owner_matches = entry
                    .authenticated_owner_pubkey
                    .read()
                    .ok()
                    .and_then(|value| value.as_ref().map(|stored| stored.as_slice() == pubkey))
                    .unwrap_or(false);
                (principal_matches || owner_matches).then_some(*entry.key())
            })
            .collect::<Vec<_>>();
        for conn_id in conn_ids {
            if let Some(entry) = self.connections.get(&conn_id) {
                if entry.community_id != community {
                    continue;
                }
                // Best-effort delivery: a full control buffer still gets the
                // close via cancel below, just without the reason frame.
                let _ = entry
                    .ctrl_tx
                    .try_send(WsMessage::Text(frame.clone().into()));
                entry.cancel.cancel();
                closed += 1;
            }
        }
        closed
    }

    /// Closes every live connection with a `1012 Service Restart` close frame.
    ///
    /// This is the original, all-at-once drain, retained as the default path
    /// (`NIMINO_DRAIN_JITTER_MS` unset or `0`). It is synchronous and returns as
    /// soon as every close is queued and every connection cancelled, so the
    /// caller's hard-drain timeout backstops delivery unchanged.
    ///
    /// Called when graceful shutdown starts draining. Without this, upgraded
    /// WebSocket connections outlive the axum listener drain: clients ride the
    /// dying pod until the forced exit and then learn about the restart from a
    /// TCP reset (or, on an abrupt kill, from up to 60s of stall-watchdog
    /// silence). The explicit close frame tells them to reconnect immediately
    /// — and that the disconnect is a restart, not a policy action.
    ///
    /// Uses the "queue frame on ctrl, then cancel" idiom (see
    /// [`ConnectionManager::disconnect_pubkey`]): the send loop drains queued
    /// control frames — including this close — before its cancel branch closes
    /// the socket. Best-effort: a full control buffer still gets the close via
    /// cancel, just without the restart code.
    ///
    /// Returns the number of connections signalled.
    pub fn drain_all(&self) -> usize {
        // Store-then-iterate pairs with register's insert-then-check: a
        // registration that misses this iteration observes the flag and
        // self-signals instead. The flag is sticky — drain is one-way.
        self.draining.store(true, Ordering::SeqCst);
        let frame = Self::restart_close_frame();
        let mut closed = 0usize;
        for entry in self.connections.iter() {
            let _ = entry.ctrl_tx.try_send(frame.clone());
            entry.cancel.cancel();
            closed += 1;
        }
        closed
    }

    /// Closes every live connection with a `1012 Service Restart` frame,
    /// spreading closes across `[1, jitter_ms]`.
    ///
    /// This is the jittered drain, used only when `NIMINO_DRAIN_JITTER_MS > 0`.
    /// It is kept deliberately separate from [`Self::drain_all`] so that the
    /// default (jitter-off) shutdown path is byte-for-byte the previously
    /// shipped behavior; the new close-acknowledgement machinery only runs when
    /// jitter is explicitly enabled. Once the jittered path is proven in
    /// production for all cases, the two can be unified and the old one dropped.
    ///
    /// A pod under a rolling deploy can hold thousands of WebSocket sessions.
    /// Closing them simultaneously ([`Self::drain_all`]) makes every client
    /// reconnect at the same moment — a thundering herd that drives the DB
    /// pool-timeout bursts observed on each roll. Delaying each connection's
    /// close by an independent uniform random offset in `[1, jitter_ms]`
    /// smears the reconnects across the window while keeping the well-attributed
    /// 1012 close.
    ///
    /// Each delayed close is delivered over the connection's dedicated
    /// [`RestartClose`] channel: the writer flushes the 1012 frame and
    /// acknowledges the flush, so drain waits for confirmed delivery (up to
    /// [`RESTART_CLOSE_ACK_TIMEOUT`]) rather than assuming it. If the channel is
    /// full/closed or the ack times out, drain falls back to cancellation.
    ///
    /// The sticky drain flag is set before the first await, preserving
    /// [`Self::drain_all`]'s shutdown-boundary race guarantee: a registration
    /// that lands after the snapshot self-signals immediately (no jitter — a
    /// client arriving mid-shutdown should be closed at once). The returned
    /// future owns every delayed close, so the caller must await it before the
    /// relay runtime is allowed to stop.
    ///
    /// Returns the number of connections signalled.
    pub async fn drain_all_jittered(&self, jitter_ms: u64) -> usize {
        // Store-then-snapshot pairs with register's insert-then-check: either
        // the snapshot captures a registration, or it observes the sticky flag
        // and self-signals immediately.
        self.draining.store(true, Ordering::SeqCst);
        let jitter_ms = jitter_ms.max(1);
        let pending: Vec<_> = self
            .connections
            .iter()
            .map(|entry| {
                let ctrl_tx = entry.ctrl_tx.clone();
                let restart_tx = entry.restart_tx.clone();
                let cancel = entry.cancel.clone();
                let delay_ms = 1 + rand::random::<u64>() % jitter_ms;
                async move {
                    tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
                    let Some(restart_tx) = restart_tx else {
                        // Unit-only registrations do not own a writer task.
                        let _ = ctrl_tx.try_send(Self::restart_close_frame());
                        cancel.cancel();
                        return;
                    };
                    let (flushed_tx, flushed_rx) = tokio::sync::oneshot::channel();
                    if restart_tx
                        .try_send(RestartClose {
                            flushed: flushed_tx,
                        })
                        .is_err()
                    {
                        cancel.cancel();
                        return;
                    }
                    let flushed = tokio::time::timeout(RESTART_CLOSE_ACK_TIMEOUT, flushed_rx).await;
                    if !matches!(flushed, Ok(Ok(true))) {
                        cancel.cancel();
                    }
                }
            })
            .collect();
        let count = pending.len();
        join_all(pending).await;
        count
    }

    /// The WS close frame announcing a graceful restart: 1012 Service Restart.
    fn restart_close_frame() -> WsMessage {
        WsMessage::Close(Some(axum::extract::ws::CloseFrame {
            code: axum::extract::ws::close_code::RESTART,
            reason: axum::extract::ws::Utf8Bytes::from_static("relay restarting"),
        }))
    }

    /// Return the server-resolved community that the connection's host bound to.
    pub fn community_for_conn(&self, conn_id: Uuid) -> Option<CommunityId> {
        self.connections
            .get(&conn_id)
            .map(|entry| entry.community_id)
    }

    /// Return the subscription map for a connection, if it is still live.
    pub fn subscriptions_for(&self, conn_id: Uuid) -> Option<ConnectionSubscriptions> {
        self.connections
            .get(&conn_id)
            .map(|entry| Arc::clone(&entry.subscriptions))
    }

    /// Snapshot the number of live WebSocket connections per community.
    ///
    /// Returns a map from community UUID to connection count. Used by the
    /// usage poller; snapshotting avoids per-community gauge drift from
    /// mismatched inc/dec across async boundaries.
    pub fn per_community_ws_connections(&self) -> HashMap<CommunityId, u64> {
        let mut counts: HashMap<CommunityId, u64> = HashMap::new();
        for entry in self.connections.iter() {
            *counts.entry(entry.community_id).or_default() += 1;
        }
        counts
    }

    /// Snapshot the number of distinct authenticated pubkeys online per community.
    ///
    /// A pubkey connected to multiple pods will be counted once per pod — the
    /// dashboard sums across pods, so per-pod partial counts are correct.
    /// A pubkey connected twice on the same pod is counted once (distinct set).
    pub fn per_community_users_online(&self) -> HashMap<CommunityId, u64> {
        // community_id → set of pubkey bytes
        let mut seen: HashMap<CommunityId, HashSet<Vec<u8>>> = HashMap::new();
        for entry in self.connections.iter() {
            if let Ok(lock) = entry.authenticated_pubkey.read() {
                if let Some(pk) = lock.as_ref() {
                    seen.entry(entry.community_id)
                        .or_default()
                        .insert(pk.clone());
                }
            }
        }
        seen.into_iter()
            .map(|(cid, set)| (cid, set.len() as u64))
            .collect()
    }

    /// Return the authenticated pubkey for a connection, if any.
    pub fn pubkey_for(&self, conn_id: Uuid) -> Option<Vec<u8>> {
        self.connections
            .get(&conn_id)
            .and_then(|entry| entry.authenticated_pubkey.read().ok()?.clone())
    }

    /// Sends a text message to the given connection.
    ///
    /// Returns `false` if the connection is gone or the buffer is full.
    /// On sustained backpressure (>grace_limit consecutive full buffers),
    /// cancels the connection. Transient stalls get a warning only.
    pub fn send_to(&self, conn_id: Uuid, msg: String) -> bool {
        self.try_send_ws_message(conn_id, WsMessage::Text(msg.into()))
    }

    /// Sends an already-serialized UTF-8 text payload to the given connection.
    ///
    /// The shared `Bytes` payload is cloned into the outbound WS message without
    /// copying the frame body. Callers must only pass valid UTF-8 bytes.
    pub fn send_to_text_bytes(&self, conn_id: Uuid, msg: Arc<Bytes>) -> bool {
        let text = WsUtf8Bytes::try_from(Bytes::clone(msg.as_ref()))
            .expect("relay fan-out frames are serialized UTF-8 JSON");
        self.try_send_ws_message(conn_id, WsMessage::Text(text))
    }

    fn try_send_ws_message(&self, conn_id: Uuid, msg: WsMessage) -> bool {
        if let Some(entry) = self.connections.get(&conn_id) {
            let conn = entry.value();
            match conn.tx.try_send(msg) {
                Ok(_) => {
                    conn.backpressure_count.store(0, Ordering::Relaxed);
                    true
                }
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    let count = conn.backpressure_count.fetch_add(1, Ordering::Relaxed) + 1;
                    if count >= conn.grace_limit {
                        tracing::warn!(conn_id = %conn_id, count, "fan-out: sustained backpressure — cancelling slow client");
                        metrics::counter!("nimino_ws_backpressure_disconnects_total").increment(1);
                        conn.cancel.cancel();
                    } else {
                        tracing::warn!(conn_id = %conn_id, count, grace = conn.grace_limit, "fan-out: send buffer full — grace {count}/{}", conn.grace_limit);
                    }
                    false
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    tracing::debug!(conn_id = %conn_id, "fan-out: send channel closed");
                    false
                }
            }
        } else {
            false
        }
    }
}

impl Default for ConnectionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Shared application state, cloned cheaply via inner `Arc` fields.
#[derive(Clone)]
pub struct AppState {
    /// Relay configuration.
    pub config: Arc<Config>,
    /// Database connection pool.
    pub db: Db,
    /// Mandatory Nim policy and per-node canonical persistence adapters.
    pub domain: RelayDomainAdapters,
    /// Audit event service, absent when audit logging is disabled.
    pub audit: Option<Arc<AuditService>>,
    /// Pub/sub manager for broadcasting events to subscribers.
    pub local_delivery: Arc<LocalDelivery>,
    /// Authentication service.
    pub auth: Arc<AuthService>,
    /// Full-text search service.
    pub search: Arc<SearchService>,
    /// Registry of active client subscriptions.
    pub sub_registry: Arc<SubscriptionRegistry>,
    /// Registry of active WebSocket connections.
    pub conn_manager: Arc<ConnectionManager>,
    /// Lifecycle cancellation for every long-lived socket, including huddle audio.
    pub community_connections: Arc<CommunityConnectionRegistry>,
    /// Stops only the periodic lifecycle revalidator during graceful shutdown.
    pub community_revalidator_cancel: CancellationToken,
    /// Test/telemetry counter for archive disconnect publication attempts.
    /// Semaphore limiting total concurrent connections.
    pub conn_semaphore: Arc<Semaphore>,
    /// Semaphore limiting concurrent message handler tasks.
    pub handler_semaphore: Arc<Semaphore>,
    /// Semaphore limiting concurrent git subprocess operations across
    /// the whole relay. Bounds resource use; **not** writer
    /// serialization — that's the CAS at the manifest pointer (spec
    /// §Push step 7, `Inv_NoFork`).
    pub git_semaphore: Arc<Semaphore>,
    /// Semaphore limiting concurrent media upload parsing/transcoding work.
    pub media_upload_semaphore: Arc<Semaphore>,

    /// Workflow engine for background processing.
    pub workflow_engine: Arc<WorkflowEngine>,
    /// Relay signing keypair — used to sign system messages (kind 40099).
    pub relay_keypair: nostr::Keys,

    /// Membership cache: (community_id, channel_id, pubkey_bytes) → is_member.
    /// Short TTL (10s) — membership changes are rare but must propagate.
    #[allow(clippy::type_complexity)]
    pub membership_cache: Arc<moka::sync::Cache<(CommunityId, Uuid, Vec<u8>), bool>>,
    /// Accessible channel IDs cache: (community_id, pubkey_bytes) → channel UUIDs.
    /// Short TTL (10s) — invalidated on membership or channel visibility changes.
    #[allow(clippy::type_complexity)]
    pub accessible_channels_cache: Arc<moka::sync::Cache<(CommunityId, Vec<u8>), Vec<Uuid>>>,
    /// Per-community channel visibility string, used to gate the private-channel fan-out
    /// access check so open channels stay zero-cost. Invalidated on a flip.
    pub channel_visibility_cache: Arc<moka::sync::Cache<(CommunityId, Uuid), String>>,

    /// Bounded channel for audit logging, absent when audit logging is disabled.
    pub audit_tx: Option<mpsc::Sender<nimino_audit::NewAuditEntry>>,
    /// Media storage client (S3/MinIO).
    pub media_storage: Arc<MediaStorage>,
    /// Single-flight + cache state for the hourly S3 storage sweep. See
    /// `storage_sweep` module docs; shared with the usage-metrics tick via
    /// `Arc` the same way other cross-tick poller state lives on `AppState`.
    pub storage_sweep: Arc<tokio::sync::Mutex<crate::storage_sweep::StorageSweepState>>,
    /// Git object-store backend (content-addressed packs/manifests plus
    /// CAS-guarded manifest pointer). This is the durable git source of truth;
    /// see `api::git::store` and `docs/git-on-object-storage.md`.
    pub git_store: crate::api::git::store::GitStore,
    /// Process-local, byte-bounded cache of immutable Git pack/index pairs.
    /// Object storage remains authoritative; this only avoids repeated reads
    /// and index generation for content-addressed packs.
    pub git_pack_cache: Arc<crate::api::git::pack_cache::GitPackCache>,
    /// Audio relay room manager — tracks active huddle audio rooms.
    pub audio_rooms: Arc<AudioRoomManager>,
    /// Set to `true` on SIGTERM — readiness probe returns 503.
    pub shutting_down: Arc<AtomicBool>,
    /// True only while the mandatory Nim/Chirps synchronization runtime is live.
    pub cluster_ready: Arc<AtomicBool>,
    /// Process start time — used by `/_status` endpoint.
    pub started_at: Instant,
    /// Quorum-owned, community-scoped NIP-98 replay prevention.
    pub nip98_replay: Arc<dyn Nip98ReplayGuard>,
    /// Quorum-owned admission limits; tests may compose the local fallback.
    pub(crate) admission_rate_limiter: Arc<AdmissionRateLimiter>,

    /// Per-agent sliding-window rate limiter for observer frames (kind 24200).
    /// Key: (community_id, agent pubkey bytes). Value: (count, window_start).
    /// 100 events/sec per agent — prevents relay/DB pressure from bursty telemetry.
    pub observer_rate_limiter: Arc<ScopedRateLimiter>,
    /// Per-uploader sliding-window rate limiter for media upload starts.
    /// Key: (community_id, uploader pubkey bytes). Value: (count, window_start).
    pub media_upload_rate_limiter: Arc<ScopedRateLimiter>,
    /// Per-claimer fixed-window rate limiter for invite claim attempts
    /// (`POST /api/invites/claim`). Entries expire after the claim window and
    /// the cache has a hard capacity because pre-membership callers can cheaply
    /// generate fresh Nostr keys.
    pub invite_claim_rate_limiter:
        Arc<moka::sync::Cache<ScopedPubkeyKey, Arc<std::sync::atomic::AtomicU32>>>,
    /// Current in-flight media uploads per (community, uploader pubkey).
    pub media_uploads_in_flight: Arc<DashMap<ScopedPubkeyKey, u32>>,
    /// Cache for observer agent-owner authorization (kind 24200).
    /// Key: (community_id, agent_pubkey_bytes, owner_pubkey_bytes). Value: is_owner.
    /// `agent_owner_pubkey` is immutable inside one community, so a long TTL
    /// (5 min) is safe once the community label is part of the key.
    /// Prevents repeated DB lookups from bursty observer traffic.
    #[allow(clippy::type_complexity)]
    pub observer_owner_cache: Arc<moka::sync::Cache<(CommunityId, Vec<u8>, Vec<u8>), bool>>,
    /// Cache for the `author_type` metric label on the ingest path.
    /// Key: (community_id, author pubkey bytes). Value: is_agent
    /// (`users.agent_owner_pubkey IS NOT NULL`). The mapping is
    /// first-write-wins and set during auth before an agent's first event,
    /// so a short TTL only bounds staleness for the rare backfill race.
    pub author_type_cache: Arc<moka::sync::Cache<(CommunityId, Vec<u8>), bool>>,

    /// Runtime conformance tracer. Production binds [`crate::conformance::NoopTracer`]
    /// (zero cost). Conformance tests bind [`crate::conformance::JsonlTracer`] to
    /// record traces for replay against `docs/spec/MultiTenantRelay.tla`.
    /// See `crates/nimino-conformance/` and `crate::conformance` for the
    /// schema, emitter helpers, and the independent checker.
    pub tracer: Arc<dyn nimino_conformance::Tracer>,
}

impl AppState {
    /// Projects and advertises one verified presence event.
    pub async fn publish_presence_transition(
        &self,
        tenant: &TenantContext,
        pubkey: &nostr::PublicKey,
        status: &str,
        event: &nostr::Event,
    ) -> Result<(), nimino_control::EphemeralRuntimeError> {
        if let Some(ephemeral) = self.domain.ephemeral() {
            let observed_at_ms = event.created_at.as_secs().saturating_mul(1_000);
            if status == "offline" {
                ephemeral
                    .clear_presence_at(
                        tenant.community().to_string(),
                        pubkey.to_hex(),
                        observed_at_ms,
                        event.id.to_hex(),
                    )
                    .await?;
            } else {
                ephemeral
                    .publish_presence(
                        tenant.community().to_string(),
                        pubkey.to_hex(),
                        status,
                        observed_at_ms,
                        event.id.to_hex(),
                        serde_json::to_string(event).map_err(|error| {
                            nimino_control::EphemeralRuntimeError::InvalidFrame(error.to_string())
                        })?,
                    )
                    .await?;
            }
        } else if status == "offline" {
            self.local_delivery.clear_presence(tenant, pubkey).await;
        } else {
            self.local_delivery
                .set_presence(tenant, pubkey, status)
                .await;
        }
        Ok(())
    }

    /// Projects and advertises one verified typing event.
    pub async fn publish_typing_transition(
        &self,
        tenant: &TenantContext,
        pubkey: &nostr::PublicKey,
        channel_id: Uuid,
        event: &nostr::Event,
    ) -> Result<(), nimino_control::EphemeralRuntimeError> {
        if let Some(ephemeral) = self.domain.ephemeral() {
            ephemeral
                .publish_typing(
                    tenant.community().to_string(),
                    pubkey.to_hex(),
                    channel_id.to_string(),
                    "typing",
                    event.created_at.as_secs().saturating_mul(1_000),
                    event.id.to_hex(),
                    serde_json::to_string(event).map_err(|error| {
                        nimino_control::EphemeralRuntimeError::InvalidFrame(error.to_string())
                    })?,
                )
                .await?;
        }
        Ok(())
    }

    /// Clears this relay node's presence contribution after its last local socket closes.
    pub async fn clear_presence(
        &self,
        tenant: &TenantContext,
        pubkey: &nostr::PublicKey,
    ) -> Result<(), nimino_control::EphemeralRuntimeError> {
        if let Some(ephemeral) = self.domain.ephemeral() {
            ephemeral
                .clear_presence(tenant.community().to_string(), pubkey.to_hex())
                .await?;
        } else {
            self.local_delivery.clear_presence(tenant, pubkey).await;
        }
        Ok(())
    }

    /// Returns converged live presence values for one resolved community.
    pub async fn presence_bulk(
        &self,
        tenant: &TenantContext,
        pubkeys: &[nostr::PublicKey],
    ) -> HashMap<String, String> {
        if let Some(ephemeral) = self.domain.ephemeral() {
            match ephemeral
                .presence(
                    tenant.community().to_string(),
                    pubkeys.iter().map(nostr::PublicKey::to_hex).collect(),
                )
                .await
            {
                Ok(presence) => presence,
                Err(error) => {
                    tracing::warn!(%error, "cluster presence query failed closed");
                    HashMap::new()
                }
            }
        } else {
            self.local_delivery.get_presence_bulk(tenant, pubkeys).await
        }
    }

    /// Constructs `AppState` from its component services.
    ///
    /// Returns `(state, audit_shutdown)`. The caller should call
    /// `audit_shutdown.drain().await` during graceful shutdown so queued
    /// audit entries are flushed before the process exits.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_domain(
        config: Config,
        db: Db,
        audit: impl Into<Option<AuditService>>,
        local_delivery: Arc<LocalDelivery>,
        auth: AuthService,
        search: SearchService,
        workflow_engine: Arc<WorkflowEngine>,
        relay_keypair: nostr::Keys,
        media_storage: MediaStorage,
        domain: RelayDomainAdapters,
        cluster_ready: Arc<AtomicBool>,
    ) -> (Self, AuditShutdownHandle) {
        let max_connections = config.max_connections;
        let max_concurrent_handlers = config.max_concurrent_handlers;
        let search_arc = Arc::new(search);

        let audit_arc = audit.into().map(Arc::new);
        let (audit_tx, mut audit_rx) = mpsc::channel::<nimino_audit::NewAuditEntry>(1000);
        let audit_for_worker = audit_arc.clone();
        let audit_cancel = CancellationToken::new();
        let audit_cancel_worker = audit_cancel.clone();
        let audit_worker_handle = tokio::spawn(async move {
            let Some(audit_for_worker) = audit_for_worker else {
                audit_cancel_worker.cancelled().await;
                return;
            };
            // Normal operation: process entries as they arrive.
            loop {
                tokio::select! {
                    entry = audit_rx.recv() => {
                        match entry {
                            Some(entry) => log_audit_entry(&audit_for_worker, entry).await,
                            None => break, // channel closed
                        }
                    }
                    _ = audit_cancel_worker.cancelled() => {
                        // Close the receiver: rejects future sends and lets us
                        // drain everything already buffered without a race.
                        audit_rx.close();
                        break;
                    }
                }
            }
            // Drain: recv() returns buffered entries, then None once empty.
            let mut drained = 0u32;
            while let Some(entry) = audit_rx.recv().await {
                log_audit_entry(&audit_for_worker, entry).await;
                drained += 1;
            }
            if drained > 0 {
                tracing::info!(drained, "audit worker flushed remaining entries");
            }
            tracing::warn!("audit log worker exited (expected on shutdown)");
        });

        let git_max_concurrent_ops = config.git_max_concurrent_ops;
        let media_max_concurrent_uploads = config.media_max_concurrent_uploads;
        let git_store = crate::api::git::store::GitStore::new(
            &config.media.s3_endpoint,
            &config.media.s3_access_key,
            &config.media.s3_secret_key,
            &config.media.s3_bucket,
            &config.media.s3_region,
            config.media.s3_addressing_style,
        )
        .expect("media storage was already constructed with this S3 config");
        let git_pack_cache = Arc::new(
            crate::api::git::pack_cache::GitPackCache::new(
                &config.git_pack_cache_path,
                config.git_pack_cache_max_bytes,
                config.git_pack_cache_max_concurrent_populations,
            )
            .expect("git pack cache path must be available"),
        );
        #[cfg(not(test))]
        let nip98_replay: Arc<dyn Nip98ReplayGuard> = Arc::new(ClusterReplayGuard {
            admission: domain.admission().clone(),
        });
        #[cfg(test)]
        let nip98_replay: Arc<dyn Nip98ReplayGuard> = match domain.admission_for_tests().cloned() {
            Some(admission) => Arc::new(ClusterReplayGuard { admission }),
            None => Arc::new(LocalReplayGuard::new()),
        };
        #[cfg(not(test))]
        let admission_rate_limiter =
            Arc::new(AdmissionRateLimiter::Cluster(domain.admission().clone()));
        #[cfg(test)]
        let admission_rate_limiter = Arc::new(match domain.admission_for_tests().cloned() {
            Some(admission) => AdmissionRateLimiter::Cluster(admission),
            None => AdmissionRateLimiter::Local(LocalRateLimiter::new()),
        });
        let audit_enabled = audit_arc.is_some();
        let state = Self {
            config: Arc::new(config),
            db,
            domain,
            audit: audit_arc,
            local_delivery,
            auth: Arc::new(auth),
            search: search_arc,
            sub_registry: Arc::new(SubscriptionRegistry::new()),
            conn_manager: Arc::new(ConnectionManager::new()),
            community_connections: Arc::new(CommunityConnectionRegistry::new()),
            community_revalidator_cancel: CancellationToken::new(),
            conn_semaphore: Arc::new(Semaphore::new(max_connections)),
            handler_semaphore: Arc::new(Semaphore::new(max_concurrent_handlers)),
            git_semaphore: Arc::new(Semaphore::new(git_max_concurrent_ops)),
            media_upload_semaphore: Arc::new(Semaphore::new(media_max_concurrent_uploads)),
            workflow_engine,
            relay_keypair,

            membership_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(std::time::Duration::from_secs(10))
                    .support_invalidation_closures()
                    .build(),
            ),
            accessible_channels_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(std::time::Duration::from_secs(10))
                    .support_invalidation_closures()
                    .build(),
            ),
            channel_visibility_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(std::time::Duration::from_secs(10))
                    .support_invalidation_closures()
                    .build(),
            ),
            audit_tx: audit_enabled.then_some(audit_tx),
            media_storage: Arc::new(media_storage),
            storage_sweep: Arc::new(tokio::sync::Mutex::new(
                crate::storage_sweep::StorageSweepState::default(),
            )),
            git_store,
            git_pack_cache,
            audio_rooms: Arc::new(AudioRoomManager::new()),
            shutting_down: Arc::new(AtomicBool::new(false)),
            cluster_ready,
            started_at: Instant::now(),
            nip98_replay,
            admission_rate_limiter,
            observer_rate_limiter: Arc::new(DashMap::new()),
            media_upload_rate_limiter: Arc::new(DashMap::new()),
            invite_claim_rate_limiter: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(crate::api::invites::CLAIM_RATE_CACHE_CAPACITY)
                    .time_to_live(crate::api::invites::CLAIM_RATE_WINDOW)
                    .build(),
            ),
            media_uploads_in_flight: Arc::new(DashMap::new()),
            observer_owner_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(1_000)
                    .time_to_live(std::time::Duration::from_secs(300))
                    .build(),
            ),
            author_type_cache: Arc::new(
                moka::sync::Cache::builder()
                    .max_capacity(10_000)
                    .time_to_live(std::time::Duration::from_secs(300))
                    .build(),
            ),
            // Default to NoopTracer: production builds pay zero cost.
            // Conformance tests overwrite this with a JsonlTracer after
            // construction (see test helpers in
            // `crates/nimino-test-client` once those land).
            tracer: Arc::new(crate::conformance::NoopTracer),
        };
        (
            state,
            AuditShutdownHandle {
                cancel: audit_cancel,
                handle: audit_worker_handle,
            },
        )
    }

    #[cfg(test)]
    /// Constructs state for adapter-focused tests that do not execute domain policy.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: Config,
        db: Db,
        audit: impl Into<Option<AuditService>>,
        local_delivery: Arc<LocalDelivery>,
        auth: AuthService,
        search: SearchService,
        workflow_engine: Arc<WorkflowEngine>,
        relay_keypair: nostr::Keys,
        media_storage: MediaStorage,
    ) -> (Self, AuditShutdownHandle) {
        Self::new_with_domain(
            config,
            db,
            audit,
            local_delivery,
            auth,
            search,
            workflow_engine,
            relay_keypair,
            media_storage,
            RelayDomainAdapters::unavailable_for_tests(),
            Arc::new(AtomicBool::new(false)),
        )
    }

    /// Check channel membership with a 10-second cache. Falls back to DB on miss.
    pub async fn is_member_cached(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) -> Result<bool, nimino_db::DbError> {
        let key = (community_id, channel_id, pubkey.to_vec());
        if let Some(cached) = self.membership_cache.get(&key) {
            metrics::counter!("nimino_membership_cache_hits_total").increment(1);
            return Ok(cached);
        }
        metrics::counter!("nimino_membership_cache_misses_total").increment(1);
        let result = self.db.is_member(community_id, channel_id, pubkey).await?;
        self.membership_cache.insert(key, result);
        Ok(result)
    }

    /// Invalidate caches after a membership change (add/remove member).
    ///
    /// Drops the process-local derived entries. Durable membership remains the
    /// authority and is rechecked on cache misses.
    pub fn invalidate_membership(&self, tenant: &TenantContext, channel_id: Uuid, pubkey: &[u8]) {
        self.invalidate_membership_local(tenant.community(), channel_id, pubkey);
        self.publish_authorization_invalidation(
            tenant.community(),
            nimino_boundary::AuthorizationInvalidationKind::Membership,
            hex::encode(pubkey),
            channel_id.to_string(),
            None,
        );
    }

    /// Process-local membership drop.
    pub(crate) fn invalidate_membership_local(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        pubkey: &[u8],
    ) {
        self.membership_cache
            .invalidate(&(community_id, channel_id, pubkey.to_vec()));
        self.accessible_channels_cache
            .invalidate(&(community_id, pubkey.to_vec()));
    }

    /// Invalidate all users' accessible-channels cache (e.g. new open channel created).
    pub fn invalidate_all_accessible_channels(&self, tenant: &TenantContext) {
        self.invalidate_all_accessible_channels_local(tenant.community());
    }

    /// Local-only accessible-channels drop. See [`invalidate_membership_local`].
    pub(crate) fn invalidate_all_accessible_channels_local(&self, community_id: CommunityId) {
        if let Err(error) = self
            .accessible_channels_cache
            .invalidate_entries_if(move |(entry_community, _), _| *entry_community == community_id)
        {
            // AppState enables invalidation closures at construction time. If
            // that invariant ever regresses, prefer over-invalidating to
            // serving stale access state.
            tracing::error!(
                ?error,
                "community-scoped accessible-channel invalidation unavailable; falling back to full invalidation"
            );
            self.accessible_channels_cache.invalidate_all();
        }
    }

    /// Invalidate the cached visibility for a single channel (e.g. after a flip).
    pub fn invalidate_channel_visibility(&self, tenant: &TenantContext, channel_id: Uuid) {
        self.invalidate_channel_visibility_local(tenant.community(), channel_id);
        self.publish_authorization_invalidation(
            tenant.community(),
            nimino_boundary::AuthorizationInvalidationKind::Visibility,
            String::new(),
            channel_id.to_string(),
            None,
        );
    }

    /// Local-only visibility drop. See [`invalidate_membership_local`].
    pub(crate) fn invalidate_channel_visibility_local(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
    ) {
        self.channel_visibility_cache
            .invalidate(&(community_id, channel_id));
    }

    /// Invalidate all caches after a channel is deleted.
    ///
    /// Channel deletion is a rare admin operation, but it is still tenant-local:
    /// a deletion in A must not flush B's cache entries. Predicate invalidation
    /// keeps the safety property that stale `is_member=true` entries for the
    /// deleted channel are removed without turning the cache drop into a
    /// cross-community signal.
    pub fn invalidate_channel_deleted(&self, tenant: &TenantContext) {
        self.invalidate_channel_deleted_local(tenant.community());
        self.publish_authorization_invalidation(
            tenant.community(),
            nimino_boundary::AuthorizationInvalidationKind::Community,
            String::new(),
            String::new(),
            None,
        );
    }

    /// Local-only channel-deleted drop. See [`invalidate_membership_local`].
    pub(crate) fn invalidate_channel_deleted_local(&self, community_id: CommunityId) {
        if let Err(error) =
            self.membership_cache
                .invalidate_entries_if(move |(entry_community, _, _), _| {
                    *entry_community == community_id
                })
        {
            tracing::error!(
                ?error,
                "community-scoped membership invalidation unavailable; falling back to full invalidation"
            );
            self.membership_cache.invalidate_all();
        }
        if let Err(error) = self
            .accessible_channels_cache
            .invalidate_entries_if(move |(entry_community, _), _| *entry_community == community_id)
        {
            tracing::error!(
                ?error,
                "community-scoped accessible-channel invalidation unavailable; falling back to full invalidation"
            );
            self.accessible_channels_cache.invalidate_all();
        }
        if let Err(error) = self
            .channel_visibility_cache
            .invalidate_entries_if(move |(entry_community, _), _| *entry_community == community_id)
        {
            tracing::error!(
                ?error,
                "community-scoped visibility invalidation unavailable; falling back to full invalidation"
            );
            self.channel_visibility_cache.invalidate_all();
        }
    }

    /// Closes this process's sockets for a banned principal.
    ///
    /// The durable ban row remains authoritative for subsequent authentication
    /// and writes. Cluster propagation belongs to the Nim domain sync path.
    pub fn disconnect_pubkey(
        &self,
        tenant: &TenantContext,
        pubkey: &[u8],
        event_id: &str,
        reason: &str,
    ) -> usize {
        let closed =
            self.conn_manager
                .disconnect_pubkey(tenant.community(), pubkey, event_id, reason);
        self.publish_authorization_invalidation(
            tenant.community(),
            nimino_boundary::AuthorizationInvalidationKind::Ban,
            hex::encode(pubkey),
            String::new(),
            Some(event_id.to_owned()),
        );
        closed
    }

    /// Closes this process's sockets for an archived community.
    pub fn disconnect_community(&self, tenant: &TenantContext) -> usize {
        let closed = self
            .community_connections
            .disconnect_community(tenant.community());
        self.publish_authorization_invalidation(
            tenant.community(),
            nimino_boundary::AuthorizationInvalidationKind::Community,
            String::new(),
            String::new(),
            None,
        );
        closed
    }

    fn publish_authorization_invalidation(
        &self,
        community: CommunityId,
        kind: nimino_boundary::AuthorizationInvalidationKind,
        subject: String,
        channel_id: String,
        fact_id: Option<String>,
    ) {
        let Some(admission) = self.domain.admission_optional().cloned() else {
            return;
        };
        let sequence = AUTH_INVALIDATION_SEQUENCE.fetch_add(1, Ordering::Relaxed) + 1;
        let fact_id = fact_id
            .unwrap_or_else(|| format!("{}:{sequence}", self.domain.node_id().unwrap_or("local")));
        tokio::spawn(async move {
            if let Err(error) = admission
                .publish_invalidation(community.to_string(), kind, subject, channel_id, fact_id)
                .await
            {
                tracing::warn!(%error, %community, ?kind, "cluster authorization invalidation publish failed; durable revalidation remains active");
            }
        });
    }

    /// Revalidate all communities with live sockets and cancel inactive ones.
    ///
    /// This is the durable backstop for changes written by another process.
    pub async fn revalidate_live_communities(&self) -> usize {
        let (closed, failures) =
            revalidate_registered_communities(&self.community_connections, |community_id| {
                self.db.is_community_active(community_id)
            })
            .await;
        for (community_id, error) in failures {
            tracing::warn!(%community_id, %error, "community lifecycle revalidation failed; retaining its sockets until next tick");
        }
        closed
    }

    /// Revalidates live bans and private-channel subscriptions from durable state.
    ///
    /// This is the bounded safety backstop when a Chirps invalidation is missed
    /// during a partition or process restart.
    pub async fn revalidate_live_authorization(&self) -> usize {
        let mut revoked = 0;
        for (community, pubkey, owner) in self.conn_manager.authenticated_identities() {
            let (principal, owner_restriction) = tokio::join!(
                self.db.moderation_restriction_state(community, &pubkey),
                async {
                    match owner.as_ref() {
                        Some(owner) => self.db.moderation_restriction_state(community, owner).await,
                        None => Ok(Default::default()),
                    }
                }
            );
            match (principal, owner_restriction) {
                (Ok(principal), Ok(owner)) if !principal.banned && !owner.banned => {}
                _ => {
                    revoked += self.conn_manager.disconnect_pubkey(
                        community,
                        &pubkey,
                        &"0".repeat(64),
                        "blocked: authorization state changed",
                    );
                }
            }
        }
        // ponytail: periodic O(active subscriptions) safety scan; add a durable
        // revision index only when measured fleet size makes this material.
        for (community, channel_id, conn_id) in self.sub_registry.scoped_channel_connections() {
            let Some(pubkey) = self.conn_manager.pubkey_for_conn(conn_id) else {
                continue;
            };
            let allowed = match self.db.get_channel(community, channel_id).await {
                Ok(channel) if channel.visibility != "private" => true,
                Ok(_) => self
                    .db
                    .is_member(community, channel_id, &pubkey)
                    .await
                    .unwrap_or(false),
                Err(_) => false,
            };
            if !allowed {
                revoked += self
                    .evict_live_channel_subscriptions_local(community, channel_id, &pubkey)
                    .await;
            }
        }
        revoked
    }

    /// Removes one principal's local subscriptions for one revoked channel.
    pub(crate) async fn evict_live_channel_subscriptions_local(
        &self,
        community: CommunityId,
        channel_id: Uuid,
        target_pubkey: &[u8],
    ) -> usize {
        let conn_ids = self
            .conn_manager
            .connection_ids_for_pubkey_in_community(community, target_pubkey);
        let mut revoked = 0;
        for conn_id in conn_ids {
            revoked += self
                .evict_conn_channel_subscriptions_local(community, channel_id, conn_id)
                .await;
        }
        revoked
    }

    pub(crate) async fn evict_conn_channel_subscriptions_local(
        &self,
        community: CommunityId,
        channel_id: Uuid,
        conn_id: Uuid,
    ) -> usize {
        let removed = self
            .sub_registry
            .remove_channel_subscriptions_scoped(community, conn_id, channel_id);
        if removed.is_empty() {
            return 0;
        }
        if let Some(subscriptions) = self.conn_manager.subscriptions_for(conn_id) {
            let mut conn_subscriptions = subscriptions.lock().await;
            for update in &removed {
                if update.removed {
                    conn_subscriptions.remove(&update.sub_id);
                }
            }
        }
        let mut revoked = 0;
        for update in removed {
            if update.removed {
                let _ = self.conn_manager.send_to(
                    conn_id,
                    crate::protocol::RelayMessage::closed(
                        &update.sub_id,
                        "restricted: channel access revoked",
                    ),
                );
                revoked += 1;
            }
        }
        revoked
    }

    /// Get accessible channel IDs with a 10-second cache. Falls back to DB on miss.
    pub async fn get_accessible_channel_ids_cached(
        &self,
        community_id: CommunityId,
        pubkey: &[u8],
    ) -> Result<Vec<Uuid>, nimino_db::DbError> {
        let key = (community_id, pubkey.to_vec());
        if let Some(cached) = self.accessible_channels_cache.get(&key) {
            metrics::counter!("nimino_accessible_channels_cache_hits_total").increment(1);
            return Ok(cached);
        }
        metrics::counter!("nimino_accessible_channels_cache_misses_total").increment(1);
        let result = self
            .db
            .get_accessible_channel_ids(community_id, pubkey)
            .await?;
        self.accessible_channels_cache.insert(key, result.clone());
        Ok(result)
    }

    /// Channel visibility string. Caches only `private` (10s); never caches a
    /// non-private value.
    ///
    /// The fan-out access gate fails open on a non-private result, so a stale
    /// cached `open` on another node would mask the filter for the whole TTL
    /// after an open->private flip (no cross-node cache invalidation). Caching
    /// only `private` keeps the cache fail-safe: the worst stale entry is an
    /// over-restrictive `private` (drops non-members on a now-open channel for
    /// <=10s), never a leak.
    ///
    /// `prefetched` lets a caller that already holds the channel row for this
    /// request (ingest's once-per-request fetch, E1 §4.8) reuse it instead of
    /// re-SELECTing. The gate is unchanged: a cached `private` still wins over
    /// the prefetched row (the cache is fail-safe by design), and a `private`
    /// read from the row still populates the cache. With `Some(row)` this
    /// method performs no DB I/O and cannot error.
    pub async fn channel_visibility_cached(
        &self,
        community_id: CommunityId,
        channel_id: Uuid,
        prefetched: Option<&nimino_db::channel::ChannelRecord>,
    ) -> Result<String, nimino_db::DbError> {
        if let Some(cached) = self
            .channel_visibility_cache
            .get(&(community_id, channel_id))
        {
            return Ok(cached);
        }
        let visibility = match prefetched {
            Some(row) => row.visibility.clone(),
            None => {
                self.db
                    .get_channel(community_id, channel_id)
                    .await?
                    .visibility
            }
        };
        if visibility == "private" {
            self.channel_visibility_cache
                .insert((community_id, channel_id), visibility.clone());
        }
        Ok(visibility)
    }
}

/// A channel-visibility read resolved at ingest and threaded through to
/// fan-out within the same request (E1 phase-2, §4.8 phase-2 addendum).
///
/// The community and channel ids the visibility was resolved under travel
/// with the value so it can never be consulted for a different channel or
/// community's fan-out (channel UUIDs collide across communities —
/// `Inv_LabelPropagation`). Consumers must treat a missing/mismatched bundle
/// as "no threaded visibility" and fall back to a fresh fail-closed lookup —
/// never as "assume open".
#[derive(Debug, Clone)]
pub struct ThreadedChannelVisibility {
    /// Community the visibility was resolved under (server-resolved tenant).
    pub community_id: CommunityId,
    /// Channel the visibility was resolved for.
    pub channel_id: Uuid,
    /// The visibility string read at ingest (`"open"` / `"private"` / ...).
    pub visibility: String,
}

/// Handle for graceful audit worker shutdown.
///
/// Signals the worker to stop accepting new entries, drain its buffer,
/// and exit. Independent of `Arc<AppState>` lifetime — works even when
/// background tasks (reaper, local_delivery, health) still hold state clones.
pub struct AuditShutdownHandle {
    cancel: CancellationToken,
    handle: JoinHandle<()>,
}

impl AuditShutdownHandle {
    /// Signal the audit worker to drain and wait up to `timeout` for it to finish.
    pub async fn drain(self, timeout: std::time::Duration) {
        self.cancel.cancel();
        match tokio::time::timeout(timeout, self.handle).await {
            Ok(Ok(())) => tracing::info!("Audit worker drained cleanly"),
            Ok(Err(e)) => tracing::error!("Audit worker panicked: {e}"),
            Err(_) => tracing::error!(
                ?timeout,
                "Audit worker did not drain in time — exiting anyway"
            ),
        }
    }
}

/// Log a single audit entry with metrics. Extracted so the normal loop
/// and the post-cancel drain share the same logic.
async fn log_audit_entry(audit: &nimino_audit::AuditService, entry: nimino_audit::NewAuditEntry) {
    let t = std::time::Instant::now();
    if let Err(e) = audit.log(entry).await {
        metrics::counter!("nimino_audit_log_errors_total").increment(1);
        tracing::error!("Audit log failed: {e}");
    } else {
        metrics::histogram!("nimino_audit_log_seconds").record(t.elapsed().as_secs_f64());
    }
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("relay_url", &self.config.relay_url)
            .field("max_connections", &self.config.max_connections)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::{AuthState, ConnectionState};
    use std::collections::HashMap;
    use tokio::sync::{Mutex, RwLock};

    /// Helper: create a ConnectionManager with one registered connection.
    /// Returns (manager, conn_id, receiver, ctrl_receiver, cancel,
    /// shared_backpressure_count).
    fn setup_conn(
        buffer_size: usize,
    ) -> (
        ConnectionManager,
        Uuid,
        mpsc::Receiver<WsMessage>,
        mpsc::Receiver<WsMessage>,
        CancellationToken,
        Arc<AtomicU8>,
    ) {
        let mgr = ConnectionManager::new();
        let conn_id = Uuid::new_v4();
        let (tx, rx) = mpsc::channel(buffer_size);
        let (ctrl_tx, ctrl_rx) = mpsc::channel(buffer_size);
        let cancel = CancellationToken::new();
        let bp = Arc::new(AtomicU8::new(0));
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            None,
            cancel.clone(),
            nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::clone(&bp),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );
        (mgr, conn_id, rx, ctrl_rx, cancel, bp)
    }

    async fn test_state() -> Arc<AppState> {
        let mut config = crate::config::Config::from_env().expect("default config loads");
        config.require_relay_membership = false;
        let pool = sqlx::PgPool::connect_lazy(&config.database_url).expect("lazy pg pool");
        let db = nimino_db::Db::from_pool(pool.clone());
        let local_delivery = Arc::new(nimino_local_delivery::LocalDelivery::new());
        let audit = nimino_audit::AuditService::new(pool.clone());
        let auth = nimino_auth::AuthService::new(config.auth.clone());
        let search = nimino_search::SearchService::new(pool.clone());
        let workflow_engine = Arc::new(
            nimino_workflow::WorkflowEngine::new_without_policy_for_tests(
                db.clone(),
                nimino_workflow::WorkflowConfig::default(),
            ),
        );
        let media_storage = nimino_media::MediaStorage::new(&config.media).expect("media storage");
        let (state, _audit_shutdown) = AppState::new(
            config,
            db,
            audit,
            local_delivery,
            auth,
            search,
            workflow_engine,
            nostr::Keys::generate(),
            media_storage,
        );
        Arc::new(state)
    }

    #[test]
    fn send_to_resets_grace_counter_on_success() {
        let (mgr, id, _rx, _ctrl_rx, _cancel, bp) = setup_conn(16);
        // Simulate prior backpressure.
        bp.store(2, Ordering::Relaxed);
        assert!(mgr.send_to(id, "hello".into()));
        assert_eq!(
            bp.load(Ordering::Relaxed),
            0,
            "successful send should reset counter"
        );
    }

    #[test]
    fn send_to_increments_grace_counter_on_full() {
        // Buffer size 1 — fill it, then the next send is Full.
        let (mgr, id, _rx, _ctrl_rx, cancel, bp) = setup_conn(1);
        assert!(mgr.send_to(id, "fill".into()));
        // Buffer is now full.
        assert!(!mgr.send_to(id, "overflow-1".into()));
        assert_eq!(bp.load(Ordering::Relaxed), 1, "first overflow → count=1");
        assert!(
            !cancel.is_cancelled(),
            "should not cancel on first overflow"
        );

        assert!(!mgr.send_to(id, "overflow-2".into()));
        assert_eq!(bp.load(Ordering::Relaxed), 2);
        assert!(
            !cancel.is_cancelled(),
            "should not cancel on second overflow"
        );
    }

    #[test]
    fn send_to_cancels_after_grace_limit() {
        let (mgr, id, _rx, _ctrl_rx, cancel, _bp) = setup_conn(1);
        assert!(mgr.send_to(id, "fill".into()));
        // Exhaust grace: 3 consecutive Full events (matches grace_limit=3 from setup_conn).
        for _ in 0..3u8 {
            mgr.send_to(id, "overflow".into());
        }
        assert!(
            cancel.is_cancelled(),
            "should cancel after grace_limit overflows"
        );
    }

    #[test]
    fn shared_counter_between_direct_and_fanout() {
        // Verify that ConnectionState::send() and ConnectionManager::send_to()
        // share the same backpressure counter via Arc<AtomicU8>.
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(1);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        let bp = Arc::new(AtomicU8::new(0));

        let conn = ConnectionState {
            conn_id,
            tenant: nimino_core::tenant::TenantContext::resolved(
                nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                "test.local".to_string(),
            ),
            remote_addr: "127.0.0.1:1234".parse().unwrap(),
            auth_state: RwLock::new(AuthState::Failed),
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
            send_tx: tx.clone(),
            ctrl_tx,
            cancel: cancel.clone(),
            backpressure_count: Arc::clone(&bp),
            grace_limit: 3,
        };

        let mgr = ConnectionManager::new();
        mgr.register(
            conn_id,
            tx,
            conn.ctrl_tx.clone(),
            None,
            cancel.clone(),
            nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::clone(&bp),
            Arc::clone(&conn.subscriptions),
            3,
        );

        // Fill the buffer via direct send.
        assert!(conn.send("fill".into()));
        // Overflow via fan-out.
        assert!(!mgr.send_to(conn_id, "overflow-fanout".into()));
        assert_eq!(
            bp.load(Ordering::Relaxed),
            1,
            "fan-out overflow increments shared counter"
        );
        // Overflow via direct send.
        assert!(!conn.send("overflow-direct".into()));
        assert_eq!(
            bp.load(Ordering::Relaxed),
            2,
            "direct overflow increments same counter"
        );
        // One more fan-out overflow → should cancel (3 consecutive).
        mgr.send_to(conn_id, "overflow-final".into());
        assert!(
            cancel.is_cancelled(),
            "shared counter reached limit via mixed path"
        );
    }

    #[tokio::test]
    async fn tracks_connections_by_authenticated_pubkey_within_community() {
        let mgr = ConnectionManager::new();
        let community_a = nimino_core::tenant::CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
        let community_b = nimino_core::tenant::CommunityId::from_uuid(Uuid::from_u128(0xBBBB));
        let conn_a = Uuid::new_v4();
        let conn_b = Uuid::new_v4();
        let (tx_a, _rx_a) = mpsc::channel(1);
        let (ctrl_tx_a, _ctrl_rx_a) = mpsc::channel(1);
        let (tx_b, _rx_b) = mpsc::channel(1);
        let (ctrl_tx_b, _ctrl_rx_b) = mpsc::channel(1);
        mgr.register(
            conn_a,
            tx_a,
            ctrl_tx_a,
            None,
            CancellationToken::new(),
            community_a,
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );
        mgr.register(
            conn_b,
            tx_b,
            ctrl_tx_b,
            None,
            CancellationToken::new(),
            community_b,
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );

        let pubkey = vec![7u8; 32];
        mgr.set_authenticated_pubkey(conn_a, pubkey.clone());
        mgr.set_authenticated_pubkey(conn_b, pubkey.clone());

        assert_eq!(
            mgr.connection_ids_for_pubkey_in_community(community_a, &pubkey),
            vec![conn_a]
        );
        assert_eq!(
            mgr.connection_ids_for_pubkey_in_community(community_b, &pubkey),
            vec![conn_b]
        );
        assert!(mgr.subscriptions_for(conn_a).is_some());
        assert!(mgr.subscriptions_for(conn_b).is_some());
    }

    #[tokio::test]
    async fn pubkey_for_conn_returns_authenticated_pubkey() {
        let mgr = ConnectionManager::new();
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(1);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        let bp = Arc::new(AtomicU8::new(0));
        let subscriptions = Arc::new(Mutex::new(HashMap::new()));
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            None,
            cancel,
            nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            bp,
            subscriptions,
            3,
        );

        assert_eq!(mgr.pubkey_for_conn(conn_id), None);
        let pubkey = vec![9u8; 32];
        mgr.set_authenticated_pubkey(conn_id, pubkey.clone());
        assert_eq!(mgr.pubkey_for_conn(conn_id), Some(pubkey));
        assert_eq!(mgr.pubkey_for_conn(Uuid::new_v4()), None);
    }

    #[tokio::test]
    async fn accessible_channel_invalidation_is_scoped_to_community() {
        let state = test_state().await;
        let community_a = CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
        let community_b = CommunityId::from_uuid(Uuid::from_u128(0xBBBB));
        let pubkey = vec![7u8; 32];
        let channels_a = vec![Uuid::from_u128(1)];
        let channels_b = vec![Uuid::from_u128(2)];

        state
            .accessible_channels_cache
            .insert((community_a, pubkey.clone()), channels_a);
        state
            .accessible_channels_cache
            .insert((community_b, pubkey.clone()), channels_b.clone());

        state.invalidate_all_accessible_channels_local(community_a);

        assert_eq!(
            state
                .accessible_channels_cache
                .get(&(community_a, pubkey.clone())),
            None
        );
        assert_eq!(
            state
                .accessible_channels_cache
                .get(&(community_b, pubkey.clone())),
            Some(channels_b),
            "A's cache drop must not evict B's accessible-channel entry"
        );
    }

    #[tokio::test]
    async fn channel_deleted_invalidation_is_scoped_to_community() {
        let state = test_state().await;
        let community_a = CommunityId::from_uuid(Uuid::from_u128(0xAAAA));
        let community_b = CommunityId::from_uuid(Uuid::from_u128(0xBBBB));
        let channel_id = Uuid::from_u128(1);
        let pubkey = vec![7u8; 32];

        for community in [community_a, community_b] {
            state
                .membership_cache
                .insert((community, channel_id, pubkey.clone()), true);
            state
                .accessible_channels_cache
                .insert((community, pubkey.clone()), vec![channel_id]);
            state
                .channel_visibility_cache
                .insert((community, channel_id), "private".to_string());
        }

        state.invalidate_channel_deleted_local(community_a);

        assert_eq!(
            state
                .membership_cache
                .get(&(community_a, channel_id, pubkey.clone())),
            None
        );
        assert_eq!(
            state
                .accessible_channels_cache
                .get(&(community_a, pubkey.clone())),
            None
        );
        assert_eq!(
            state
                .channel_visibility_cache
                .get(&(community_a, channel_id)),
            None
        );
        assert_eq!(
            state
                .membership_cache
                .get(&(community_b, channel_id, pubkey.clone())),
            Some(true)
        );
        assert_eq!(
            state
                .accessible_channels_cache
                .get(&(community_b, pubkey.clone())),
            Some(vec![channel_id])
        );
        assert_eq!(
            state
                .channel_visibility_cache
                .get(&(community_b, channel_id)),
            Some("private".to_string()),
            "A's channel deletion must not evict B's cache entries"
        );
    }

    #[test]
    fn community_lifecycle_disconnect_covers_socket_types_and_preserves_tenant_fence() {
        let registry = CommunityConnectionRegistry::new();
        let community_a = CommunityId::from_uuid(Uuid::from_u128(0xa));
        let community_b = CommunityId::from_uuid(Uuid::from_u128(0xb));
        let ordinary_a = CancellationToken::new();
        let audio_a = CancellationToken::new();
        let ordinary_b = CancellationToken::new();
        let ordinary_a_control = CommunityConnectionControl::new(ordinary_a.clone());
        let audio_a_control = CommunityConnectionControl::new(audio_a.clone());
        let ordinary_b_control = CommunityConnectionControl::new(ordinary_b.clone());
        let ordinary_a_reason = ordinary_a_control.disconnect_reason();
        let audio_a_reason = audio_a_control.disconnect_reason();
        let ordinary_b_reason = ordinary_b_control.disconnect_reason();
        let _ordinary_a_guard = registry.register(Uuid::new_v4(), community_a, ordinary_a_control);
        let _audio_a_guard = registry.register(Uuid::new_v4(), community_a, audio_a_control);
        let _ordinary_b_guard = registry.register(Uuid::new_v4(), community_b, ordinary_b_control);

        assert_eq!(registry.disconnect_community(community_a), 2);
        assert!(ordinary_a.is_cancelled());
        assert!(audio_a.is_cancelled());
        assert!(!ordinary_b.is_cancelled());
        assert_eq!(
            *ordinary_a_reason.borrow(),
            Some(CommunityDisconnectReason::CommunityDeleted)
        );
        assert_eq!(
            *audio_a_reason.borrow(),
            Some(CommunityDisconnectReason::CommunityDeleted)
        );
        assert_eq!(*ordinary_b_reason.borrow(), None);
    }

    #[tokio::test]
    async fn register_then_revalidate_closes_both_archive_race_orderings() {
        let registry = CommunityConnectionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xa));

        // Archive wins before durable revalidation: the check observes inactive
        // and the socket body never starts.
        let cancel_before = CancellationToken::new();
        let started_before = Arc::new(AtomicBool::new(false));
        let started_before_run = Arc::clone(&started_before);
        run_registered_community_connection(
            &registry,
            Uuid::new_v4(),
            community,
            CommunityConnectionControl::new(cancel_before.clone()),
            || async { Ok(false) },
            move |_| async move { started_before_run.store(true, Ordering::SeqCst) },
        )
        .await;
        assert!(cancel_before.is_cancelled());
        assert!(!started_before.load(Ordering::SeqCst));

        // Archive wins after registration but while revalidation is paused: its
        // sweep sees the token, and even an active query result cannot start the
        // socket body afterward.
        let cancel_during = CancellationToken::new();
        let registered = Arc::new(tokio::sync::Notify::new());
        let resume = Arc::new(tokio::sync::Notify::new());
        let registered_check = Arc::clone(&registered);
        let resume_check = Arc::clone(&resume);
        let started_during = Arc::new(AtomicBool::new(false));
        let started_during_run = Arc::clone(&started_during);
        let future = run_registered_community_connection(
            &registry,
            Uuid::new_v4(),
            community,
            CommunityConnectionControl::new(cancel_during.clone()),
            move || async move {
                registered_check.notify_one();
                resume_check.notified().await;
                Ok(true)
            },
            move |_| async move { started_during_run.store(true, Ordering::SeqCst) },
        );
        tokio::pin!(future);
        tokio::select! {
            _ = registered.notified() => {}
            _ = &mut future => panic!("revalidation should be paused"),
        }
        assert_eq!(registry.disconnect_community(community), 1);
        resume.notify_one();
        future.await;
        assert!(cancel_during.is_cancelled());
        assert!(!started_during.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn revalidation_continues_after_one_community_lookup_failure() {
        let registry = CommunityConnectionRegistry::new();
        let archived_a = CommunityId::from_uuid(Uuid::from_u128(0xa));
        let failed = CommunityId::from_uuid(Uuid::from_u128(0xb));
        let archived_c = CommunityId::from_uuid(Uuid::from_u128(0xc));
        let cancel_a = CancellationToken::new();
        let cancel_failed = CancellationToken::new();
        let cancel_c = CancellationToken::new();
        let _guard_a = registry.register(
            Uuid::new_v4(),
            archived_a,
            CommunityConnectionControl::new(cancel_a.clone()),
        );
        let _guard_failed = registry.register(
            Uuid::new_v4(),
            failed,
            CommunityConnectionControl::new(cancel_failed.clone()),
        );
        let _guard_c = registry.register(
            Uuid::new_v4(),
            archived_c,
            CommunityConnectionControl::new(cancel_c.clone()),
        );

        let (closed, failures) =
            revalidate_registered_communities(&registry, |community| async move {
                if community == failed {
                    Err(nimino_db::DbError::InvalidData(
                        "injected lookup failure".into(),
                    ))
                } else {
                    Ok(false)
                }
            })
            .await;

        assert_eq!(closed, 2);
        assert!(cancel_a.is_cancelled());
        assert!(!cancel_failed.is_cancelled());
        assert!(cancel_c.is_cancelled());
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, failed);
        assert_eq!(
            registry.bound_communities(),
            HashSet::from([archived_a, failed, archived_c])
        );
    }

    #[test]
    fn community_lifecycle_guard_deregisters_on_early_return() {
        let registry = CommunityConnectionRegistry::new();
        let community = CommunityId::from_uuid(Uuid::from_u128(0xa));
        let cancel = CancellationToken::new();
        let guard = registry.register(
            Uuid::new_v4(),
            community,
            CommunityConnectionControl::new(cancel.clone()),
        );
        assert_eq!(registry.bound_communities(), HashSet::from([community]));

        drop(guard);

        assert!(registry.bound_communities().is_empty());
        assert_eq!(registry.disconnect_community(community), 0);
        assert!(!cancel.is_cancelled());
    }

    #[tokio::test]
    async fn disconnect_pubkey_closes_matching_conns_with_reason() {
        let (mgr, id, _rx, mut ctrl_rx, cancel, _bp) = setup_conn(8);
        let pubkey = vec![3u8; 32];
        mgr.set_authenticated_pubkey(id, pubkey.clone());

        // setup_conn registers the connection under the nil community.
        let community = nimino_core::tenant::CommunityId::from_uuid(Uuid::nil());
        let closed = mgr.disconnect_pubkey(
            community,
            &pubkey,
            "0".repeat(64).as_str(),
            "blocked: banned",
        );

        assert_eq!(closed, 1, "the one matching connection is closed");
        assert!(
            cancel.is_cancelled(),
            "connection is cancelled (socket close)"
        );
        // The reason frame is queued on the control channel ahead of the close.
        let frame = ctrl_rx.try_recv().expect("reason frame delivered");
        match frame {
            WsMessage::Text(t) => {
                assert!(t.as_str().contains("blocked: banned"), "carries the reason");
                assert!(t.as_str().contains("false"), "is an OK false frame");
            }
            other => panic!("expected text frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn disconnect_pubkey_ignores_non_matching_conns() {
        let (mgr, id, _rx, _ctrl_rx, cancel, _bp) = setup_conn(8);
        mgr.set_authenticated_pubkey(id, vec![1u8; 32]);

        let community = nimino_core::tenant::CommunityId::from_uuid(Uuid::nil());
        let closed = mgr.disconnect_pubkey(
            community,
            &[2u8; 32],
            "0".repeat(64).as_str(),
            "blocked: banned",
        );

        assert_eq!(closed, 0, "no connection matches a different pubkey");
        assert!(!cancel.is_cancelled(), "unrelated connection stays live");
    }

    #[tokio::test]
    async fn disconnect_pubkey_cascades_owner_ban_to_nip_oa_agent() {
        let mgr = ConnectionManager::new();
        let community = CommunityId::from_uuid(Uuid::nil());
        let agent = vec![1u8; 32];
        let owner = vec![2u8; 32];
        let unrelated = vec![3u8; 32];
        let mut cancels = Vec::new();

        for (principal, attested_owner) in [
            (agent, Some(owner.clone())),
            (owner.clone(), None),
            (unrelated, None),
        ] {
            let conn_id = Uuid::new_v4();
            let (tx, _rx) = mpsc::channel(8);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
            let cancel = CancellationToken::new();
            mgr.register(
                conn_id,
                tx,
                ctrl_tx,
                None,
                cancel.clone(),
                community,
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
            );
            mgr.set_authenticated_identity(conn_id, principal, attested_owner);
            cancels.push(cancel);
        }

        assert_eq!(
            mgr.disconnect_pubkey(community, &owner, &"0".repeat(64), "blocked: owner banned",),
            2
        );
        assert!(cancels[0].is_cancelled(), "the owner's agent is closed");
        assert!(cancels[1].is_cancelled(), "the owner session is closed");
        assert!(!cancels[2].is_cancelled(), "unrelated identity stays live");
    }

    #[tokio::test]
    async fn disconnect_pubkey_is_fenced_to_the_banning_community() {
        // Same pubkey, two live sockets in two different communities on one pod.
        // A ban in community A must close only A's socket, never B's — the
        // tenant fence on live-disconnect fan-out (B1).
        let mgr = ConnectionManager::new();
        let pubkey = vec![7u8; 32];

        let community_a = nimino_core::tenant::CommunityId::from_uuid(Uuid::from_u128(0xa));
        let community_b = nimino_core::tenant::CommunityId::from_uuid(Uuid::from_u128(0xb));

        let register = |community| {
            let conn_id = Uuid::new_v4();
            let (tx, _rx) = mpsc::channel(8);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
            let cancel = CancellationToken::new();
            mgr.register(
                conn_id,
                tx,
                ctrl_tx,
                None,
                cancel.clone(),
                community,
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
            );
            mgr.set_authenticated_pubkey(conn_id, pubkey.clone());
            cancel
        };

        let cancel_a = register(community_a);
        let cancel_b = register(community_b);

        let closed = mgr.disconnect_pubkey(
            community_a,
            &pubkey,
            "0".repeat(64).as_str(),
            "blocked: banned",
        );

        assert_eq!(closed, 1, "only the community-A socket is closed");
        assert!(cancel_a.is_cancelled(), "community-A session is closed");
        assert!(
            !cancel_b.is_cancelled(),
            "community-B session stays live — ban does not cross the tenant fence"
        );
    }

    #[tokio::test]
    async fn drain_all_jittered_waits_for_writer_acknowledgement_without_cancelling() {
        let mgr = Arc::new(ConnectionManager::new());
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
        let (restart_tx, mut restart_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            Some(restart_tx),
            cancel.clone(),
            nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );

        let drain_mgr = Arc::clone(&mgr);
        let drain = tokio::spawn(async move { drain_mgr.drain_all_jittered(1).await });
        let restart = restart_rx.recv().await.expect("restart command delivered");
        assert!(!drain.is_finished(), "drain waits for the writer flush");
        restart.flushed.send(true).expect("acknowledge flush");

        assert_eq!(drain.await.expect("drain task"), 1);
        assert!(
            !cancel.is_cancelled(),
            "successful flush does not use cancellation fallback"
        );
    }

    #[tokio::test]
    async fn drain_all_jittered_cancels_when_restart_channel_is_full_or_closed() {
        for keep_receiver in [true, false] {
            let mgr = ConnectionManager::new();
            let conn_id = Uuid::new_v4();
            let (tx, _rx) = mpsc::channel(8);
            let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
            let (restart_tx, restart_rx) = mpsc::channel(1);
            let (pending_tx, _pending_rx) = tokio::sync::oneshot::channel();
            if keep_receiver {
                restart_tx
                    .try_send(RestartClose {
                        flushed: pending_tx,
                    })
                    .expect("fill restart channel");
            } else {
                drop(restart_rx);
            }
            let cancel = CancellationToken::new();
            mgr.register(
                conn_id,
                tx,
                ctrl_tx,
                Some(restart_tx),
                cancel.clone(),
                nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
            );

            assert_eq!(mgr.drain_all_jittered(1).await, 1);
            assert!(
                cancel.is_cancelled(),
                "unavailable writer cancels as fallback"
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn drain_all_jittered_cancels_when_flush_ack_times_out() {
        // A writer that accepts the restart command but never acknowledges the
        // flush (e.g. wedged mid-send) must not stall the drain: after
        // RESTART_CLOSE_ACK_TIMEOUT the connection falls back to cancellation.
        let mgr = Arc::new(ConnectionManager::new());
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, _ctrl_rx) = mpsc::channel(8);
        let (restart_tx, mut restart_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            Some(restart_tx),
            cancel.clone(),
            nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );

        let drain_mgr = Arc::clone(&mgr);
        let drain = tokio::spawn(async move { drain_mgr.drain_all_jittered(1).await });
        // Take the restart command but hold the ack sender forever.
        let restart = restart_rx.recv().await.expect("restart command delivered");
        assert!(!drain.is_finished(), "drain waits on the ack timeout");
        // Advance past the 5s ack timeout under paused time.
        tokio::time::sleep(RESTART_CLOSE_ACK_TIMEOUT + std::time::Duration::from_millis(1)).await;

        assert_eq!(drain.await.expect("drain task"), 1);
        assert!(
            cancel.is_cancelled(),
            "an un-acknowledged flush falls back to cancellation"
        );
        drop(restart);
    }

    #[tokio::test]
    async fn drain_all_sends_restart_close_and_cancels_every_conn() {
        // Graceful shutdown must tell every live client to reconnect — across
        // all communities — with a 1012 restart close frame queued ahead of
        // the cancel-driven socket close.
        let mgr = ConnectionManager::new();

        let register = |community| {
            let conn_id = Uuid::new_v4();
            let (tx, _rx) = mpsc::channel(8);
            let (ctrl_tx, ctrl_rx) = mpsc::channel(8);
            let cancel = CancellationToken::new();
            mgr.register(
                conn_id,
                tx,
                ctrl_tx,
                None,
                cancel.clone(),
                community,
                Arc::new(AtomicU8::new(0)),
                Arc::new(Mutex::new(HashMap::new())),
                3,
            );
            (ctrl_rx, cancel)
        };

        let (mut ctrl_a, cancel_a) = register(nimino_core::tenant::CommunityId::from_uuid(
            Uuid::from_u128(0xa),
        ));
        let (mut ctrl_b, cancel_b) = register(nimino_core::tenant::CommunityId::from_uuid(
            Uuid::from_u128(0xb),
        ));

        let closed = mgr.drain_all();

        assert_eq!(closed, 2, "every connection is signalled, no tenant fence");
        assert!(cancel_a.is_cancelled(), "community-A session is cancelled");
        assert!(cancel_b.is_cancelled(), "community-B session is cancelled");

        for ctrl_rx in [&mut ctrl_a, &mut ctrl_b] {
            let frame = ctrl_rx.try_recv().expect("close frame delivered");
            match frame {
                WsMessage::Close(Some(close)) => {
                    assert_eq!(
                        close.code,
                        axum::extract::ws::close_code::RESTART,
                        "close code is 1012 Service Restart"
                    );
                    assert_eq!(close.reason.as_str(), "relay restarting");
                }
                other => panic!("expected a restart close frame, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn drain_all_full_control_buffer_still_cancels() {
        // Best-effort delivery: a wedged control channel must not block the
        // drain — the cancel still closes the socket, just without the frame.
        let mgr = ConnectionManager::new();
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel(1);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx.clone(),
            None,
            cancel.clone(),
            nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );
        // Wedge the 1-slot control channel.
        ctrl_tx
            .try_send(WsMessage::Text("wedge".into()))
            .expect("fill control channel");

        let closed = mgr.drain_all();

        assert_eq!(closed, 1);
        assert!(
            cancel.is_cancelled(),
            "cancel fires even when the close frame cannot be queued"
        );
        // Only the wedge frame is present — the close was dropped, not queued.
        assert!(matches!(
            ctrl_rx.try_recv().expect("wedge frame"),
            WsMessage::Text(_)
        ));
        assert!(ctrl_rx.try_recv().is_err(), "no second frame queued");
    }

    #[tokio::test]
    async fn register_after_drain_self_signals_restart_close_and_cancel() {
        // The shutdown-boundary race: an upgrade accepted before SIGTERM can
        // finish its async admission check and register AFTER drain_all's
        // one-shot snapshot. The sticky drain flag makes that interleaving
        // deterministic — register itself queues the 1012 and cancels, so no
        // late registration can ride out graceful shutdown unclosed.
        let mgr = ConnectionManager::new();

        // Drain with zero connections — sets the sticky flag.
        assert_eq!(mgr.drain_all(), 0);

        // Late registration lands after the snapshot.
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            None,
            cancel.clone(),
            nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );

        assert!(
            cancel.is_cancelled(),
            "late registration is cancelled by the sticky drain flag"
        );
        match ctrl_rx.try_recv().expect("close frame delivered") {
            WsMessage::Close(Some(close)) => {
                assert_eq!(
                    close.code,
                    axum::extract::ws::close_code::RESTART,
                    "late registration still gets the 1012 restart close"
                );
                assert_eq!(close.reason.as_str(), "relay restarting");
            }
            other => panic!("expected a restart close frame, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn drain_all_is_immediate() {
        // The default (jitter-off) drain queues the frame and cancels
        // synchronously — the frame is present the moment drain_all() returns.
        let mgr = Arc::new(ConnectionManager::new());
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            None,
            cancel.clone(),
            nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );

        let closed = mgr.drain_all();

        assert_eq!(closed, 1);
        assert!(cancel.is_cancelled(), "default drain cancels synchronously");
        assert!(
            matches!(
                ctrl_rx
                    .try_recv()
                    .expect("close frame delivered synchronously"),
                WsMessage::Close(Some(_))
            ),
            "the restart close is queued before drain_all() returns"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn drain_all_jittered_defers_close_until_within_jitter_window() {
        // With jitter, the close is deferred within the owned drain future.
        // The sticky drain flag is still set immediately, so a late
        // registration self-signals with no delay.
        let mgr = Arc::new(ConnectionManager::new());
        let conn_id = Uuid::new_v4();
        let (tx, _rx) = mpsc::channel(8);
        let (ctrl_tx, mut ctrl_rx) = mpsc::channel(8);
        let cancel = CancellationToken::new();
        mgr.register(
            conn_id,
            tx,
            ctrl_tx,
            None,
            cancel.clone(),
            nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );

        let jitter_ms = 20_000u64;
        // Poll the owned drain through its first await. Dropping this future
        // would drop the timers too; the shutdown path must retain and await it.
        let drain = mgr.drain_all_jittered(jitter_ms);
        tokio::pin!(drain);
        assert!(
            futures_util::poll!(&mut drain).is_pending(),
            "jittered drain remains pending while its timers are owned"
        );

        // Not closed yet — the delayed drain is parked on its timer.
        assert!(
            !cancel.is_cancelled(),
            "jittered close is deferred, not synchronous"
        );
        assert!(
            ctrl_rx.try_recv().is_err(),
            "no close frame queued before the delay elapses"
        );

        // A registration racing past the snapshot still self-signals at once,
        // regardless of jitter — clients arriving mid-shutdown are closed now.
        let late_id = Uuid::new_v4();
        let (late_tx, _late_rx) = mpsc::channel(8);
        let (late_ctrl_tx, mut late_ctrl_rx) = mpsc::channel(8);
        let late_cancel = CancellationToken::new();
        mgr.register(
            late_id,
            late_tx,
            late_ctrl_tx,
            None,
            late_cancel.clone(),
            nimino_core::tenant::CommunityId::from_uuid(Uuid::nil()),
            Arc::new(AtomicU8::new(0)),
            Arc::new(Mutex::new(HashMap::new())),
            3,
        );
        assert!(
            late_cancel.is_cancelled(),
            "late registration self-signals immediately, unaffected by jitter"
        );
        assert!(
            matches!(
                late_ctrl_rx.try_recv().expect("late close frame"),
                WsMessage::Close(Some(_))
            ),
            "late registration gets the restart close with no delay"
        );

        // Advance past the whole jitter window; awaiting the owned drain must
        // complete only after the deferred close has fired.
        tokio::time::advance(std::time::Duration::from_millis(jitter_ms + 1)).await;
        assert_eq!(drain.await, 1, "one captured connection drained");

        assert!(
            cancel.is_cancelled(),
            "the jittered connection is closed within the jitter window"
        );
        match ctrl_rx.try_recv().expect("deferred close frame delivered") {
            WsMessage::Close(Some(close)) => {
                assert_eq!(
                    close.code,
                    axum::extract::ws::close_code::RESTART,
                    "jittered close is still 1012 Service Restart"
                );
                assert_eq!(close.reason.as_str(), "relay restarting");
            }
            other => panic!("expected a restart close frame, got {other:?}"),
        }
    }
}
