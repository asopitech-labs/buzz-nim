//! Automatic bounded anti-entropy orchestration over Chirps.
//!
//! Nim owns every synchronization decision through `nimino-boundary`. This
//! crate only authenticates transport scope, computes digests, encodes frames,
//! and executes exact-checkpoint store effects.

#![deny(missing_docs)]

use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use nimino_boundary::{
    BoundaryClient, BoundaryError, BoundaryRequest, BoundaryResult, CallContext, DigestFrame,
    InventoryFact, InventoryMergeEffect, InventoryMergeError, InventoryMergePair, RangeBatchFrame,
    RangeBatchPlan, RangeRequestFrame, SyncCancelFrame, SyncDecision, SyncEffect, SyncEnvelope,
    SyncPhase, SyncPolicyError, SyncPolicyRequest, SyncPolicyResult, SyncRecord, SyncState,
};
use nimino_chirps::{MeshClient, MeshRuntimeError, NodeId};
use nimino_store::{
    canonical_logical_record_digest, canonical_prefix_digest_at, canonical_record_digest,
    canonical_state_digest, extend_prefix_digest, CanonicalCommit, CanonicalStateDigest, LogAppend,
    NodeStorePort, RecordClass, RecordWrite, StoreError, StoredRecord, MAX_PAGE_SIZE,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::{
    sync::{watch, Mutex},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

const SYNC_PROTOCOL: &str = "nimino.sync";
const SYNC_VERSION: u16 = 2;
const WIRE_PREFIX: &[u8] = b"NIMINO-SYNC/2\n";
const MAX_SYNC_RECORDS: u16 = 1_000;
const MAX_SYNC_BYTES: u32 = 1_048_576;

/// Bounded timings and range limits for automatic synchronization.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncRuntimeOptions {
    advertise_interval: Duration,
    session_timeout: Duration,
    policy_timeout: Duration,
    max_records: u16,
    max_encoded_bytes: u32,
}

impl SyncRuntimeOptions {
    /// Creates explicit runtime bounds validated by [`SyncRuntime::start`].
    pub fn new(
        advertise_interval: Duration,
        session_timeout: Duration,
        policy_timeout: Duration,
        max_records: u16,
        max_encoded_bytes: u32,
    ) -> Self {
        Self {
            advertise_interval,
            session_timeout,
            policy_timeout,
            max_records,
            max_encoded_bytes,
        }
    }

    fn validate(self) -> Result<(), SyncRuntimeError> {
        if self.advertise_interval.is_zero()
            || self.session_timeout.is_zero()
            || self.policy_timeout.is_zero()
            || self.max_records == 0
            || self.max_records > MAX_SYNC_RECORDS
            || self.max_encoded_bytes == 0
            || self.max_encoded_bytes > MAX_SYNC_BYTES
            || self.session_timeout.as_millis() > i64::MAX as u128
        {
            return Err(SyncRuntimeError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl Default for SyncRuntimeOptions {
    fn default() -> Self {
        Self::new(
            Duration::from_secs(1),
            Duration::from_secs(10),
            Duration::from_secs(2),
            MAX_SYNC_RECORDS,
            MAX_SYNC_BYTES,
        )
    }
}

/// Typed automatic-sync runtime failure.
#[derive(Debug, Error)]
pub enum SyncRuntimeError {
    /// One or more configured limits are empty or exceed the protocol ceiling.
    #[error("invalid sync runtime configuration")]
    InvalidConfiguration,
    /// A community identifier is empty or contains NUL.
    #[error("invalid sync community identifier")]
    InvalidCommunity,
    /// Chirps rejected or stopped an opaque transport operation.
    #[error("Chirps transport failed: {0}")]
    Transport(#[from] MeshRuntimeError),
    /// The supervised Nim policy worker failed.
    #[error("Nim sync policy failed: {0}")]
    Boundary(#[from] BoundaryError),
    /// The local canonical store failed.
    #[error("sync store effect failed: {0}")]
    Store(#[from] StoreError),
    /// An opaque transport payload violated the typed frame contract.
    #[error("invalid sync frame: {0}")]
    InvalidFrame(String),
    /// Nim returned a response for a different operation variant.
    #[error("Nim returned an unexpected sync policy result")]
    UnexpectedPolicyResult,
    /// Nim rejected authenticated synchronization facts.
    #[error("Nim rejected sync facts: {0:?}")]
    PolicyRejected(SyncPolicyError),
    /// Nim rejected or quarantined a logical-state merge fact.
    #[error("Nim rejected convergence facts: {0:?}")]
    ConvergenceRejected(InventoryMergeError),
    /// No canonical record fits within the negotiated byte bound.
    #[error("canonical range cannot fit the negotiated byte bound")]
    RangeTooLarge,
    /// The background task panicked or was aborted.
    #[error("sync task failed")]
    TaskFailed,
}

/// Cloneable control and health facade for automatic synchronization.
#[derive(Clone)]
pub struct SyncClient {
    communities: watch::Sender<Arc<HashSet<String>>>,
    last_error: watch::Receiver<Option<String>>,
    running: watch::Receiver<bool>,
    counters: Arc<SyncCounters>,
}

impl SyncClient {
    /// Atomically replaces the communities eligible for replication.
    pub fn replace_communities(
        &self,
        communities: impl IntoIterator<Item = String>,
    ) -> Result<(), SyncRuntimeError> {
        let communities = validated_communities(communities)?;
        self.communities.send_replace(Arc::new(communities));
        Ok(())
    }

    /// Returns the most recent non-fatal synchronization failure.
    pub fn last_error(&self) -> Option<String> {
        self.last_error.borrow().clone()
    }

    /// Returns whether the synchronization task is still accepting work.
    pub fn is_running(&self) -> bool {
        *self.running.borrow()
    }

    /// Returns monotonic transport and store-effect counters.
    pub fn stats(&self) -> SyncStats {
        SyncStats {
            sent_frames: self.counters.sent.load(Ordering::Relaxed),
            received_frames: self.counters.received.load(Ordering::Relaxed),
            applied_batches: self.counters.applied.load(Ordering::Relaxed),
        }
    }
}

/// Monotonic automatic-sync health counters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SyncStats {
    /// Successfully handed to Chirps.
    pub sent_frames: u64,
    /// Authenticated frames received from Chirps.
    pub received_frames: u64,
    /// Exact-checkpoint batches committed locally.
    pub applied_batches: u64,
}

#[derive(Default)]
struct SyncCounters {
    sent: AtomicU64,
    received: AtomicU64,
    applied: AtomicU64,
}

/// Lifecycle owner for one node's automatic synchronization task.
pub struct SyncRuntime {
    client: SyncClient,
    shutdown: CancellationToken,
    task: Option<JoinHandle<Result<(), SyncRuntimeError>>>,
}

impl SyncRuntime {
    /// Starts automatic digest advertisement and bounded range handling.
    pub fn start(
        mesh: MeshClient,
        boundary: BoundaryClient,
        store: Arc<dyn NodeStorePort>,
        communities: impl IntoIterator<Item = String>,
        options: SyncRuntimeOptions,
    ) -> Result<Self, SyncRuntimeError> {
        options.validate()?;
        let communities = Arc::new(validated_communities(communities)?);
        let (community_sender, community_receiver) = watch::channel(communities);
        let (error_sender, last_error) = watch::channel(None);
        let (running_sender, running) = watch::channel(true);
        let counters = Arc::new(SyncCounters::default());
        let shutdown = CancellationToken::new();
        let context = RuntimeContext {
            local_node: node_name(mesh.local_node_id()),
            mesh,
            boundary,
            store,
            communities: community_receiver,
            options,
            started: Instant::now(),
            sessions: Mutex::new(HashMap::new()),
            snapshot_cursors: Mutex::new(HashMap::new()),
            advertised: Mutex::new(HashMap::new()),
            errors: error_sender,
            counters: counters.clone(),
        };
        let task_shutdown = shutdown.clone();
        let task_errors = context.errors.clone();
        let task = tokio::spawn(async move {
            let result = context.run(task_shutdown).await;
            if let Err(error) = &result {
                task_errors.send_replace(Some(error.to_string()));
            }
            running_sender.send_replace(false);
            result
        });
        Ok(Self {
            client: SyncClient {
                communities: community_sender,
                last_error,
                running,
                counters,
            },
            shutdown,
            task: Some(task),
        })
    }

    /// Returns a cloneable configuration and health facade.
    pub fn client(&self) -> SyncClient {
        self.client.clone()
    }

    /// Cancels the task and waits for it to release its Chirps subscription.
    pub async fn stop(mut self) -> Result<(), SyncRuntimeError> {
        self.shutdown.cancel();
        if let Some(task) = self.task.take() {
            task.await.map_err(|_| SyncRuntimeError::TaskFailed)??;
        }
        Ok(())
    }
}

impl Drop for SyncRuntime {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

#[derive(Clone)]
struct AdvertisedRange {
    community_id: String,
    target_node: String,
    state: CanonicalStateDigest,
    next_advertise: Instant,
    expires_at: Instant,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InventoryCursor {
    record_type: String,
    key: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SnapshotRequestFrame {
    envelope: SyncEnvelope,
    after: Option<InventoryCursor>,
    limit_records: u16,
    max_encoded_bytes: u32,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SnapshotBatchFrame {
    envelope: SyncEnvelope,
    source_checkpoint: u64,
    source_digest: String,
    encoded_bytes: u32,
    after: Option<InventoryCursor>,
    next: Option<InventoryCursor>,
    records: Vec<SyncRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
enum WireFrame {
    Digest(DigestFrame),
    RangeRequest(RangeRequestFrame),
    RangeBatch(RangeBatchFrame),
    SnapshotRequest(SnapshotRequestFrame),
    SnapshotBatch(SnapshotBatchFrame),
    Cancel(SyncCancelFrame),
}

impl WireFrame {
    fn envelope(&self) -> &SyncEnvelope {
        match self {
            Self::Digest(frame) => &frame.envelope,
            Self::RangeRequest(frame) => &frame.envelope,
            Self::RangeBatch(frame) => &frame.envelope,
            Self::SnapshotRequest(frame) => &frame.envelope,
            Self::SnapshotBatch(frame) => &frame.envelope,
            Self::Cancel(frame) => &frame.envelope,
        }
    }
}

struct RuntimeContext {
    local_node: String,
    mesh: MeshClient,
    boundary: BoundaryClient,
    store: Arc<dyn NodeStorePort>,
    communities: watch::Receiver<Arc<HashSet<String>>>,
    options: SyncRuntimeOptions,
    started: Instant,
    sessions: Mutex<HashMap<String, SyncState>>,
    snapshot_cursors: Mutex<HashMap<String, Option<InventoryCursor>>>,
    advertised: Mutex<HashMap<String, AdvertisedRange>>,
    errors: watch::Sender<Option<String>>,
    counters: Arc<SyncCounters>,
}

impl RuntimeContext {
    async fn run(self, shutdown: CancellationToken) -> Result<(), SyncRuntimeError> {
        let mut messages = self.mesh.subscribe();
        let mut advertise = tokio::time::interval(self.options.advertise_interval);
        advertise.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return Ok(()),
                _ = advertise.tick() => {
                    if let Err(error) = self.expire_and_advertise().await {
                        self.record_error(&error);
                    }
                }
                message = messages.recv() => {
                    match message {
                        Ok(message) => {
                            if let Err(error) = self.handle_message(message.from(), message.payload()).await {
                                self.record_error(&error);
                            }
                        }
                        Err(MeshRuntimeError::SubscriberLagged { skipped }) => {
                            self.record_error(&SyncRuntimeError::Transport(
                                MeshRuntimeError::SubscriberLagged { skipped },
                            ));
                        }
                        Err(error) => return Err(error.into()),
                    }
                }
            }
        }
    }

    fn record_error(&self, error: &SyncRuntimeError) {
        self.errors.send_replace(Some(error.to_string()));
    }

    fn now_tick(&self) -> u64 {
        u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    async fn expire_and_advertise(&self) -> Result<(), SyncRuntimeError> {
        let now = Instant::now();
        self.advertised
            .lock()
            .await
            .retain(|_, range| range.expires_at > now);
        let states = self
            .sessions
            .lock()
            .await
            .iter()
            .filter(|(_, state)| {
                matches!(state.phase, SyncPhase::WaitingBatch | SyncPhase::Applying)
            })
            .map(|(session, state)| (session.clone(), state.clone()))
            .collect::<Vec<_>>();
        for (session, state) in states {
            let result = self
                .policy(SyncPolicyRequest::CheckDeadline {
                    state,
                    now_tick: self.now_tick(),
                })
                .await?;
            let SyncPolicyResult::CheckDeadline { result } = result else {
                return Err(SyncRuntimeError::UnexpectedPolicyResult);
            };
            if result.effect == SyncEffect::Cancel {
                self.sessions.lock().await.remove(&session);
                self.snapshot_cursors.lock().await.remove(&session);
            } else {
                self.sessions.lock().await.insert(session, result.state);
            }
        }

        let peers = self.mesh.peers().await?;
        let communities = self.communities.borrow().clone();
        for peer in peers {
            for community_id in communities.iter() {
                self.advertise_to(peer, community_id).await?;
            }
        }
        Ok(())
    }

    async fn advertise_to(&self, peer: NodeId, community_id: &str) -> Result<(), SyncRuntimeError> {
        let state =
            canonical_state_digest(self.store.as_ref(), community_id, MAX_PAGE_SIZE, || false)?;
        let target_node = node_name(peer);
        let session_id = session_id(
            community_id,
            &self.local_node,
            &target_node,
            state.checkpoint,
            state.digest,
        );
        let timeout = self
            .options
            .session_timeout
            .checked_mul(2)
            .unwrap_or(self.options.session_timeout);
        let expires_at = Instant::now()
            .checked_add(timeout)
            .ok_or(SyncRuntimeError::InvalidConfiguration)?;
        let now = Instant::now();
        let next_advertise = now
            .checked_add(self.options.advertise_interval.max(Duration::from_secs(1)))
            .ok_or(SyncRuntimeError::InvalidConfiguration)?;
        let mut advertised = self.advertised.lock().await;
        if advertised
            .get(&session_id)
            .is_some_and(|current| current.next_advertise > now)
        {
            return Ok(());
        }
        advertised.insert(
            session_id.clone(),
            AdvertisedRange {
                community_id: community_id.to_owned(),
                target_node: target_node.clone(),
                state,
                next_advertise,
                expires_at,
            },
        );
        drop(advertised);
        self.send(
            peer,
            WireFrame::Digest(DigestFrame {
                envelope: SyncEnvelope {
                    protocol: SYNC_PROTOCOL.to_owned(),
                    version: SYNC_VERSION,
                    session_id,
                    community_id: community_id.to_owned(),
                    sender_node_id: self.local_node.clone(),
                    receiver_node_id: target_node,
                },
                checkpoint: state.checkpoint,
                prefix_digest: state.hex(),
            }),
        )
        .await
    }

    async fn handle_message(
        &self,
        authenticated_peer: NodeId,
        payload: &[u8],
    ) -> Result<(), SyncRuntimeError> {
        let Some(payload) = payload.strip_prefix(WIRE_PREFIX) else {
            return Ok(());
        };
        let frame: WireFrame = serde_json::from_slice(payload)
            .map_err(|error| SyncRuntimeError::InvalidFrame(error.to_string()))?;
        self.counters.received.fetch_add(1, Ordering::Relaxed);
        let envelope = frame.envelope();
        let peer_name = node_name(authenticated_peer);
        if envelope.sender_node_id != peer_name
            || envelope.receiver_node_id != self.local_node
            || !self.communities.borrow().contains(&envelope.community_id)
        {
            return Err(SyncRuntimeError::InvalidFrame(
                "authenticated peer or community scope mismatch".to_owned(),
            ));
        }
        match frame {
            WireFrame::Digest(frame) => self.handle_digest(authenticated_peer, frame).await,
            WireFrame::RangeRequest(frame) => {
                self.handle_range_request(authenticated_peer, frame).await
            }
            WireFrame::RangeBatch(mut frame) => {
                frame.encoded_bytes = u32::try_from(payload.len()).unwrap_or(u32::MAX);
                self.handle_range_batch(authenticated_peer, frame).await
            }
            WireFrame::SnapshotRequest(frame) => {
                self.handle_snapshot_request(authenticated_peer, frame)
                    .await
            }
            WireFrame::SnapshotBatch(mut frame) => {
                frame.encoded_bytes = u32::try_from(payload.len()).unwrap_or(u32::MAX);
                self.handle_snapshot_batch(authenticated_peer, frame).await
            }
            WireFrame::Cancel(frame) => self.handle_cancel(frame).await,
        }
    }

    async fn handle_digest(
        &self,
        peer: NodeId,
        frame: DigestFrame,
    ) -> Result<(), SyncRuntimeError> {
        let active = self
            .sessions
            .lock()
            .await
            .get(&frame.envelope.session_id)
            .cloned();
        if let Some(state) = active {
            if state.phase == SyncPhase::WaitingBatch {
                let cursor = self
                    .snapshot_cursors
                    .lock()
                    .await
                    .get(&frame.envelope.session_id)
                    .cloned()
                    .flatten();
                return self.send_snapshot_request(peer, state, cursor).await;
            }
            if state.phase == SyncPhase::Applying {
                return Ok(());
            }
        }
        let local = canonical_state_digest(
            self.store.as_ref(),
            &frame.envelope.community_id,
            MAX_PAGE_SIZE,
            || false,
        )?;
        let state = SyncState {
            valid: true,
            revision: 0,
            phase: SyncPhase::Idle,
            session_id: frame.envelope.session_id.clone(),
            community_id: frame.envelope.community_id.clone(),
            local_node_id: self.local_node.clone(),
            remote_node_id: frame.envelope.sender_node_id.clone(),
            checkpoint: local.checkpoint,
            checkpoint_digest: local.hex(),
            remote_checkpoint: 0,
            remote_digest: String::new(),
            max_records: self.options.max_records,
            max_encoded_bytes: self.options.max_encoded_bytes,
            timeout_ticks: u64::try_from(self.options.session_timeout.as_millis())
                .map_err(|_| SyncRuntimeError::InvalidConfiguration)?,
            deadline_tick: 0,
            pending_batch_id: String::new(),
        };
        let result = self
            .policy(SyncPolicyRequest::AcceptDigest {
                state,
                frame,
                now_tick: self.now_tick(),
            })
            .await?;
        let SyncPolicyResult::AcceptDigest { result } = result else {
            return Err(SyncRuntimeError::UnexpectedPolicyResult);
        };
        self.apply_decision(peer, result).await
    }

    async fn apply_decision(
        &self,
        peer: NodeId,
        decision: SyncDecision,
    ) -> Result<(), SyncRuntimeError> {
        let session_id = decision.state.session_id.clone();
        match decision.effect {
            SyncEffect::RequestRange => {
                self.sessions
                    .lock()
                    .await
                    .insert(session_id, decision.state.clone());
                self.send_next_range(peer, decision.state).await
            }
            SyncEffect::RequestSnapshot => {
                self.sessions
                    .lock()
                    .await
                    .insert(session_id.clone(), decision.state.clone());
                self.snapshot_cursors.lock().await.insert(session_id, None);
                self.send_snapshot_request(peer, decision.state, None).await
            }
            SyncEffect::Complete => {
                self.sessions.lock().await.remove(&session_id);
                self.snapshot_cursors.lock().await.remove(&session_id);
                Ok(())
            }
            SyncEffect::Cancel => {
                self.sessions.lock().await.remove(&session_id);
                self.snapshot_cursors.lock().await.remove(&session_id);
                Ok(())
            }
            SyncEffect::Reject if decision.error == SyncPolicyError::RemoteBehind => Ok(()),
            SyncEffect::Reject => Err(SyncRuntimeError::PolicyRejected(decision.error)),
            _ => Ok(()),
        }
    }

    async fn send_next_range(
        &self,
        peer: NodeId,
        state: SyncState,
    ) -> Result<(), SyncRuntimeError> {
        let result = self.policy(SyncPolicyRequest::NextRange { state }).await?;
        let SyncPolicyResult::NextRange { frame } = result else {
            return Err(SyncRuntimeError::UnexpectedPolicyResult);
        };
        if let Some(frame) = frame {
            self.send(peer, WireFrame::RangeRequest(frame)).await?;
        }
        Ok(())
    }

    async fn send_snapshot_request(
        &self,
        peer: NodeId,
        state: SyncState,
        after: Option<InventoryCursor>,
    ) -> Result<(), SyncRuntimeError> {
        self.send(
            peer,
            WireFrame::SnapshotRequest(SnapshotRequestFrame {
                envelope: SyncEnvelope {
                    protocol: SYNC_PROTOCOL.to_owned(),
                    version: SYNC_VERSION,
                    session_id: state.session_id,
                    community_id: state.community_id,
                    sender_node_id: state.local_node_id,
                    receiver_node_id: state.remote_node_id,
                },
                after,
                limit_records: state.max_records,
                max_encoded_bytes: state.max_encoded_bytes,
            }),
        )
        .await
    }

    async fn handle_snapshot_request(
        &self,
        peer: NodeId,
        frame: SnapshotRequestFrame,
    ) -> Result<(), SyncRuntimeError> {
        let advertised = self
            .advertised
            .lock()
            .await
            .get(&frame.envelope.session_id)
            .cloned();
        let Some(advertised) = advertised else {
            return self.advertise_to(peer, &frame.envelope.community_id).await;
        };
        if advertised.community_id != frame.envelope.community_id
            || advertised.target_node != frame.envelope.sender_node_id
            || frame.limit_records == 0
            || frame.limit_records > self.options.max_records
            || frame.max_encoded_bytes == 0
            || frame.max_encoded_bytes > self.options.max_encoded_bytes
        {
            return Err(SyncRuntimeError::InvalidFrame(
                "snapshot request differs from advertised scope or bounds".to_owned(),
            ));
        }
        let community_id = frame.envelope.community_id.clone();
        if let Some(payload) = self.build_snapshot_batch(frame, advertised)? {
            self.mesh.send(peer, payload).await?;
            self.counters.sent.fetch_add(1, Ordering::Relaxed);
            Ok(())
        } else {
            self.advertise_to(peer, &community_id).await
        }
    }

    fn build_snapshot_batch(
        &self,
        request: SnapshotRequestFrame,
        advertised: AdvertisedRange,
    ) -> Result<Option<Vec<u8>>, SyncRuntimeError> {
        let request_after = request.after.clone();
        let current = canonical_state_digest(
            self.store.as_ref(),
            &request.envelope.community_id,
            MAX_PAGE_SIZE,
            || false,
        )?;
        if current != advertised.state {
            return Ok(None);
        }
        let after = request
            .after
            .as_ref()
            .map(|cursor| (cursor.record_type.as_str(), cursor.key.as_str()));
        let stored = self.store.canonical_page(
            &request.envelope.community_id,
            after,
            usize::from(request.limit_records),
        )?;
        let mut records = stored
            .iter()
            .map(logical_sync_record)
            .collect::<Result<Vec<_>, _>>()?;
        let page_was_full = records.len() == usize::from(request.limit_records);
        let original_len = records.len();
        loop {
            let next = if records.is_empty() || (!page_was_full && records.len() == original_len) {
                None
            } else {
                records.last().map(|record| InventoryCursor {
                    record_type: record.record_type.clone(),
                    key: record.key.clone(),
                })
            };
            let mut batch = SnapshotBatchFrame {
                envelope: SyncEnvelope {
                    protocol: SYNC_PROTOCOL.to_owned(),
                    version: SYNC_VERSION,
                    session_id: request.envelope.session_id.clone(),
                    community_id: request.envelope.community_id.clone(),
                    sender_node_id: self.local_node.clone(),
                    receiver_node_id: request.envelope.sender_node_id.clone(),
                },
                source_checkpoint: current.checkpoint,
                source_digest: current.hex(),
                encoded_bytes: 1,
                after: request_after.clone(),
                next,
                records: records.clone(),
            };
            let first = encode_wire(&WireFrame::SnapshotBatch(batch.clone()))?;
            batch.encoded_bytes = u32::try_from(first.len()).unwrap_or(u32::MAX);
            let encoded = encode_wire(&WireFrame::SnapshotBatch(batch))?;
            if encoded.len() <= request.max_encoded_bytes as usize
                && encoded.len() <= MAX_SYNC_BYTES as usize
            {
                return Ok(Some(encoded));
            }
            if records.is_empty() {
                return Err(SyncRuntimeError::RangeTooLarge);
            }
            records.pop();
        }
    }

    async fn handle_snapshot_batch(
        &self,
        peer: NodeId,
        frame: SnapshotBatchFrame,
    ) -> Result<(), SyncRuntimeError> {
        let state = self
            .sessions
            .lock()
            .await
            .get(&frame.envelope.session_id)
            .cloned();
        let Some(state) = state else {
            return Ok(());
        };
        let expected_cursor = self
            .snapshot_cursors
            .lock()
            .await
            .get(&frame.envelope.session_id)
            .cloned()
            .flatten();
        if frame.after != expected_cursor {
            return Ok(());
        }
        if state.phase != SyncPhase::WaitingBatch
            || frame.source_checkpoint != state.remote_checkpoint
            || frame.source_digest != state.remote_digest
            || frame.encoded_bytes == 0
            || frame.encoded_bytes > state.max_encoded_bytes
            || frame.records.len() > usize::from(state.max_records)
        {
            return Err(SyncRuntimeError::InvalidFrame(
                "snapshot batch differs from accepted session facts".to_owned(),
            ));
        }
        for record in &frame.records {
            verify_logical_sync_record(record)?;
        }
        self.merge_snapshot_records(&frame).await?;
        if !frame.records.is_empty() {
            self.counters.applied.fetch_add(1, Ordering::Relaxed);
        }
        if let Some(next) = frame.next {
            self.snapshot_cursors
                .lock()
                .await
                .insert(frame.envelope.session_id.clone(), Some(next.clone()));
            self.send_snapshot_request(peer, state, Some(next)).await
        } else {
            self.sessions
                .lock()
                .await
                .remove(&frame.envelope.session_id);
            self.snapshot_cursors
                .lock()
                .await
                .remove(&frame.envelope.session_id);
            Ok(())
        }
    }

    async fn merge_snapshot_records(
        &self,
        frame: &SnapshotBatchFrame,
    ) -> Result<(), SyncRuntimeError> {
        for _ in 0..8 {
            let checkpoint = self
                .store
                .canonical_checkpoint(&frame.envelope.community_id)?;
            let mut pairs = Vec::with_capacity(frame.records.len());
            let mut current_records = Vec::with_capacity(frame.records.len());
            for incoming in &frame.records {
                let current = self.store.get(
                    RecordClass::Canonical,
                    &frame.envelope.community_id,
                    &incoming.record_type,
                    &incoming.key,
                )?;
                let current_fact = current.as_ref().map(inventory_fact).transpose()?;
                pairs.push(InventoryMergePair {
                    current: current_fact,
                    incoming: inventory_fact_from_sync(incoming)?,
                });
                current_records.push(current);
            }
            let result = self
                .policy(SyncPolicyRequest::MergeInventory {
                    community_id: frame.envelope.community_id.clone(),
                    records: pairs,
                })
                .await?;
            let SyncPolicyResult::MergeInventory { results } = result else {
                return Err(SyncRuntimeError::UnexpectedPolicyResult);
            };
            if results.len() != frame.records.len() {
                return Err(SyncRuntimeError::InvalidFrame(
                    "Nim returned a different inventory result count".to_owned(),
                ));
            }
            let mut writes = Vec::new();
            let mut quarantines = Vec::new();
            for ((incoming, current), decision) in
                frame.records.iter().zip(&current_records).zip(results)
            {
                match decision.effect {
                    InventoryMergeEffect::Insert | InventoryMergeEffect::Replace => {
                        writes.push(record_write(incoming)?);
                    }
                    InventoryMergeEffect::Keep | InventoryMergeEffect::Duplicate => {}
                    InventoryMergeEffect::Quarantine => quarantines.push((current, incoming)),
                    InventoryMergeEffect::Reject => {
                        return Err(SyncRuntimeError::ConvergenceRejected(decision.error));
                    }
                }
            }
            let commit = if writes.is_empty() {
                if self
                    .store
                    .canonical_checkpoint(&frame.envelope.community_id)?
                    != checkpoint
                {
                    continue;
                }
                Ok(())
            } else {
                match self.store.commit_canonical(CanonicalCommit {
                    intent_id: snapshot_intent(frame),
                    community_id: frame.envelope.community_id.clone(),
                    expected_checkpoint: checkpoint,
                    writes,
                }) {
                    Ok(_) => Ok(()),
                    Err(StoreError::CheckpointConflict { .. }) => continue,
                    Err(error) => Err(error),
                }
            };
            commit?;
            for (current, incoming) in quarantines {
                self.persist_quarantine(&frame.envelope.community_id, current.as_ref(), incoming)?;
            }
            return Ok(());
        }
        Err(StoreError::CheckpointConflict {
            expected: 0,
            actual: self
                .store
                .canonical_checkpoint(&frame.envelope.community_id)?,
        }
        .into())
    }

    fn persist_quarantine(
        &self,
        community_id: &str,
        current: Option<&StoredRecord>,
        incoming: &SyncRecord,
    ) -> Result<(), SyncRuntimeError> {
        let current = current.ok_or_else(|| {
            SyncRuntimeError::InvalidFrame("quarantine requires a current record".to_owned())
        })?;
        let current_digest = hex::encode(canonical_logical_record_digest(current)?);
        let mut digests = [current_digest, incoming.content_digest.clone()];
        digests.sort();
        let key = quarantine_key(community_id, &incoming.record_type, &incoming.key, &digests);
        self.store.append_log(LogAppend {
            intent_id: format!("sync-quarantine:{key}"),
            community_id: community_id.to_owned(),
            entries: vec![RecordWrite {
                record_type: "sync_quarantine_v1".to_owned(),
                key: key.clone(),
                deleted: false,
                value: json!({
                    "recordType": incoming.record_type,
                    "key": incoming.key,
                    "digestBounds": digests,
                }),
            }],
        })?;
        Ok(())
    }

    async fn handle_range_request(
        &self,
        peer: NodeId,
        frame: RangeRequestFrame,
    ) -> Result<(), SyncRuntimeError> {
        let advertised = self
            .advertised
            .lock()
            .await
            .get(&frame.envelope.session_id)
            .cloned()
            .ok_or_else(|| SyncRuntimeError::InvalidFrame("unknown sync session".to_owned()))?;
        if advertised.community_id != frame.envelope.community_id
            || advertised.target_node != frame.envelope.sender_node_id
        {
            return Err(SyncRuntimeError::InvalidFrame(
                "range request differs from advertised scope".to_owned(),
            ));
        }
        let result = self
            .policy(SyncPolicyRequest::PlanRangeRead {
                frame: frame.clone(),
                session_id: frame.envelope.session_id.clone(),
                community_id: advertised.community_id.clone(),
                source_node_id: self.local_node.clone(),
                target_node_id: advertised.target_node,
                source_checkpoint: advertised.state.checkpoint,
            })
            .await?;
        let SyncPolicyResult::PlanRangeRead { plan } = result else {
            return Err(SyncRuntimeError::UnexpectedPolicyResult);
        };
        if !plan.allowed {
            return Err(SyncRuntimeError::PolicyRejected(plan.error));
        }
        let payload = self.build_batch(frame, advertised.state.checkpoint)?;
        self.mesh.send(peer, payload).await?;
        self.counters.sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn build_batch(
        &self,
        request: RangeRequestFrame,
        advertised_checkpoint: u64,
    ) -> Result<Vec<u8>, SyncRuntimeError> {
        let remaining = advertised_checkpoint.saturating_sub(request.after_checkpoint);
        let limit = usize::from(request.limit_records)
            .min(usize::try_from(remaining).unwrap_or(usize::MAX));
        let mut records = self.store.changes(
            &request.envelope.community_id,
            request.after_checkpoint,
            limit,
        )?;
        let base = canonical_prefix_digest_at(
            self.store.as_ref(),
            &request.envelope.community_id,
            request.after_checkpoint,
            MAX_PAGE_SIZE,
            || false,
        )?;
        while !records.is_empty() {
            let through_checkpoint = records
                .last()
                .map_or(request.after_checkpoint, |r| r.sequence);
            let result_digest = extend_prefix_digest(base.digest, &records)?;
            let sync_records = records
                .iter()
                .map(sync_record)
                .collect::<Result<Vec<_>, _>>()?;
            let mut batch = RangeBatchFrame {
                envelope: SyncEnvelope {
                    protocol: SYNC_PROTOCOL.to_owned(),
                    version: SYNC_VERSION,
                    session_id: request.envelope.session_id.clone(),
                    community_id: request.envelope.community_id.clone(),
                    sender_node_id: self.local_node.clone(),
                    receiver_node_id: request.envelope.sender_node_id.clone(),
                },
                batch_id: format!(
                    "{}:{}:{}",
                    request.envelope.session_id, request.after_checkpoint, through_checkpoint
                ),
                base_checkpoint: request.after_checkpoint,
                base_digest: base.hex(),
                through_checkpoint,
                result_digest: hex::encode(result_digest),
                encoded_bytes: 1,
                digest_verified: true,
                records: sync_records,
            };
            let first = encode_wire(&WireFrame::RangeBatch(batch.clone()))?;
            batch.encoded_bytes = u32::try_from(first.len()).unwrap_or(u32::MAX);
            let encoded = encode_wire(&WireFrame::RangeBatch(batch))?;
            if encoded.len() <= request.max_encoded_bytes as usize
                && encoded.len() <= MAX_SYNC_BYTES as usize
            {
                return Ok(encoded);
            }
            records.pop();
        }
        Err(SyncRuntimeError::RangeTooLarge)
    }

    async fn handle_range_batch(
        &self,
        peer: NodeId,
        mut frame: RangeBatchFrame,
    ) -> Result<(), SyncRuntimeError> {
        let state = self
            .sessions
            .lock()
            .await
            .get(&frame.envelope.session_id)
            .cloned()
            .ok_or_else(|| SyncRuntimeError::InvalidFrame("unknown sync session".to_owned()))?;
        frame.digest_verified = verify_batch(&frame)?;
        let result = self
            .policy(SyncPolicyRequest::PlanBatch {
                state,
                frame,
                now_tick: self.now_tick(),
            })
            .await?;
        let SyncPolicyResult::PlanBatch { plan } = result else {
            return Err(SyncRuntimeError::UnexpectedPolicyResult);
        };
        match plan.effect {
            SyncEffect::ApplyBatch => self.commit_plan(peer, plan).await,
            SyncEffect::AcknowledgeDuplicate => {
                let state = plan.next_state;
                self.sessions
                    .lock()
                    .await
                    .insert(state.session_id.clone(), state.clone());
                if state.phase == SyncPhase::WaitingBatch {
                    self.send_next_range(peer, state).await
                } else {
                    Ok(())
                }
            }
            SyncEffect::Reject => Err(SyncRuntimeError::PolicyRejected(plan.error)),
            _ => Ok(()),
        }
    }

    async fn commit_plan(
        &self,
        peer: NodeId,
        plan: RangeBatchPlan,
    ) -> Result<(), SyncRuntimeError> {
        let session_id = plan.inflight_state.session_id.clone();
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), plan.inflight_state.clone());
        let commit = self.store.commit_canonical(CanonicalCommit {
            intent_id: format!("sync:{session_id}:{}", plan.through_checkpoint),
            community_id: plan.inflight_state.community_id.clone(),
            expected_checkpoint: plan.expected_checkpoint,
            writes: plan
                .records
                .iter()
                .map(record_write)
                .collect::<Result<Vec<_>, _>>()?,
        });
        let (store_succeeded, committed_checkpoint) = match &commit {
            Ok(result) => (true, result.checkpoint),
            Err(_) => (false, plan.expected_checkpoint),
        };
        let result = self
            .policy(SyncPolicyRequest::SettleBatch {
                plan: plan.clone(),
                current_state: plan.inflight_state,
                store_succeeded,
                committed_checkpoint,
            })
            .await?;
        let SyncPolicyResult::SettleBatch { result } = result else {
            return Err(SyncRuntimeError::UnexpectedPolicyResult);
        };
        self.apply_decision(peer, result).await?;
        commit?;
        self.counters.applied.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn handle_cancel(&self, frame: SyncCancelFrame) -> Result<(), SyncRuntimeError> {
        let state = self
            .sessions
            .lock()
            .await
            .get(&frame.envelope.session_id)
            .cloned()
            .ok_or_else(|| SyncRuntimeError::InvalidFrame("unknown sync session".to_owned()))?;
        let result = self
            .policy(SyncPolicyRequest::Cancel { state, frame })
            .await?;
        let SyncPolicyResult::Cancel { result } = result else {
            return Err(SyncRuntimeError::UnexpectedPolicyResult);
        };
        if result.effect == SyncEffect::Cancel {
            self.sessions.lock().await.remove(&result.state.session_id);
            self.snapshot_cursors
                .lock()
                .await
                .remove(&result.state.session_id);
        } else {
            self.sessions
                .lock()
                .await
                .insert(result.state.session_id.clone(), result.state);
        }
        Ok(())
    }

    async fn policy(
        &self,
        request: SyncPolicyRequest,
    ) -> Result<SyncPolicyResult, SyncRuntimeError> {
        match self
            .boundary
            .call(
                BoundaryRequest::sync_policy(request),
                CallContext::with_timeout(self.options.policy_timeout),
            )
            .await?
        {
            BoundaryResult::SyncPolicy(result) => Ok(result),
            _ => Err(SyncRuntimeError::UnexpectedPolicyResult),
        }
    }

    async fn send(&self, peer: NodeId, frame: WireFrame) -> Result<(), SyncRuntimeError> {
        let payload = encode_wire(&frame)?;
        if payload.len() > self.options.max_encoded_bytes as usize {
            return Err(SyncRuntimeError::RangeTooLarge);
        }
        self.mesh.send(peer, payload).await?;
        self.counters.sent.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

fn encode_wire(frame: &WireFrame) -> Result<Vec<u8>, SyncRuntimeError> {
    let encoded = serde_json::to_vec(frame)
        .map_err(|error| SyncRuntimeError::InvalidFrame(error.to_string()))?;
    let mut payload = Vec::with_capacity(WIRE_PREFIX.len() + encoded.len());
    payload.extend_from_slice(WIRE_PREFIX);
    payload.extend(encoded);
    Ok(payload)
}

fn validated_communities(
    communities: impl IntoIterator<Item = String>,
) -> Result<HashSet<String>, SyncRuntimeError> {
    let communities = communities.into_iter().collect::<HashSet<_>>();
    if communities
        .iter()
        .any(|community| community.is_empty() || community.as_bytes().contains(&0))
    {
        return Err(SyncRuntimeError::InvalidCommunity);
    }
    Ok(communities)
}

fn node_name(node: NodeId) -> String {
    hex::encode(node.as_bytes())
}

fn session_id(
    community_id: &str,
    source_node: &str,
    target_node: &str,
    checkpoint: u64,
    digest: [u8; 32],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"nimino.sync/v2/session");
    for value in [
        community_id.as_bytes(),
        source_node.as_bytes(),
        target_node.as_bytes(),
    ] {
        hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
        hasher.update(value);
    }
    hasher.update(checkpoint.to_be_bytes());
    hasher.update(digest);
    hex::encode(hasher.finalize())
}

fn inventory_identity(record_type: &str, key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"nimino.sync/v2/identity");
    for value in [record_type.as_bytes(), key.as_bytes()] {
        hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
        hasher.update(value);
    }
    hex::encode(hasher.finalize())
}

fn inventory_fact(record: &StoredRecord) -> Result<InventoryFact, SyncRuntimeError> {
    Ok(InventoryFact {
        record_type: record.record_type.clone(),
        key: record.key.clone(),
        deleted: record.deleted,
        identity: inventory_identity(&record.record_type, &record.key),
        content_digest: hex::encode(canonical_logical_record_digest(record)?),
    })
}

fn inventory_fact_from_sync(record: &SyncRecord) -> Result<InventoryFact, SyncRuntimeError> {
    verify_logical_sync_record(record)?;
    Ok(InventoryFact {
        record_type: record.record_type.clone(),
        key: record.key.clone(),
        deleted: record.deleted,
        identity: inventory_identity(&record.record_type, &record.key),
        content_digest: record.content_digest.clone(),
    })
}

fn logical_sync_record(record: &StoredRecord) -> Result<SyncRecord, SyncRuntimeError> {
    Ok(SyncRecord {
        sequence: record.sequence,
        record_type: record.record_type.clone(),
        key: record.key.clone(),
        deleted: record.deleted,
        payload: serde_json::to_string(&record.value)
            .map_err(|error| SyncRuntimeError::InvalidFrame(error.to_string()))?,
        content_digest: hex::encode(canonical_logical_record_digest(record)?),
    })
}

fn verify_logical_sync_record(record: &SyncRecord) -> Result<(), SyncRuntimeError> {
    let stored = stored_record(record)?;
    let expected = hex::encode(canonical_logical_record_digest(&stored)?);
    if record.content_digest != expected {
        return Err(SyncRuntimeError::InvalidFrame(
            "logical record digest mismatch".to_owned(),
        ));
    }
    Ok(())
}

fn snapshot_intent(frame: &SnapshotBatchFrame) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"nimino.sync/v2/snapshot-intent");
    hasher.update(frame.envelope.session_id.as_bytes());
    hasher.update(frame.source_digest.as_bytes());
    for record in &frame.records {
        hasher.update(record.content_digest.as_bytes());
    }
    format!("sync-snapshot:{}", hex::encode(hasher.finalize()))
}

fn quarantine_key(
    community_id: &str,
    record_type: &str,
    key: &str,
    digest_bounds: &[String; 2],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"nimino.sync/v2/quarantine");
    for value in [
        community_id,
        record_type,
        key,
        &digest_bounds[0],
        &digest_bounds[1],
    ] {
        hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
        hasher.update(value.as_bytes());
    }
    hex::encode(hasher.finalize())
}

fn sync_record(record: &StoredRecord) -> Result<SyncRecord, SyncRuntimeError> {
    Ok(SyncRecord {
        sequence: record.sequence,
        record_type: record.record_type.clone(),
        key: record.key.clone(),
        deleted: record.deleted,
        payload: serde_json::to_string(&record.value)
            .map_err(|error| SyncRuntimeError::InvalidFrame(error.to_string()))?,
        content_digest: hex::encode(canonical_record_digest(record)?),
    })
}

fn stored_record(record: &SyncRecord) -> Result<StoredRecord, SyncRuntimeError> {
    Ok(StoredRecord {
        sequence: record.sequence,
        record_type: record.record_type.clone(),
        key: record.key.clone(),
        deleted: record.deleted,
        value: serde_json::from_str(&record.payload)
            .map_err(|error| SyncRuntimeError::InvalidFrame(error.to_string()))?,
    })
}

fn record_write(record: &SyncRecord) -> Result<RecordWrite, SyncRuntimeError> {
    Ok(RecordWrite {
        record_type: record.record_type.clone(),
        key: record.key.clone(),
        deleted: record.deleted,
        value: serde_json::from_str(&record.payload)
            .map_err(|error| SyncRuntimeError::InvalidFrame(error.to_string()))?,
    })
}

fn verify_batch(frame: &RangeBatchFrame) -> Result<bool, SyncRuntimeError> {
    let records = frame
        .records
        .iter()
        .map(stored_record)
        .collect::<Result<Vec<_>, _>>()?;
    for (wire, stored) in frame.records.iter().zip(&records) {
        if wire.content_digest != hex::encode(canonical_record_digest(stored)?) {
            return Ok(false);
        }
    }
    let base = digest_bytes(&frame.base_digest)?;
    let expected = digest_bytes(&frame.result_digest)?;
    Ok(extend_prefix_digest(base, &records)? == expected)
}

fn digest_bytes(value: &str) -> Result<[u8; 32], SyncRuntimeError> {
    let bytes =
        hex::decode(value).map_err(|error| SyncRuntimeError::InvalidFrame(error.to_string()))?;
    bytes
        .try_into()
        .map_err(|_| SyncRuntimeError::InvalidFrame("digest must contain 32 bytes".to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nimino_store::empty_prefix_digest;

    #[test]
    fn sessions_are_community_and_checkpoint_scoped() {
        let digest = empty_prefix_digest();
        assert_ne!(
            session_id("community-a", "source", "target", 0, digest),
            session_id("community-b", "source", "target", 0, digest)
        );
        assert_ne!(
            session_id("community-a", "source", "target", 0, digest),
            session_id("community-a", "source", "target", 1, digest)
        );
    }
}
