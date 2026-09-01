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
use nimino_control::{ControlRuntime, ControlRuntimeOptions, LeaseClient, LeaseRuntime};
use nimino_db::Db;
use nimino_object_store::{LocalObjectStore, ObjectSyncClient, ObjectSyncRuntime};
use nimino_store::{ControlLogStorePort, NodeStorePort, RedbNodeStore};
use nimino_sync::{SyncRuntime, SyncRuntimeOptions};
use tokio::{net::lookup_host, task::JoinHandle};
use tokio_util::sync::CancellationToken;

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
    lease: LeaseRuntime,
    control: ControlRuntime,
    mesh: MeshRuntime,
    boundary: BoundaryRuntime,
    store: Arc<RedbNodeStore>,
    ready: Arc<AtomicBool>,
    projection_ready: Arc<AtomicBool>,
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

    /// Returns the Nim-planned content-addressed object transfer facade.
    pub fn objects(&self) -> Option<&ObjectSyncClient> {
        match &self.inner {
            RelayDomainAdapterInner::Active { objects, .. } => Some(objects),
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
        let seeds = match resolve_seeds(&config.seeds).await {
            Ok(seeds) => seeds,
            Err(error) => {
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
            boundary.client(),
            control_store,
            voters,
            ControlRuntimeOptions::default(),
        )
        .await
        {
            Ok(control) => control,
            Err(error) => {
                let _ = mesh.stop().await;
                let _ = boundary.shutdown().await;
                return Err(error.into());
            }
        };
        let control_client = control.client();
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
                let _ = control.stop().await;
                let _ = mesh.stop().await;
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
                let _ = lease.stop().await;
                let _ = control.stop().await;
                let _ = mesh.stop().await;
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
                let _ = lease.stop().await;
                let _ = control.stop().await;
                let _ = mesh.stop().await;
                let _ = boundary.shutdown().await;
                return Err(error.into());
            }
        };
        let object_client = objects.client();
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
                                if let Err(error) = sync_client.replace_communities(communities) {
                                    tracing::error!(%error, "cluster community scope update failed");
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
            lease,
            control,
            mesh,
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
                objects: self.objects.client(),
                node_id: hex::encode(self.mesh.client().local_node_id().as_bytes()),
            },
        }
    }

    /// Gate set by the canonical-to-query projection consumer after catch-up.
    pub fn projection_ready_gate(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.projection_ready)
    }

    /// Stops sync before transport and reaps the Nim worker last.
    pub async fn stop(self) -> Result<()> {
        self.ready.store(false, Ordering::Release);
        self.refresh_cancel.cancel();
        let refresh_result = self.refresh_task.await;
        let object_result = self.objects.shutdown().await;
        let sync_result = self.sync.stop().await;
        let lease_result = self.lease.stop().await;
        let control_result = self.control.stop().await;
        let mesh_result = self.mesh.stop().await;
        let boundary_result = self.boundary.shutdown().await;
        refresh_result.context("join cluster health task")?;
        object_result.context("stop object replication")?;
        sync_result.context("stop automatic sync")?;
        lease_result.context("stop lease projection")?;
        control_result.context("stop replicated control")?;
        mesh_result.context("stop Chirps mesh")?;
        boundary_result.context("stop Nim worker")?;
        Ok(())
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
