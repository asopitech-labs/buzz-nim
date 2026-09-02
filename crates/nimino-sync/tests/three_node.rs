use std::{
    fs,
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use nimino_boundary::{BoundaryConfig, BoundaryRuntime};
use nimino_chirps::{
    MeshClient, MeshRuntime, MeshRuntimeError, MeshRuntimeOptions, NodeConfig, NodeId,
    MAX_MESSAGE_BYTES,
};
use nimino_store::{
    canonical_state_digest, CanonicalCommit, NodeStorePort, RecordClass, RecordWrite,
    RedbNodeStore, MAX_PAGE_SIZE,
};
use nimino_sync::{SyncRuntime, SyncRuntimeOptions};
use rcgen::generate_simple_self_signed;
use serde_json::json;
use tempfile::TempDir;

const STEP_TIMEOUT: Duration = Duration::from_secs(15);

struct ClusterMaterial {
    root: TempDir,
    certificate: PathBuf,
    private_key: PathBuf,
}

impl ClusterMaterial {
    fn new() -> Self {
        let root = TempDir::new().expect("cluster tempdir");
        let certificate =
            generate_simple_self_signed(["alopex.local".to_owned()]).expect("certificate");
        let certificate_path = root.path().join("cluster.crt");
        let private_key_path = root.path().join("cluster.key");
        fs::write(
            &certificate_path,
            certificate.serialize_der().expect("certificate DER"),
        )
        .expect("write certificate");
        fs::write(&private_key_path, certificate.serialize_private_key_der())
            .expect("write private key");
        #[cfg(unix)]
        fs::set_permissions(&private_key_path, fs::Permissions::from_mode(0o600))
            .expect("secure key");
        Self {
            root,
            certificate: certificate_path,
            private_key: private_key_path,
        }
    }

    fn config(&self, index: usize, bind_addr: SocketAddr, seeds: Vec<SocketAddr>) -> NodeConfig {
        NodeConfig::new(
            bind_addr,
            self.root.path().join(format!("node-{index}.identity")),
            self.certificate.clone(),
            self.private_key.clone(),
            vec![self.certificate.clone()],
        )
        .with_seeds(seeds)
    }

    fn store(&self, index: usize) -> Arc<RedbNodeStore> {
        Arc::new(
            RedbNodeStore::open(self.root.path().join(format!("node-{index}.redb")))
                .expect("open store"),
        )
    }
}

fn free_addr() -> SocketAddr {
    UdpSocket::bind("127.0.0.1:0")
        .expect("reserve UDP port")
        .local_addr()
        .expect("read UDP port")
}

fn writes(first: u64, last: u64) -> Vec<RecordWrite> {
    (first..=last)
        .map(|sequence| RecordWrite {
            record_type: "event".to_owned(),
            key: format!("event-{sequence}"),
            deleted: false,
            value: json!({"sequence": sequence}),
        })
        .collect()
}

async fn wait_for_connection(client: &MeshClient, peer: NodeId) {
    for _ in 0..240 {
        if client.send(peer, b"transport-probe".to_vec()).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("peer did not become reachable");
}

async fn wait_for_peer(client: &MeshClient, peer: NodeId) {
    for _ in 0..240 {
        if client.peers().await.expect("peer view").contains(&peer) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("peer view did not include node");
}

async fn wait_for_digest(stores: &[Arc<RedbNodeStore>], community: &str, checkpoint: u64) -> bool {
    tokio::time::timeout(STEP_TIMEOUT, async {
        loop {
            let states = stores
                .iter()
                .map(|store| {
                    canonical_state_digest(store.as_ref(), community, MAX_PAGE_SIZE, || false)
                })
                .collect::<Result<Vec<_>, _>>();
            let converged = states.is_ok_and(|states| {
                states.first().is_some_and(|expected| {
                    expected.checkpoint == checkpoint
                        && states.iter().all(|actual| actual == expected)
                })
            });
            if converged {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or(false)
}

async fn wait_for_quarantine(stores: &[Arc<RedbNodeStore>], community: &str) -> bool {
    tokio::time::timeout(STEP_TIMEOUT, async {
        loop {
            if stores.iter().any(|store| {
                store
                    .page(RecordClass::Log, community, "sync_quarantine_v1", None, 1)
                    .is_ok_and(|records| !records.is_empty())
            }) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the production Nim worker; run `just nimino-sync-scenarios`"]
async fn three_nodes_bootstrap_resume_and_isolate_communities_over_real_chirps() {
    let worker = std::env::var_os("NIMINO_BOUNDARY_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_WORKER must point to the production worker");
    let boundary = BoundaryRuntime::start(BoundaryConfig::new(worker))
        .await
        .expect("start Nim boundary");
    let material = ClusterMaterial::new();
    let addresses = [free_addr(), free_addr(), free_addr()];
    let mesh0 = MeshRuntime::start(
        material.config(0, addresses[0], Vec::new()),
        MeshRuntimeOptions::default(),
    )
    .await
    .expect("start source mesh");
    let mesh1 = MeshRuntime::start(
        material.config(1, addresses[1], vec![addresses[0]]),
        MeshRuntimeOptions::default(),
    )
    .await
    .expect("start target one mesh");
    let mesh2 = MeshRuntime::start(
        material.config(2, addresses[2], vec![addresses[0]]),
        MeshRuntimeOptions::new(64, 1),
    )
    .await
    .expect("start target two mesh");
    wait_for_connection(&mesh1.client(), mesh0.local_node_id()).await;
    wait_for_connection(&mesh2.client(), mesh0.local_node_id()).await;
    wait_for_peer(&mesh0.client(), mesh1.local_node_id()).await;
    wait_for_peer(&mesh0.client(), mesh2.local_node_id()).await;
    wait_for_peer(&mesh1.client(), mesh0.local_node_id()).await;
    wait_for_peer(&mesh2.client(), mesh0.local_node_id()).await;

    let mut slow_peer = mesh2.client().subscribe();
    for sequence in 0..3 {
        mesh0
            .client()
            .send(mesh2.local_node_id(), vec![sequence])
            .await
            .expect("send slow-peer probe");
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    let slow_peer_skipped = match slow_peer.recv().await {
        Err(MeshRuntimeError::SubscriberLagged { skipped }) if skipped >= 2 => skipped,
        result => panic!("slow peer did not report bounded lag: {result:?}"),
    };

    let stores = [material.store(0), material.store(1), material.store(2)];
    stores[0]
        .commit_canonical(CanonicalCommit {
            intent_id: "seed-community-a".to_owned(),
            community_id: "community-a".to_owned(),
            expected_checkpoint: 0,
            writes: writes(1, 4),
        })
        .expect("seed source");
    stores[0]
        .commit_canonical(CanonicalCommit {
            intent_id: "seed-private-community".to_owned(),
            community_id: "community-private".to_owned(),
            expected_checkpoint: 0,
            writes: writes(1, 1),
        })
        .expect("seed excluded community");

    let options = SyncRuntimeOptions::new(
        Duration::from_millis(200),
        Duration::from_secs(5),
        Duration::from_secs(1),
        2,
        MAX_MESSAGE_BYTES as u32,
    );
    let mut observed1 = mesh1.client().subscribe();
    let mut observed2 = mesh2.client().subscribe();
    let sync0 = SyncRuntime::start(
        mesh0.client(),
        boundary.client(),
        stores[0].clone(),
        ["community-a".to_owned()],
        options,
    )
    .expect("start source sync");
    let sync1 = SyncRuntime::start(
        mesh1.client(),
        boundary.client(),
        stores[1].clone(),
        ["community-a".to_owned()],
        options,
    )
    .expect("start target one sync");
    let sync2 = SyncRuntime::start(
        mesh2.client(),
        boundary.client(),
        stores[2].clone(),
        ["community-a".to_owned()],
        options,
    )
    .expect("start target two sync");
    tokio::time::timeout(Duration::from_secs(2), observed1.recv())
        .await
        .expect("target one receives source sync traffic")
        .expect("target one transport remains live");
    tokio::time::timeout(Duration::from_secs(2), observed2.recv())
        .await
        .expect("target two receives source sync traffic")
        .expect("target two transport remains live");

    assert!(
        wait_for_digest(&stores, "community-a", 4).await,
        "bootstrap failed: checkpoints={:?}, errors={:?}, stats={:?}",
        stores
            .iter()
            .map(|store| store.canonical_checkpoint("community-a"))
            .collect::<Vec<_>>(),
        [
            sync0.client().last_error(),
            sync1.client().last_error(),
            sync2.client().last_error(),
        ],
        [
            sync0.client().stats(),
            sync1.client().stats(),
            sync2.client().stats(),
        ]
    );
    assert_eq!(
        stores[1].canonical_checkpoint("community-private").unwrap(),
        0
    );
    assert_eq!(
        stores[2].canonical_checkpoint("community-private").unwrap(),
        0
    );

    sync1.stop().await.expect("stop target one sync");
    stores[0]
        .commit_canonical(CanonicalCommit {
            intent_id: "append-community-a".to_owned(),
            community_id: "community-a".to_owned(),
            expected_checkpoint: 4,
            writes: writes(5, 6),
        })
        .expect("append source");
    let resumed1 = SyncRuntime::start(
        mesh1.client(),
        boundary.client(),
        stores[1].clone(),
        ["community-a".to_owned()],
        options,
    )
    .expect("resume target one sync");
    assert!(
        wait_for_digest(&stores, "community-a", 6).await,
        "resume failed: checkpoints={:?}, errors={:?}, stats={:?}",
        stores
            .iter()
            .map(|store| store.canonical_checkpoint("community-a"))
            .collect::<Vec<_>>(),
        [
            sync0.client().last_error(),
            resumed1.client().last_error(),
            sync2.client().last_error(),
        ],
        [
            sync0.client().stats(),
            resumed1.client().stats(),
            sync2.client().stats(),
        ]
    );
    tokio::time::sleep(Duration::from_millis(450)).await;
    assert_eq!(stores[1].canonical_checkpoint("community-a").unwrap(), 6);
    assert!(sync0.client().last_error().is_none());
    assert!(resumed1.client().last_error().is_none());
    let slow_peer_runtime_warning = sync2
        .client()
        .last_error()
        .unwrap_or_else(|| "none".to_owned());
    assert!(
        slow_peer_runtime_warning == "none"
            || slow_peer_runtime_warning.contains("subscriber missed"),
        "unexpected slow-peer warning: {slow_peer_runtime_warning}"
    );
    let bootstrap_stats = [
        sync0.client().stats(),
        resumed1.client().stats(),
        sync2.client().stats(),
    ];
    let duplicate_checkpoint = stores[1]
        .canonical_checkpoint("community-a")
        .expect("duplicate checkpoint");

    sync0.stop().await.expect("stop source sync");
    resumed1.stop().await.expect("stop resumed sync");
    sync2.stop().await.expect("stop target two sync");

    stores[0]
        .commit_canonical(CanonicalCommit {
            intent_id: "divergent-node-zero".to_owned(),
            community_id: "community-a".to_owned(),
            expected_checkpoint: 6,
            writes: writes(7, 7),
        })
        .expect("append divergent node-zero event");
    stores[1]
        .commit_canonical(CanonicalCommit {
            intent_id: "divergent-node-one".to_owned(),
            community_id: "community-a".to_owned(),
            expected_checkpoint: 6,
            writes: writes(8, 8),
        })
        .expect("append divergent node-one event");
    let converging0 = SyncRuntime::start(
        mesh0.client(),
        boundary.client(),
        stores[0].clone(),
        ["community-a".to_owned()],
        options,
    )
    .expect("restart node zero sync");
    let converging1 = SyncRuntime::start(
        mesh1.client(),
        boundary.client(),
        stores[1].clone(),
        ["community-a".to_owned()],
        options,
    )
    .expect("restart node one sync");
    let converging2 = SyncRuntime::start(
        mesh2.client(),
        boundary.client(),
        stores[2].clone(),
        ["community-a".to_owned()],
        options,
    )
    .expect("restart node two sync");
    assert!(
        wait_for_digest(&stores, "community-a", 8).await,
        "divergent histories did not converge: checkpoints={:?}, errors={:?}, states={:?}, keys={:?}",
        stores
            .iter()
            .map(|store| store.canonical_checkpoint("community-a"))
            .collect::<Vec<_>>(),
        [
            converging0.client().last_error(),
            converging1.client().last_error(),
            converging2.client().last_error(),
        ],
        stores
            .iter()
            .map(|store| canonical_state_digest(store.as_ref(), "community-a", MAX_PAGE_SIZE, || false)
                .map(|state| state.hex()))
            .collect::<Vec<_>>(),
        stores
            .iter()
            .map(|store| store
                .canonical_page("community-a", None, MAX_PAGE_SIZE)
                .map(|records| records.into_iter().map(|record| record.key).collect::<Vec<_>>()))
            .collect::<Vec<_>>(),
    );
    for store in &stores {
        assert!(store
            .get(RecordClass::Canonical, "community-a", "event", "event-7")
            .expect("read event seven")
            .is_some());
        assert!(store
            .get(RecordClass::Canonical, "community-a", "event", "event-8")
            .expect("read event eight")
            .is_some());
    }
    let convergence_stats = [
        converging0.client().stats(),
        converging1.client().stats(),
        converging2.client().stats(),
    ];
    converging0.stop().await.expect("stop converged node zero");
    converging1.stop().await.expect("stop converged node one");
    converging2.stop().await.expect("stop converged node two");

    for (index, content) in [(0, "alpha"), (1, "beta")] {
        stores[index]
            .commit_canonical(CanonicalCommit {
                intent_id: format!("collision-node-{index}"),
                community_id: "community-a".to_owned(),
                expected_checkpoint: 8,
                writes: vec![RecordWrite {
                    record_type: "event".to_owned(),
                    key: "event-9".to_owned(),
                    deleted: false,
                    value: json!({"content": content}),
                }],
            })
            .expect("append colliding identity");
    }
    let colliding0 = SyncRuntime::start(
        mesh0.client(),
        boundary.client(),
        stores[0].clone(),
        ["community-a".to_owned()],
        options,
    )
    .expect("restart collision node zero");
    let colliding1 = SyncRuntime::start(
        mesh1.client(),
        boundary.client(),
        stores[1].clone(),
        ["community-a".to_owned()],
        options,
    )
    .expect("restart collision node one");
    let colliding2 = SyncRuntime::start(
        mesh2.client(),
        boundary.client(),
        stores[2].clone(),
        ["community-a".to_owned()],
        options,
    )
    .expect("restart collision node two");
    assert!(
        wait_for_quarantine(&stores, "community-a").await,
        "same identity with different content was not quarantined"
    );
    assert_eq!(
        stores[0]
            .get(RecordClass::Canonical, "community-a", "event", "event-9")
            .expect("read first collision")
            .expect("first collision remains")
            .value,
        json!({"content": "alpha"})
    );
    assert_eq!(
        stores[1]
            .get(RecordClass::Canonical, "community-a", "event", "event-9")
            .expect("read second collision")
            .expect("second collision remains")
            .value,
        json!({"content": "beta"})
    );
    colliding0.stop().await.expect("stop collision node zero");
    colliding1.stop().await.expect("stop collision node one");
    colliding2.stop().await.expect("stop collision node two");
    boundary.shutdown().await.expect("stop Nim boundary");
    mesh0.stop().await.expect("stop source mesh");
    mesh1.stop().await.expect("stop target one mesh");
    mesh2.stop().await.expect("stop target two mesh");
    for address in addresses {
        UdpSocket::bind(address).expect("mesh socket released");
    }
    let output = std::env::var_os("NIMINO_SYNC_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/nim/nimino-sync-scenarios.json"));
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent).expect("create evidence directory");
    }
    fs::write(
        output,
        serde_json::to_vec_pretty(&json!({
            "schemaVersion": 1,
            "contract": "nimino.sync-scenarios",
            "transport": "alopex-chirps/0.6.3-udp-quic-mtls",
            "nodeCount": 3,
            "maxRecords": 2,
            "bootstrapCheckpoint": 4,
            "resumedCheckpoint": 6,
            "divergentCheckpoint": 8,
            "divergentHistoriesConverged": true,
            "identityCollisionQuarantined": true,
            "duplicateCheckpoint": duplicate_checkpoint,
            "communityIsolation": stores[1]
                .canonical_checkpoint("community-private")
                .expect("excluded community checkpoint") == 0,
            "slowPeerSkippedFrames": slow_peer_skipped,
            "slowPeerRuntimeWarning": slow_peer_runtime_warning,
            "slowPeerRecoveredCheckpoint": stores[2]
                .canonical_checkpoint("community-a")
                .expect("slow peer checkpoint"),
            "releasedUdpSockets": 3,
            "bootstrapNodes": bootstrap_stats.map(|value| json!({
                "sentFrames": value.sent_frames,
                "receivedFrames": value.received_frames,
                "appliedBatches": value.applied_batches,
            })),
            "convergenceNodes": convergence_stats.map(|value| json!({
                "sentFrames": value.sent_frames,
                "receivedFrames": value.received_frames,
                "appliedBatches": value.applied_batches,
            })),
        }))
        .expect("encode sync evidence"),
    )
    .expect("write sync evidence");
}
