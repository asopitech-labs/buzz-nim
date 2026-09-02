//! Mandatory production composition for Nim policy, Chirps transport, and sync.

use std::{
    collections::BTreeSet,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{Context, Result};
use nimino_boundary::{BoundaryClient, BoundaryConfig, BoundaryRuntime};
use nimino_chirps::{MeshRuntime, MeshRuntimeOptions, NodeConfig};
use nimino_control::{
    AdmissionClient, AdmissionRuntime, ControlRuntime, ControlRuntimeOptions, EphemeralClient,
    EphemeralRuntime, EphemeralRuntimeOptions, LeaseClient, LeaseRuntime,
};
use nimino_db::Db;
use nimino_object_store::{LocalObjectStore, ObjectSyncClient, ObjectSyncRuntime};
use nimino_store::{ControlLogStorePort, NodeStorePort, RedbNodeStore};
use nimino_sync::{SyncRuntime, SyncRuntimeOptions};
use tokio::{net::lookup_host, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::{handlers::event::fan_out_event_to_local_subscribers, state::AppState};

const DEFAULT_BIND_ADDR: &str = "0.0.0.0:7443";
const DEFAULT_WORKER: &str = "/usr/local/bin/nimino-core-worker";
const DEFAULT_IDENTITY: &str = "/var/lib/nimino/cluster/node.identity";
const DEFAULT_CERTIFICATE: &str = "/etc/nimino/chirps/tls.crt";
const DEFAULT_PRIVATE_KEY: &str = "/etc/nimino/chirps/tls.key";
const DEFAULT_TRUST_ANCHOR: &str = "/etc/nimino/chirps/ca.crt";
const DEFAULT_STORE: &str = "/var/lib/nimino/cluster/data.redb";
const DEFAULT_OBJECT_STORE: &str = "/var/lib/nimino/cluster/objects";

#[derive(Debug)]
struct RelayClusterConfig {
    bind_addr: SocketAddr,
    seeds: Vec<String>,
    worker: PathBuf,
    identity: PathBuf,
    certificate: PathBuf,
    private_key: PathBuf,
    trust_anchors: Vec<PathBuf>,
    store: PathBuf,
    object_store: PathBuf,
    refresh_interval: Duration,
    min_peers: usize,
    voters: Vec<String>,
}

impl RelayClusterConfig {
    fn from_env() -> Result<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self> {
        let value = |name: &str, default: &str| {
            lookup(name)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| default.to_owned())
        };
        let bind_addr = value("NIMINO_CHIRPS_BIND_ADDR", DEFAULT_BIND_ADDR)
            .parse()
            .context("NIMINO_CHIRPS_BIND_ADDR must be an IP socket address")?;
        let seeds = lookup("NIMINO_CHIRPS_SEEDS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|seed| !seed.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let trust_anchors = value("NIMINO_CHIRPS_TRUST_ANCHOR_PATHS", DEFAULT_TRUST_ANCHOR)
            .split(',')
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            !trust_anchors.is_empty(),
            "at least one Chirps trust anchor is required"
        );
        let refresh_secs = value("NIMINO_CLUSTER_REFRESH_SECS", "5")
            .parse::<u64>()
            .context("NIMINO_CLUSTER_REFRESH_SECS must be an integer")?;
        anyhow::ensure!(
            (1..=300).contains(&refresh_secs),
            "NIMINO_CLUSTER_REFRESH_SECS must be between 1 and 300"
        );
        let min_peers = value("NIMINO_CLUSTER_MIN_PEERS", "0")
            .parse::<usize>()
            .context("NIMINO_CLUSTER_MIN_PEERS must be an integer")?;
        anyhow::ensure!(
            min_peers <= 4096,
            "NIMINO_CLUSTER_MIN_PEERS must not exceed 4096"
        );
        let voters = lookup("NIMINO_CLUSTER_VOTERS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|voter| !voter.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        anyhow::ensure!(
            seeds.is_empty() || !voters.is_empty(),
            "NIMINO_CLUSTER_VOTERS is required when Chirps seeds are configured"
        );
        Ok(Self {
            bind_addr,
            seeds,
            worker: value("NIMINO_BOUNDARY_WORKER", DEFAULT_WORKER).into(),
            identity: value("NIMINO_CHIRPS_IDENTITY_PATH", DEFAULT_IDENTITY).into(),
            certificate: value("NIMINO_CHIRPS_CERTIFICATE_PATH", DEFAULT_CERTIFICATE).into(),
            private_key: value("NIMINO_CHIRPS_PRIVATE_KEY_PATH", DEFAULT_PRIVATE_KEY).into(),
            trust_anchors,
            store: value("NIMINO_NODE_STORE_PATH", DEFAULT_STORE).into(),
            object_store: value("NIMINO_OBJECT_STORE_PATH", DEFAULT_OBJECT_STORE).into(),
            refresh_interval: Duration::from_secs(refresh_secs),
            min_peers,
            voters,
        })
    }
}

/// Lifecycle owner for the mandatory Relay cluster composition.
pub struct RelayClusterRuntime {
    refresh_cancel: CancellationToken,
    refresh_task: JoinHandle<()>,
    sync: SyncRuntime,
    objects: ObjectSyncRuntime,
    ephemeral: EphemeralRuntime,
    admission: AdmissionRuntime,
    lease: LeaseRuntime,
    control: ControlRuntime,
    mesh: MeshRuntime,
    admission_boundary: BoundaryRuntime,
    control_boundary: BoundaryRuntime,
    boundary: BoundaryRuntime,
    store: Arc<RedbNodeStore>,
    ready: Arc<AtomicBool>,
    projection_ready: Arc<AtomicBool>,
}

/// Lifecycle owner for applying remote ephemeral and authorization transitions locally.
pub struct RelayClusterDelivery {
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl RelayClusterDelivery {
    /// Stops remote delivery and releases its bounded subscription.
    pub async fn stop(self) -> Result<()> {
        self.cancel.cancel();
        self.task.await.context("join ephemeral delivery task")
    }
}

/// Required policy and persistence adapters shared by every relay request.
#[derive(Clone)]
pub struct RelayDomainAdapters {
    inner: RelayDomainAdapterInner,
}

#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
enum RelayDomainAdapterInner {
    Active {
        policy: BoundaryClient,
        store: Arc<dyn NodeStorePort>,
        lease: LeaseClient,
        admission: AdmissionClient,
        ephemeral: EphemeralClient,
        objects: ObjectSyncClient,
        node_id: String,
    },
    #[cfg(test)]
    Unavailable,
}

impl RelayDomainAdapters {
    /// Returns the supervised Nim policy client.
    pub fn policy(&self) -> Option<&BoundaryClient> {
        match &self.inner {
            RelayDomainAdapterInner::Active { policy, .. } => Some(policy),
            #[cfg(test)]
            RelayDomainAdapterInner::Unavailable => None,
        }
    }

    /// Returns the per-node canonical store.
    pub fn store(&self) -> Option<&Arc<dyn NodeStorePort>> {
        match &self.inner {
            RelayDomainAdapterInner::Active { store, .. } => Some(store),
            #[cfg(test)]
            RelayDomainAdapterInner::Unavailable => None,
        }
    }

    /// Returns the quorum-backed singleton lease and fencing facade.
    pub fn lease(&self) -> Option<&LeaseClient> {
        match &self.inner {
            RelayDomainAdapterInner::Active { lease, .. } => Some(lease),
            #[cfg(test)]
            RelayDomainAdapterInner::Unavailable => None,
        }
    }

    /// Returns the mandatory quorum-backed request admission facade.
    #[cfg(not(test))]
    pub fn admission(&self) -> &AdmissionClient {
        match &self.inner {
            RelayDomainAdapterInner::Active { admission, .. } => admission,
        }
    }

    /// Returns the quorum-backed admission/control projector when composed.
    pub fn admission_optional(&self) -> Option<&AdmissionClient> {
        match &self.inner {
            RelayDomainAdapterInner::Active { admission, .. } => Some(admission),
            #[cfg(test)]
            RelayDomainAdapterInner::Unavailable => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn admission_for_tests(&self) -> Option<&AdmissionClient> {
        match &self.inner {
            RelayDomainAdapterInner::Active { admission, .. } => Some(admission),
            RelayDomainAdapterInner::Unavailable => None,
        }
    }

    /// Returns the Nim-planned content-addressed object transfer facade.
    pub fn objects(&self) -> Option<&ObjectSyncClient> {
        match &self.inner {
            RelayDomainAdapterInner::Active { objects, .. } => Some(objects),
            #[cfg(test)]
            RelayDomainAdapterInner::Unavailable => None,
        }
    }

    /// Returns the bounded presence and typing convergence facade.
    pub fn ephemeral(&self) -> Option<&EphemeralClient> {
        match &self.inner {
            RelayDomainAdapterInner::Active { ephemeral, .. } => Some(ephemeral),
            #[cfg(test)]
            RelayDomainAdapterInner::Unavailable => None,
        }
    }

    /// Returns this relay's stable Chirps node identity.
    pub fn node_id(&self) -> Option<&str> {
        match &self.inner {
            RelayDomainAdapterInner::Active { node_id, .. } => Some(node_id),
            #[cfg(test)]
            RelayDomainAdapterInner::Unavailable => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn unavailable_for_tests() -> Self {
        Self {
            inner: RelayDomainAdapterInner::Unavailable,
        }
    }
}

impl RelayClusterRuntime {
    /// Starts the worker, mTLS mesh, per-node store, and automatic sync.
    pub async fn start(db: Db, ready: Arc<AtomicBool>) -> Result<Self> {
        let config = RelayClusterConfig::from_env()?;
        if let Some(parent) = config.store.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create node store directory {}", parent.display()))?;
        }
        let store = Arc::new(
            RedbNodeStore::open(&config.store)
                .with_context(|| format!("open node store {}", config.store.display()))?,
        );
        let object_store = Arc::new(
            LocalObjectStore::open(&config.object_store)
                .with_context(|| format!("open object store {}", config.object_store.display()))?,
        );
        let communities = active_communities(&db).await?;
        let boundary = BoundaryRuntime::start(BoundaryConfig::new(&config.worker))
            .await
            .with_context(|| format!("start Nim worker {}", config.worker.display()))?;
        let control_boundary =
            match BoundaryRuntime::start(BoundaryConfig::new(&config.worker)).await {
                Ok(boundary) => boundary,
                Err(error) => {
                    let _ = boundary.shutdown().await;
                    return Err(error).with_context(|| {
                        format!("start control Nim worker {}", config.worker.display())
                    });
                }
            };
        let admission_boundary =
            match BoundaryRuntime::start(BoundaryConfig::new(&config.worker)).await {
                Ok(boundary) => boundary,
                Err(error) => {
                    let _ = control_boundary.shutdown().await;
                    let _ = boundary.shutdown().await;
                    return Err(error).with_context(|| {
                        format!("start admission Nim worker {}", config.worker.display())
                    });
                }
            };
        let seeds = match resolve_seeds(&config.seeds).await {
            Ok(seeds) => seeds,
            Err(error) => {
                let _ = admission_boundary.shutdown().await;
                let _ = control_boundary.shutdown().await;
                let _ = boundary.shutdown().await;
                return Err(error);
            }
        };
        let node_config = NodeConfig::new(
            config.bind_addr,
            config.identity,
            config.certificate,
            config.private_key,
            config.trust_anchors,
        )
        .with_seeds(seeds);
        let mesh = match MeshRuntime::start(node_config, MeshRuntimeOptions::default()).await {
            Ok(mesh) => mesh,
            Err(error) => {
                let _ = admission_boundary.shutdown().await;
                let _ = control_boundary.shutdown().await;
                let _ = boundary.shutdown().await;
                return Err(error.into());
            }
        };
        let local_node_id = hex::encode(mesh.client().local_node_id().as_bytes());
        let voters = if config.voters.is_empty() {
            vec![local_node_id]
        } else {
            config.voters.clone()
        };
        let control_store: Arc<dyn ControlLogStorePort> = store.clone();
        let control = match ControlRuntime::start(
            mesh.client(),
            control_boundary.client(),
            control_store,
            voters,
            ControlRuntimeOptions::default(),
        )
        .await
        {
            Ok(control) => control,
            Err(error) => {
                let _ = mesh.stop().await;
                let _ = admission_boundary.shutdown().await;
                let _ = control_boundary.shutdown().await;
                let _ = boundary.shutdown().await;
                return Err(error.into());
            }
        };
        let control_client = control.client();
        let admission_store: Arc<dyn ControlLogStorePort> = store.clone();
        let admission = match AdmissionRuntime::start(
            control_client.clone(),
            admission_boundary.client(),
            admission_store,
            Duration::from_secs(2),
        )
        .await
        {
            Ok(admission) => admission,
            Err(error) => {
                let _ = control.stop().await;
                let _ = mesh.stop().await;
                let _ = admission_boundary.shutdown().await;
                let _ = control_boundary.shutdown().await;
                let _ = boundary.shutdown().await;
                return Err(error.into());
            }
        };
        let lease_store: Arc<dyn ControlLogStorePort> = store.clone();
        let lease = match LeaseRuntime::start(
            control_client.clone(),
            boundary.client(),
            lease_store,
            Duration::from_secs(2),
        )
        .await
        {
            Ok(lease) => lease,
            Err(error) => {
                let _ = admission.stop().await;
                let _ = control.stop().await;
                let _ = mesh.stop().await;
                let _ = admission_boundary.shutdown().await;
                let _ = control_boundary.shutdown().await;
                let _ = boundary.shutdown().await;
                return Err(error.into());
            }
        };
        let ephemeral = match EphemeralRuntime::start(
            mesh.client(),
            boundary.client(),
            communities.clone(),
            EphemeralRuntimeOptions::default(),
        ) {
            Ok(ephemeral) => ephemeral,
            Err(error) => {
                let _ = lease.stop().await;
                let _ = admission.stop().await;
                let _ = control.stop().await;
                let _ = mesh.stop().await;
                let _ = admission_boundary.shutdown().await;
                let _ = control_boundary.shutdown().await;
                let _ = boundary.shutdown().await;
                return Err(error.into());
            }
        };
        let node_store: Arc<dyn NodeStorePort> = store.clone();
        let sync = match SyncRuntime::start(
            mesh.client(),
            boundary.client(),
            node_store,
            communities,
            SyncRuntimeOptions::default(),
        ) {
            Ok(sync) => sync,
            Err(error) => {
                let _ = ephemeral.stop().await;
                let _ = lease.stop().await;
                let _ = admission.stop().await;
                let _ = control.stop().await;
                let _ = mesh.stop().await;
                let _ = admission_boundary.shutdown().await;
                let _ = control_boundary.shutdown().await;
                let _ = boundary.shutdown().await;
                return Err(error.into());
            }
        };
        let sync_client = sync.client();
        let objects = match ObjectSyncRuntime::start(
            mesh.client(),
            boundary.client(),
            object_store,
            Duration::from_secs(5),
        ) {
            Ok(objects) => objects,
            Err(error) => {
                let _ = sync.stop().await;
                let _ = ephemeral.stop().await;
                let _ = lease.stop().await;
                let _ = admission.stop().await;
                let _ = control.stop().await;
                let _ = mesh.stop().await;
                let _ = admission_boundary.shutdown().await;
                let _ = control_boundary.shutdown().await;
                let _ = boundary.shutdown().await;
                return Err(error.into());
            }
        };
        let object_client = objects.client();
        let ephemeral_client = ephemeral.client();
        let admission_client = admission.client();
        let lease_client = lease.client();
        let mesh_client = mesh.client();
        let projection_ready = Arc::new(AtomicBool::new(false));
        ready.store(false, Ordering::Release);
        let refresh_cancel = CancellationToken::new();
        let task_cancel = refresh_cancel.clone();
        let task_ready = Arc::clone(&ready);
        let task_projection_ready = Arc::clone(&projection_ready);
        let refresh_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(config.refresh_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = task_cancel.cancelled() => break,
                    _ = interval.tick() => {
                        match mesh_client.peers().await {
                            Ok(peers) => {
                                metrics::gauge!("nimino_cluster_peers").set(peers.len() as f64);
                                task_ready.store(
                                    sync_client.is_running()
                                        && object_client.is_running()
                                        && control_client.status().running
                                        && control_client.status().quorum_available
                                        && ephemeral_client.is_running()
                                        && admission_client.status().running
                                        && lease_client.status().running
                                        && peers.len() >= config.min_peers
                                        && task_projection_ready.load(Ordering::Acquire),
                                    Ordering::Release,
                                );
                            }
                            Err(error) => {
                                task_ready.store(false, Ordering::Release);
                                tracing::warn!(%error, "cluster peer health check failed");
                            }
                        }
                        match active_communities(&db).await {
                            Ok(communities) => {
                                if let Err(error) = sync_client.replace_communities(communities.clone()) {
                                    tracing::error!(%error, "cluster community scope update failed");
                                }
                                if let Err(error) = ephemeral_client.replace_scopes(communities) {
                                    tracing::error!(%error, "ephemeral community scope update failed");
                                }
                            }
                            Err(error) => tracing::warn!(%error, "cluster community scope refresh failed"),
                        }
                        if let Some(error) = sync_client.last_error() {
                            tracing::warn!(%error, "automatic cluster sync reported an error");
                        }
                    }
                }
            }
        });
        Ok(Self {
            refresh_cancel,
            refresh_task,
            sync,
            objects,
            ephemeral,
            admission,
            lease,
            control,
            mesh,
            admission_boundary,
            control_boundary,
            boundary,
            store,
            ready,
            projection_ready,
        })
    }

    /// Returns the exact worker and store used by cluster synchronization.
    pub fn domain_adapters(&self) -> RelayDomainAdapters {
        RelayDomainAdapters {
            inner: RelayDomainAdapterInner::Active {
                policy: self.boundary.client(),
                store: self.store.clone(),
                lease: self.lease.client(),
                admission: self.admission.client(),
                ephemeral: self.ephemeral.client(),
                objects: self.objects.client(),
                node_id: hex::encode(self.mesh.client().local_node_id().as_bytes()),
            },
        }
    }

    /// Gate set by the canonical-to-query projection consumer after catch-up.
    pub fn projection_ready_gate(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.projection_ready)
    }

    /// Starts relay-local projection and fan-out for authenticated remote transitions.
    pub fn start_delivery(&self, state: Arc<AppState>) -> RelayClusterDelivery {
        let mut updates = self.ephemeral.client().subscribe_remote();
        let mut invalidations = self.admission.client().subscribe_invalidations();
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = task_cancel.cancelled() => break,
                    update = updates.recv() => match update {
                        Ok(update) => {
                            if let Err(error) = deliver_remote_ephemeral(&state, update).await {
                                tracing::warn!(%error, "remote ephemeral transition rejected");
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(skipped, "remote ephemeral delivery lagged; re-advertisement will repair state");
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    },
                    invalidation = invalidations.recv() => match invalidation {
                        Ok(invalidation) => {
                            if let Err(error) = apply_remote_authorization_invalidation(&state, invalidation).await {
                                tracing::warn!(%error, "remote authorization invalidation rejected");
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!(skipped, "authorization invalidation delivery lagged; running durable revalidation");
                            state.revalidate_live_authorization().await;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            }
        });
        RelayClusterDelivery { cancel, task }
    }

    /// Stops sync before transport and reaps the Nim workers last.
    pub async fn stop(self) -> Result<()> {
        self.ready.store(false, Ordering::Release);
        self.refresh_cancel.cancel();
        let refresh_result = self.refresh_task.await;
        let object_result = self.objects.shutdown().await;
        let sync_result = self.sync.stop().await;
        let ephemeral_result = self.ephemeral.stop().await;
        let lease_result = self.lease.stop().await;
        let admission_result = self.admission.stop().await;
        let control_result = self.control.stop().await;
        let mesh_result = self.mesh.stop().await;
        let admission_boundary_result = self.admission_boundary.shutdown().await;
        let control_boundary_result = self.control_boundary.shutdown().await;
        let boundary_result = self.boundary.shutdown().await;
        refresh_result.context("join cluster health task")?;
        object_result.context("stop object replication")?;
        sync_result.context("stop automatic sync")?;
        ephemeral_result.context("stop ephemeral convergence")?;
        lease_result.context("stop lease projection")?;
        admission_result.context("stop admission projection")?;
        control_result.context("stop replicated control")?;
        mesh_result.context("stop Chirps mesh")?;
        admission_boundary_result.context("stop admission Nim worker")?;
        control_boundary_result.context("stop control Nim worker")?;
        boundary_result.context("stop Nim worker")?;
        Ok(())
    }
}

async fn deliver_remote_ephemeral(
    state: &AppState,
    update: nimino_control::EphemeralUpdate,
) -> Result<()> {
    if !update.state.active {
        return Ok(());
    }
    let event_json = update
        .event_json
        .context("active ephemeral transition omitted its signed event")?;
    let event: nostr::Event =
        serde_json::from_str(&event_json).context("decode ephemeral event")?;
    let verified = event.clone();
    tokio::task::spawn_blocking(move || nimino_core::verification::verify_event(&verified))
        .await
        .context("join ephemeral signature verification")?
        .context("verify ephemeral event")?;
    anyhow::ensure!(
        event.pubkey.to_hex() == update.state.subject
            && event.id.to_hex() == update.state.transition_id
            && event.created_at.as_secs().saturating_mul(1_000) == update.state.observed_at_ms,
        "signed event and converged transition facts disagree"
    );
    let community = nimino_core::CommunityId::from_uuid(
        uuid::Uuid::parse_str(&update.state.scope).context("parse ephemeral community scope")?,
    );
    let channel_id = match update.state.kind {
        nimino_boundary::EphemeralKind::Presence => {
            anyhow::ensure!(
                nimino_core::kind::event_kind_u32(&event)
                    == nimino_core::kind::KIND_PRESENCE_UPDATE
                    && update.state.context.is_empty()
                    && crate::handlers::event::normalized_presence_status(&event.content)
                        == update.state.value,
                "presence event and converged state disagree"
            );
            None
        }
        nimino_boundary::EphemeralKind::Typing => {
            let channel = crate::handlers::ingest::extract_channel_id(&event)
                .context("typing event omitted its channel scope")?;
            anyhow::ensure!(
                nimino_core::kind::event_kind_u32(&event)
                    == nimino_core::kind::KIND_TYPING_INDICATOR
                    && channel.to_string() == update.state.context
                    && update.state.value == "typing",
                "typing event and converged state disagree"
            );
            Some(channel)
        }
    };
    let stored = nimino_core::StoredEvent::new(event, channel_id);
    fan_out_event_to_local_subscribers(state, community, &stored).await;
    Ok(())
}

async fn apply_remote_authorization_invalidation(
    state: &AppState,
    invalidation: nimino_boundary::AuthorizationInvalidationState,
) -> Result<()> {
    let community = nimino_core::CommunityId::from_uuid(
        uuid::Uuid::parse_str(&invalidation.scope)
            .context("parse authorization invalidation community")?,
    );
    match invalidation.kind {
        nimino_boundary::AuthorizationInvalidationKind::Ban => {
            let pubkey = hex::decode(&invalidation.subject)
                .context("decode authorization invalidation subject")?;
            anyhow::ensure!(pubkey.len() == 32, "authorization subject must be 32 bytes");
            let restriction = state
                .db
                .moderation_restriction_state(community, &pubkey)
                .await
                .context("revalidate remote ban")?;
            if restriction.banned {
                let event_id = if invalidation.fact_id.len() == 64 {
                    invalidation.fact_id
                } else {
                    "0".repeat(64)
                };
                state.conn_manager.disconnect_pubkey(
                    community,
                    &pubkey,
                    &event_id,
                    "blocked: you are banned from this community",
                );
            }
        }
        nimino_boundary::AuthorizationInvalidationKind::Membership => {
            let channel_id = uuid::Uuid::parse_str(&invalidation.channel_id)
                .context("parse membership invalidation channel")?;
            let pubkey = hex::decode(&invalidation.subject)
                .context("decode membership invalidation subject")?;
            anyhow::ensure!(pubkey.len() == 32, "membership subject must be 32 bytes");
            state.invalidate_membership_local(community, channel_id, &pubkey);
            if !durable_channel_access(state, community, channel_id, &pubkey).await {
                state
                    .evict_live_channel_subscriptions_local(community, channel_id, &pubkey)
                    .await;
            }
        }
        nimino_boundary::AuthorizationInvalidationKind::Visibility => {
            let channel_id = uuid::Uuid::parse_str(&invalidation.channel_id)
                .context("parse visibility invalidation channel")?;
            state.invalidate_all_accessible_channels_local(community);
            state.invalidate_channel_visibility_local(community, channel_id);
            for conn_id in state
                .sub_registry
                .channel_subscriber_conns_scoped(community, channel_id)
            {
                let Some(pubkey) = state.conn_manager.pubkey_for_conn(conn_id) else {
                    continue;
                };
                if !durable_channel_access(state, community, channel_id, &pubkey).await {
                    state
                        .evict_live_channel_subscriptions_local(community, channel_id, &pubkey)
                        .await;
                }
            }
        }
        nimino_boundary::AuthorizationInvalidationKind::Community => {
            state.invalidate_channel_deleted_local(community);
            if !state
                .db
                .is_community_active(community)
                .await
                .context("revalidate remote community lifecycle")?
            {
                state.community_connections.disconnect_community(community);
            }
        }
    }
    Ok(())
}

async fn durable_channel_access(
    state: &AppState,
    community: nimino_core::CommunityId,
    channel_id: uuid::Uuid,
    pubkey: &[u8],
) -> bool {
    match state.db.get_channel(community, channel_id).await {
        Ok(channel) if channel.visibility != "private" => true,
        Ok(_) => state
            .db
            .is_member(community, channel_id, pubkey)
            .await
            .unwrap_or(false),
        Err(_) => false,
    }
}

async fn active_communities(db: &Db) -> Result<Vec<String>> {
    Ok(db
        .active_community_ids()
        .await
        .context("load active cluster communities")?
        .into_iter()
        .map(|id| id.to_string())
        .collect())
}

async fn resolve_seeds(seeds: &[String]) -> Result<Vec<SocketAddr>> {
    let mut resolved = BTreeSet::new();
    for seed in seeds {
        let addresses = lookup_host(seed)
            .await
            .with_context(|| format!("resolve NIMINO_CHIRPS_SEEDS entry {seed}"))?;
        resolved.extend(addresses);
    }
    Ok(resolved.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_parses_dns_seeds_and_rejects_unbounded_refresh() {
        let config = RelayClusterConfig::from_lookup(|name| match name {
            "NIMINO_CHIRPS_SEEDS" => Some("node-a:7443, node-b:7443".to_owned()),
            "NIMINO_CLUSTER_VOTERS" => Some("00".repeat(16)),
            _ => None,
        })
        .expect("valid defaults");
        assert_eq!(config.seeds, ["node-a:7443", "node-b:7443"]);
        assert!(RelayClusterConfig::from_lookup(|name| {
            (name == "NIMINO_CHIRPS_SEEDS").then(|| "node-a:7443".to_owned())
        })
        .is_err());
        assert!(RelayClusterConfig::from_lookup(|name| {
            (name == "NIMINO_CLUSTER_REFRESH_SECS").then(|| "0".to_owned())
        })
        .is_err());
        assert!(RelayClusterConfig::from_lookup(|name| {
            (name == "NIMINO_CLUSTER_MIN_PEERS").then(|| "4097".to_owned())
        })
        .is_err());
    }
}
