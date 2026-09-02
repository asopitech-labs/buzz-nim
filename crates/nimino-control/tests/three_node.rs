use std::{
    fs,
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use nimino_boundary::{
    AuthorizationInvalidationKind, BoundaryConfig, BoundaryRuntime, EphemeralEffect, EphemeralKind,
    LeaseFenceError, SingletonEffectAttempt,
};
use nimino_chirps::{MeshClient, MeshRuntime, MeshRuntimeOptions, NodeConfig, NodeId};
use nimino_control::{
    AdmissionRuntime, AdmissionRuntimeError, ControlClient, ControlRuntime, ControlRuntimeError,
    ControlRuntimeOptions, EphemeralClient, EphemeralRuntime, EphemeralRuntimeError,
    EphemeralRuntimeOptions, LeaseClient, LeaseRuntime, LeaseRuntimeError,
};
use nimino_store::{ControlLogStorePort, RedbNodeStore};
use rcgen::generate_simple_self_signed;
use tempfile::TempDir;

const WAIT: Duration = Duration::from_secs(30);

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

fn node_name(node: NodeId) -> String {
    hex::encode(node.as_bytes())
}

async fn wait_for_peer(client: &MeshClient, peer: NodeId, direction: &str) {
    tokio::time::timeout(WAIT, async {
        loop {
            if client.peers().await.expect("peer view").contains(&peer) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{direction}: peer does not become reachable"));
}

async fn wait_for_connection(client: &MeshClient, peer: NodeId) {
    tokio::time::timeout(WAIT, async {
        loop {
            if client
                .send(peer, b"control-transport-probe".to_vec())
                .await
                .is_ok()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("peer accepts a connection probe");
}

async fn wait_for_leader(clients: &[ControlClient]) -> usize {
    tokio::time::timeout(WAIT, async {
        loop {
            let statuses = clients
                .iter()
                .map(ControlClient::status)
                .collect::<Vec<_>>();
            let leaders = statuses
                .iter()
                .filter_map(|status| status.leader_id.as_deref())
                .collect::<std::collections::BTreeSet<_>>();
            if leaders.len() == 1 && statuses.iter().all(|status| status.quorum_available) {
                let leader = *leaders.first().expect("one leader");
                return statuses
                    .iter()
                    .position(|status| status.local_node_id == leader)
                    .expect("leader is one cluster node");
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "cluster did not elect one live-quorum leader: {:?}",
            clients
                .iter()
                .map(ControlClient::status)
                .collect::<Vec<_>>()
        )
    })
}

async fn wait_for_commit(stores: &[Arc<RedbNodeStore>], expected: u64) {
    let result = tokio::time::timeout(WAIT, async {
        loop {
            if stores.iter().all(|store| {
                store.recover_control_state().is_ok_and(|state| {
                    state.metadata.state.commit_index == expected
                        && state.metadata.state.applied_index == expected
                })
            }) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    if result.is_err() {
        panic!(
            "all stores did not reach {expected}: {:?}",
            stores
                .iter()
                .map(|store| store.recover_control_state())
                .collect::<Vec<_>>()
        );
    }
}

async fn wait_for_active_lease(clients: &[LeaseClient], expected_fence: u64) {
    tokio::time::timeout(WAIT, async {
        loop {
            let mut converged = true;
            for client in clients {
                converged &= client
                    .state("workflow-scheduler")
                    .await
                    .as_ref()
                    .is_some_and(|state| {
                        state.last_fence_token == expected_fence
                            && state
                                .active_lease
                                .as_ref()
                                .is_some_and(|lease| lease.fence_token == expected_fence)
                    });
            }
            if converged {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("all live lease projections converge");
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test clock")
        .as_millis()
        .try_into()
        .expect("test time fits u64")
}

async fn wait_for_presence(client: &EphemeralClient, scope: &str, subject: &str, value: &str) {
    tokio::time::timeout(WAIT, async {
        loop {
            let presence = client
                .presence(scope, vec![subject.to_owned()])
                .await
                .expect("query presence");
            if presence
                .get(subject)
                .is_some_and(|current| current == value)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("presence converges");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the production Nim worker; run `just nimino-control-scenarios`"]
async fn three_node_ephemeral_state_converges_after_reorder_expiry_and_rejoin() {
    const COMMUNITY: &str = "00000000-0000-0000-0000-000000000001";
    const OTHER_COMMUNITY: &str = "00000000-0000-0000-0000-000000000002";
    const SUBJECT: &str = "0101010101010101010101010101010101010101010101010101010101010101";
    const ONLINE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const AWAY: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const TYPING: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    let worker = std::env::var_os("NIMINO_BOUNDARY_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_WORKER must point to the production worker");
    let mut boundaries = Vec::new();
    for _ in 0..3 {
        boundaries.push(
            BoundaryRuntime::start(BoundaryConfig::new(&worker))
                .await
                .expect("start Nim boundary"),
        );
    }
    let material = ClusterMaterial::new();
    let addresses = [free_addr(), free_addr(), free_addr()];
    let mut meshes = vec![
        Some(
            MeshRuntime::start(
                material.config(0, addresses[0], Vec::new()),
                MeshRuntimeOptions::default(),
            )
            .await
            .expect("start mesh zero"),
        ),
        Some(
            MeshRuntime::start(
                material.config(1, addresses[1], vec![addresses[0], addresses[2]]),
                MeshRuntimeOptions::default(),
            )
            .await
            .expect("start mesh one"),
        ),
        Some(
            MeshRuntime::start(
                material.config(2, addresses[2], vec![addresses[0], addresses[1]]),
                MeshRuntimeOptions::default(),
            )
            .await
            .expect("start mesh two"),
        ),
    ];
    wait_for_connection(
        &meshes[1].as_ref().expect("mesh one").client(),
        meshes[0].as_ref().expect("mesh zero").local_node_id(),
    )
    .await;
    wait_for_connection(
        &meshes[2].as_ref().expect("mesh two").client(),
        meshes[0].as_ref().expect("mesh zero").local_node_id(),
    )
    .await;

    let options =
        EphemeralRuntimeOptions::new(Duration::from_millis(200), Duration::from_secs(5), 64, 64);
    let mut runtimes = (0..3)
        .map(|index| {
            Some(
                EphemeralRuntime::start(
                    meshes[index].as_ref().expect("running mesh").client(),
                    boundaries[index].client(),
                    [COMMUNITY.to_owned()],
                    options,
                )
                .expect("start ephemeral runtime"),
            )
        })
        .collect::<Vec<_>>();
    let mut clients = runtimes
        .iter()
        .map(|runtime| runtime.as_ref().expect("running runtime").client())
        .collect::<Vec<_>>();

    let observed = unix_ms();
    assert_eq!(
        clients[0]
            .publish_presence(COMMUNITY, SUBJECT, "online", observed, ONLINE, "{}")
            .await
            .expect("publish online")
            .effect,
        EphemeralEffect::Apply
    );
    wait_for_presence(&clients[1], COMMUNITY, SUBJECT, "online").await;
    wait_for_presence(&clients[2], COMMUNITY, SUBJECT, "online").await;
    assert_eq!(
        clients[0]
            .publish_presence(COMMUNITY, SUBJECT, "away", observed + 1, AWAY, "{}")
            .await
            .expect("publish newer presence")
            .effect,
        EphemeralEffect::Apply
    );
    assert_eq!(
        clients[0]
            .publish_presence(COMMUNITY, SUBJECT, "online", observed, ONLINE, "{}")
            .await
            .expect("reordered presence")
            .effect,
        EphemeralEffect::Stale
    );
    wait_for_presence(&clients[1], COMMUNITY, SUBJECT, "away").await;
    assert!(matches!(
        clients[0]
            .publish_presence(OTHER_COMMUNITY, SUBJECT, "online", observed, ONLINE, "{}")
            .await,
        Err(EphemeralRuntimeError::ScopeUnavailable)
    ));

    let mut typing_updates = clients[1].subscribe_remote();
    assert_eq!(
        clients[0]
            .publish_typing(
                COMMUNITY,
                SUBJECT,
                "00000000-0000-0000-0000-000000000010",
                "typing",
                unix_ms(),
                TYPING,
                "{}",
            )
            .await
            .expect("publish typing")
            .effect,
        EphemeralEffect::Apply
    );
    let update = tokio::time::timeout(WAIT, typing_updates.recv())
        .await
        .expect("typing reaches node one")
        .expect("typing update");
    assert_eq!(update.state.kind, EphemeralKind::Typing);

    runtimes[2]
        .take()
        .expect("node two runtime")
        .stop()
        .await
        .expect("stop partitioned runtime");
    meshes[2]
        .take()
        .expect("node two mesh")
        .stop()
        .await
        .expect("stop partitioned mesh");
    let rejoin_observed = unix_ms();
    clients[0]
        .publish_presence(COMMUNITY, SUBJECT, "rejoined", rejoin_observed, AWAY, "{}")
        .await
        .expect("publish during partition");
    meshes[2] = Some(
        MeshRuntime::start(
            material.config(2, addresses[2], vec![addresses[0], addresses[1]]),
            MeshRuntimeOptions::default(),
        )
        .await
        .expect("restart mesh two"),
    );
    wait_for_connection(
        &meshes[2].as_ref().expect("rejoined mesh").client(),
        meshes[0].as_ref().expect("mesh zero").local_node_id(),
    )
    .await;
    runtimes[2] = Some(
        EphemeralRuntime::start(
            meshes[2].as_ref().expect("rejoined mesh").client(),
            boundaries[2].client(),
            [COMMUNITY.to_owned()],
            options,
        )
        .expect("restart ephemeral runtime"),
    );
    clients[2] = runtimes[2].as_ref().expect("rejoined runtime").client();
    wait_for_presence(&clients[2], COMMUNITY, SUBJECT, "rejoined").await;

    let tombstone = clients[0]
        .clear_presence(COMMUNITY, SUBJECT)
        .await
        .expect("publish disconnect tombstone");
    assert_eq!(tombstone.effect, EphemeralEffect::Apply);
    tokio::time::timeout(WAIT, async {
        loop {
            if clients[1]
                .presence(COMMUNITY, vec![SUBJECT.to_owned()])
                .await
                .expect("query tombstone")
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("tombstone converges");

    tokio::time::sleep(Duration::from_secs(11)).await;
    assert_eq!(
        clients[0]
            .publish_typing(
                COMMUNITY,
                SUBJECT,
                "00000000-0000-0000-0000-000000000010",
                "typing",
                observed,
                TYPING,
                "{}",
            )
            .await
            .expect("expired typing is classified")
            .effect,
        EphemeralEffect::Expired
    );

    for runtime in runtimes.into_iter().flatten() {
        runtime.stop().await.expect("stop ephemeral runtime");
    }
    for mesh in meshes.into_iter().flatten() {
        mesh.stop().await.expect("stop mesh runtime");
    }
    for address in addresses {
        UdpSocket::bind(address).expect("Chirps released UDP socket");
    }
    for boundary in boundaries {
        boundary.shutdown().await.expect("stop Nim boundary");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the production Nim worker; run `just nimino-control-scenarios`"]
async fn one_and_five_nodes_enforce_rate_budget_and_release_ephemeral_resources() {
    const COMMUNITY: &str = "00000000-0000-0000-0000-000000000001";
    let worker = std::env::var_os("NIMINO_BOUNDARY_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_WORKER must point to the production worker");

    for count in [1_usize, 5] {
        let material = ClusterMaterial::new();
        let addresses = (0..count).map(|_| free_addr()).collect::<Vec<_>>();
        let mut boundaries = Vec::new();
        let mut control_boundaries = Vec::new();
        let mut meshes = Vec::new();
        for index in 0..count {
            boundaries.push(
                BoundaryRuntime::start(BoundaryConfig::new(&worker))
                    .await
                    .expect("start Nim boundary"),
            );
            control_boundaries.push(
                BoundaryRuntime::start(BoundaryConfig::new(&worker))
                    .await
                    .expect("start control Nim boundary"),
            );
            meshes.push(
                MeshRuntime::start(
                    material.config(index, addresses[index], addresses[..index].to_vec()),
                    MeshRuntimeOptions::default(),
                )
                .await
                .expect("start mesh"),
            );
            for peer in 0..index {
                wait_for_connection(&meshes[index].client(), meshes[peer].local_node_id()).await;
            }
        }
        for left in 0..count {
            for right in 0..count {
                if left != right {
                    wait_for_peer(
                        &meshes[left].client(),
                        meshes[right].local_node_id(),
                        &format!("{left}->{right}"),
                    )
                    .await;
                }
            }
        }

        let voters = meshes
            .iter()
            .map(|mesh| node_name(mesh.local_node_id()))
            .collect::<Vec<_>>();
        let stores = (0..count)
            .map(|index| material.store(index))
            .collect::<Vec<_>>();
        let options = ControlRuntimeOptions::new(
            Duration::from_millis(250),
            Duration::from_secs(5),
            Duration::from_secs(5),
            64,
        );
        let mut controls = Vec::new();
        for index in 0..count {
            controls.push(
                ControlRuntime::start(
                    meshes[index].client(),
                    control_boundaries[index].client(),
                    stores[index].clone(),
                    voters.clone(),
                    options,
                )
                .await
                .expect("start control runtime"),
            );
        }
        let control_clients = controls
            .iter()
            .map(ControlRuntime::client)
            .collect::<Vec<_>>();
        wait_for_leader(&control_clients).await;
        let mut admissions = Vec::new();
        for index in 0..count {
            admissions.push(
                AdmissionRuntime::start(
                    control_clients[index].clone(),
                    boundaries[index].client(),
                    stores[index].clone(),
                    Duration::from_secs(5),
                )
                .await
                .expect("start admission runtime"),
            );
        }
        let admission_clients = admissions
            .iter()
            .map(AdmissionRuntime::client)
            .collect::<Vec<_>>();
        let request_count = if count == 1 { 128 } else { count.max(3) };
        let mut allowed = 0;
        for request in 0..request_count {
            let result = admission_clients[request % count]
                .consume_rate_at(
                    "principal",
                    "community-a:alice:api",
                    60,
                    2,
                    60_000 + u64::try_from(request).expect("small request count"),
                )
                .await
                .unwrap_or_else(|error| {
                    panic!(
                        "{count}-node request {request} failed: {error}; control={:?}",
                        control_clients
                            .iter()
                            .map(ControlClient::status)
                            .collect::<Vec<_>>()
                    )
                });
            allowed += usize::from(result.allowed);
            assert_eq!(
                result.current,
                u64::try_from(request + 1).expect("small count")
            );
        }
        assert_eq!(allowed, 2, "{count} nodes multiplied the cluster budget");
        wait_for_commit(
            &stores,
            u64::try_from(request_count).expect("small request count"),
        )
        .await;

        let mut runtimes = Vec::new();
        for index in 0..count {
            runtimes.push(
                EphemeralRuntime::start(
                    meshes[index].client(),
                    boundaries[index].client(),
                    [COMMUNITY.to_owned()],
                    EphemeralRuntimeOptions::new(
                        Duration::from_millis(200),
                        Duration::from_secs(5),
                        64,
                        64,
                    ),
                )
                .expect("start ephemeral runtime"),
            );
        }
        for (index, runtime) in runtimes.iter().enumerate() {
            let byte = u8::try_from(index + 1).expect("small node count");
            runtime
                .client()
                .publish_presence(
                    COMMUNITY,
                    format!("{byte:02x}").repeat(32),
                    "online",
                    unix_ms(),
                    format!("{:064x}", index + 1),
                    "{}",
                )
                .await
                .expect("publish active state before shutdown");
        }
        for runtime in runtimes {
            runtime.stop().await.expect("stop ephemeral runtime");
        }
        for admission in admissions {
            admission.stop().await.expect("stop admission runtime");
        }
        for control in controls {
            control.stop().await.expect("stop control runtime");
        }
        for mesh in meshes {
            mesh.stop().await.expect("stop mesh runtime");
        }
        for address in addresses {
            UdpSocket::bind(address).expect("Chirps released UDP socket");
        }
        for boundary in boundaries {
            boundary.shutdown().await.expect("stop Nim boundary");
        }
        for boundary in control_boundaries {
            boundary
                .shutdown()
                .await
                .expect("stop control Nim boundary");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the production Nim worker; run `just nimino-control-scenarios`"]
async fn three_nodes_elect_commit_fail_closed_and_catch_up_over_real_chirps() {
    let worker = std::env::var_os("NIMINO_BOUNDARY_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_WORKER must point to the production worker");
    let mut boundaries = Vec::new();
    let mut control_boundaries = Vec::new();
    for _ in 0..3 {
        boundaries.push(
            BoundaryRuntime::start(BoundaryConfig::new(&worker))
                .await
                .expect("start per-node Nim boundary"),
        );
        control_boundaries.push(
            BoundaryRuntime::start(BoundaryConfig::new(&worker))
                .await
                .expect("start per-node control boundary"),
        );
    }
    let material = ClusterMaterial::new();
    let addresses = [free_addr(), free_addr(), free_addr()];
    let meshes = [
        MeshRuntime::start(
            material.config(0, addresses[0], Vec::new()),
            MeshRuntimeOptions::default(),
        )
        .await
        .expect("start mesh zero"),
        MeshRuntime::start(
            material.config(1, addresses[1], vec![addresses[0], addresses[2]]),
            MeshRuntimeOptions::default(),
        )
        .await
        .expect("start mesh one"),
        MeshRuntime::start(
            material.config(2, addresses[2], vec![addresses[0], addresses[1]]),
            MeshRuntimeOptions::default(),
        )
        .await
        .expect("start mesh two"),
    ];
    wait_for_connection(&meshes[1].client(), meshes[0].local_node_id()).await;
    wait_for_connection(&meshes[2].client(), meshes[0].local_node_id()).await;
    wait_for_connection(&meshes[2].client(), meshes[1].local_node_id()).await;
    for left in 0..3 {
        for right in 0..3 {
            if left != right {
                wait_for_peer(
                    &meshes[left].client(),
                    meshes[right].local_node_id(),
                    &format!("{left}->{right}"),
                )
                .await;
            }
        }
    }

    let voters = meshes
        .iter()
        .map(|mesh| node_name(mesh.local_node_id()))
        .collect::<Vec<_>>();
    let stores = [material.store(0), material.store(1), material.store(2)];
    let options = ControlRuntimeOptions::new(
        Duration::from_millis(250),
        Duration::from_secs(5),
        Duration::from_secs(5),
        64,
    );
    let mut controls = Vec::new();
    for index in 0..3 {
        controls.push(Some(
            ControlRuntime::start(
                meshes[index].client(),
                control_boundaries[index].client(),
                stores[index].clone(),
                voters.clone(),
                options,
            )
            .await
            .expect("start control runtime"),
        ));
    }
    let clients = controls
        .iter()
        .map(|control| control.as_ref().expect("running").client())
        .collect::<Vec<_>>();
    let mut admissions = Vec::new();
    for index in 0..3 {
        admissions.push(Some(
            AdmissionRuntime::start(
                clients[index].clone(),
                boundaries[index].client(),
                stores[index].clone(),
                Duration::from_secs(5),
            )
            .await
            .expect("start admission runtime"),
        ));
    }
    let admission_clients = admissions
        .iter()
        .map(|admission| admission.as_ref().expect("running admission").client())
        .collect::<Vec<_>>();
    let mut leases = Vec::new();
    for index in 0..3 {
        leases.push(Some(
            LeaseRuntime::start(
                clients[index].clone(),
                boundaries[index].client(),
                stores[index].clone(),
                Duration::from_secs(2),
            )
            .await
            .expect("start lease runtime"),
        ));
    }
    let lease_clients = leases
        .iter()
        .map(|lease| lease.as_ref().expect("running lease").client())
        .collect::<Vec<_>>();
    let leader = wait_for_leader(&clients).await;
    let active = lease_clients[leader]
        .grant("workflow-scheduler", "lease-1", voters.clone(), 60_000)
        .await
        .expect("quorum commits first lease");
    assert_eq!(active.fence_token, 1);
    wait_for_active_lease(&lease_clients, 1).await;
    assert_eq!(wait_for_leader(&clients).await, leader);
    let route = lease_clients[leader]
        .route("workflow-scheduler")
        .await
        .expect("route decision");
    assert!(
        route.allowed,
        "route rejected with {:?}; control={:?}; lease={:?}",
        route.error,
        clients[leader].status(),
        lease_clients[leader].state("workflow-scheduler").await
    );
    assert_eq!(route.owner_id, active.owner_id);
    assert!(
        lease_clients[leader]
            .authorize(SingletonEffectAttempt {
                resource_id: "workflow-scheduler".to_owned(),
                owner_id: active.owner_id,
                fence_token: 1,
            })
            .await
            .expect("effect decision")
            .allowed
    );

    assert_eq!(wait_for_leader(&clients).await, leader);
    let follower = (leader + 1) % clients.len();
    let entry = tokio::time::timeout(WAIT, async {
        loop {
            match clients[follower]
                .propose("command-1", r#"{"operation":"first"}"#)
                .await
            {
                Ok(entry) => break entry,
                Err(
                    ControlRuntimeError::LeaderRequired
                    | ControlRuntimeError::QuorumRequired
                    | ControlRuntimeError::PendingEntry,
                ) => tokio::time::sleep(Duration::from_millis(25)).await,
                Err(error) => panic!("follower proposal failed: {error}"),
            }
        }
    })
    .await
    .expect("follower forwards and quorum commits first command");
    assert_eq!(entry.index, 2);
    wait_for_commit(&stores, 2).await;
    assert_eq!(wait_for_leader(&clients).await, leader);

    let event_id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let first_follower = (leader + 1) % clients.len();
    let second_follower = (leader + 2) % clients.len();
    let (first, second) = tokio::join!(
        admission_clients[first_follower].claim_replay_at("community-a", event_id, 120, 1_000,),
        admission_clients[second_follower].claim_replay_at("community-a", event_id, 120, 1_001,),
    );
    let claims = [
        first.unwrap_or_else(|error| {
            panic!(
                "first distributed claim failed: {error}; control={:?}",
                clients
                    .iter()
                    .map(ControlClient::status)
                    .collect::<Vec<_>>()
            )
        }),
        second.unwrap_or_else(|error| {
            panic!(
                "second distributed claim failed: {error}; control={:?}",
                clients
                    .iter()
                    .map(ControlClient::status)
                    .collect::<Vec<_>>()
            )
        }),
    ];
    assert_eq!(claims.into_iter().filter(|allowed| *allowed).count(), 1);
    wait_for_commit(&stores, 4).await;

    let (rate_a, rate_b, rate_c) = tokio::join!(
        admission_clients[0].consume_rate_at("principal", "community-a:alice:api", 60, 2, 60_001),
        admission_clients[1].consume_rate_at("principal", "community-a:alice:api", 60, 2, 60_002),
        admission_clients[2].consume_rate_at("principal", "community-a:alice:api", 60, 2, 60_003),
    );
    let mut rate = [
        rate_a.expect("node zero rate consume"),
        rate_b.expect("node one rate consume"),
        rate_c.expect("node two rate consume"),
    ];
    rate.sort_by_key(|result| result.current);
    assert_eq!(
        rate.iter().map(|result| result.current).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    assert_eq!(rate.iter().filter(|result| result.allowed).count(), 2);
    wait_for_commit(&stores, 7).await;

    let invalidation_source = first_follower;
    let mut invalidation_receivers = admission_clients
        .iter()
        .map(|client| client.subscribe_invalidations())
        .collect::<Vec<_>>();
    let revision = admission_clients[invalidation_source]
        .publish_invalidation(
            "community-a",
            AuthorizationInvalidationKind::Membership,
            event_id,
            "channel-a",
            "membership-a",
        )
        .await
        .expect("quorum commits authorization invalidation");
    assert_eq!(revision, 8);
    wait_for_commit(&stores, 8).await;
    for (index, receiver) in invalidation_receivers.iter_mut().enumerate() {
        if index == invalidation_source {
            continue;
        }
        let invalidation = tokio::time::timeout(WAIT, receiver.recv())
            .await
            .expect("invalidation reaches remote projector")
            .expect("remote invalidation update");
        assert_eq!(invalidation.revision, 8);
        assert_eq!(invalidation.kind, AuthorizationInvalidationKind::Membership);
    }

    let stopped = (0..3).filter(|index| *index != leader).collect::<Vec<_>>();
    for index in &stopped {
        admissions[*index]
            .take()
            .expect("running follower admission")
            .stop()
            .await
            .expect("stop follower admission");
        leases[*index]
            .take()
            .expect("running follower lease")
            .stop()
            .await
            .expect("stop follower lease");
        controls[*index]
            .take()
            .expect("running follower")
            .stop()
            .await
            .expect("stop follower control");
    }
    tokio::time::timeout(WAIT, async {
        while clients[leader].status().quorum_available {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "leader retained stale quorum: {:?}",
            clients[leader].status()
        )
    });
    assert!(matches!(
        clients[leader].propose("minority", "forbidden").await,
        Err(ControlRuntimeError::QuorumRequired)
    ));
    let minority_claim = admission_clients[leader]
        .claim_replay_at(
            "community-a",
            "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
            120,
            2_000,
        )
        .await;
    assert!(
        matches!(
            minority_claim,
            Err(AdmissionRuntimeError::Control(
                ControlRuntimeError::QuorumRequired
                    | ControlRuntimeError::LeaderRequired
                    | ControlRuntimeError::ProposalTimeout
            ))
        ),
        "minority claim did not fail closed: {minority_claim:?}"
    );
    let minority_invalidation = admission_clients[leader]
        .publish_invalidation(
            "community-a",
            AuthorizationInvalidationKind::Ban,
            event_id,
            "",
            "minority-ban",
        )
        .await;
    assert!(matches!(
        minority_invalidation,
        Err(AdmissionRuntimeError::Control(
            ControlRuntimeError::QuorumRequired
                | ControlRuntimeError::LeaderRequired
                | ControlRuntimeError::ProposalTimeout
        ))
    ));
    let minority_rate = admission_clients[leader]
        .consume_rate_at("principal", "community-a:alice:api", 60, 2, 70_000)
        .await;
    assert!(matches!(
        minority_rate,
        Err(AdmissionRuntimeError::Control(
            ControlRuntimeError::QuorumRequired
                | ControlRuntimeError::LeaderRequired
                | ControlRuntimeError::ProposalTimeout
        ))
    ));
    assert_eq!(
        stores[leader]
            .recover_control_state()
            .expect("leader control state")
            .metadata
            .state
            .commit_index,
        8
    );
    assert_eq!(
        lease_clients[leader]
            .route("workflow-scheduler")
            .await
            .expect("minority route decision")
            .error,
        LeaseFenceError::QuorumUnavailable
    );
    assert!(matches!(
        lease_clients[leader]
            .grant(
                "workflow-scheduler",
                "minority-lease",
                voters.clone(),
                60_000,
            )
            .await,
        Err(LeaseRuntimeError::PolicyRejected(
            LeaseFenceError::QuorumUnavailable
        ))
    ));

    let resume = stopped[0];
    controls[resume] = Some(
        ControlRuntime::start(
            meshes[resume].client(),
            control_boundaries[resume].client(),
            stores[resume].clone(),
            voters.clone(),
            options,
        )
        .await
        .expect("restart one follower control"),
    );
    admissions[resume] = Some(
        AdmissionRuntime::start(
            controls[resume].as_ref().expect("resumed control").client(),
            boundaries[resume].client(),
            stores[resume].clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("restart one follower admission"),
    );
    leases[resume] = Some(
        LeaseRuntime::start(
            controls[resume].as_ref().expect("resumed control").client(),
            boundaries[resume].client(),
            stores[resume].clone(),
            Duration::from_secs(2),
        )
        .await
        .expect("restart one follower lease"),
    );
    let majority_clients = vec![
        controls[leader].as_ref().expect("leader").client(),
        controls[resume]
            .as_ref()
            .expect("resumed follower")
            .client(),
    ];
    let majority_leader = wait_for_leader(&majority_clients).await;
    let majority_leases = vec![
        leases[leader].as_ref().expect("leader lease").client(),
        leases[resume].as_ref().expect("resumed lease").client(),
    ];
    let renewed = majority_leases[majority_leader]
        .grant("workflow-scheduler", "lease-2", voters.clone(), 60_000)
        .await
        .expect("restored quorum commits a fresh lease");
    assert_eq!(renewed.fence_token, 2);
    wait_for_active_lease(&majority_leases, 2).await;

    let lagged = stopped[1];
    controls[lagged] = Some(
        ControlRuntime::start(
            meshes[lagged].client(),
            control_boundaries[lagged].client(),
            stores[lagged].clone(),
            voters,
            options,
        )
        .await
        .expect("restart lagged follower control"),
    );
    admissions[lagged] = Some(
        AdmissionRuntime::start(
            controls[lagged].as_ref().expect("lagged control").client(),
            boundaries[lagged].client(),
            stores[lagged].clone(),
            Duration::from_secs(5),
        )
        .await
        .expect("restart lagged follower admission"),
    );
    leases[lagged] = Some(
        LeaseRuntime::start(
            controls[lagged].as_ref().expect("lagged control").client(),
            boundaries[lagged].client(),
            stores[lagged].clone(),
            Duration::from_secs(2),
        )
        .await
        .expect("restart lagged follower lease"),
    );
    wait_for_commit(&stores, 9).await;
    tokio::time::timeout(WAIT, async {
        loop {
            if leases[lagged]
                .as_ref()
                .expect("lagged lease")
                .client()
                .state("workflow-scheduler")
                .await
                .is_some_and(|state| state.last_fence_token == 2 && state.active_lease.is_none())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("catch-up restores fences without reviving an old lease");

    assert!(!admissions[lagged]
        .as_ref()
        .expect("lagged admission")
        .client()
        .claim_replay_at("community-a", event_id, 120, 2_000)
        .await
        .expect("recovered replay claim remains owned"));
    wait_for_commit(&stores, 10).await;
    let recovered_rate = admissions[lagged]
        .as_ref()
        .expect("lagged admission")
        .client()
        .consume_rate_at("principal", "community-a:alice:api", 60, 2, 60_004)
        .await
        .expect("recovered cluster rate state remains durable");
    assert!(!recovered_rate.allowed);
    assert_eq!(recovered_rate.current, 4);
    wait_for_commit(&stores, 11).await;

    assert!(admissions[lagged]
        .as_ref()
        .expect("lagged admission")
        .client()
        .claim_replay_at("community-a", event_id, 120, 121_002)
        .await
        .expect("expired replay claim can be acquired after recovery"));
    wait_for_commit(&stores, 12).await;

    let mut recovered_receiver = admissions[leader]
        .as_ref()
        .expect("leader admission")
        .client()
        .subscribe_invalidations();
    let recovered_revision = admissions[lagged]
        .as_ref()
        .expect("lagged admission")
        .client()
        .publish_invalidation(
            "community-a",
            AuthorizationInvalidationKind::Ban,
            event_id,
            "",
            "ban-after-rejoin",
        )
        .await
        .expect("rejoined node publishes invalidation");
    assert_eq!(recovered_revision, 13);
    wait_for_commit(&stores, 13).await;
    assert_eq!(
        tokio::time::timeout(WAIT, recovered_receiver.recv())
            .await
            .expect("post-rejoin invalidation reaches leader")
            .expect("leader invalidation update")
            .revision,
        13
    );

    let mut archive_receiver = admissions[leader]
        .as_ref()
        .expect("leader admission")
        .client()
        .subscribe_invalidations();
    let archive_revision = admissions[lagged]
        .as_ref()
        .expect("lagged admission")
        .client()
        .publish_invalidation(
            "community-a",
            AuthorizationInvalidationKind::Community,
            "",
            "",
            "archive-after-rejoin",
        )
        .await
        .expect("rejoined node publishes community archive invalidation");
    assert_eq!(archive_revision, 14);
    wait_for_commit(&stores, 14).await;
    assert_eq!(
        tokio::time::timeout(WAIT, archive_receiver.recv())
            .await
            .expect("community archive invalidation reaches leader")
            .expect("leader archive update")
            .kind,
        AuthorizationInvalidationKind::Community
    );

    let mut visibility_receiver = admissions[lagged]
        .as_ref()
        .expect("lagged admission")
        .client()
        .subscribe_invalidations();
    let visibility_revision = admissions[leader]
        .as_ref()
        .expect("leader admission")
        .client()
        .publish_invalidation(
            "community-a",
            AuthorizationInvalidationKind::Visibility,
            "",
            "channel-a",
            "visibility-after-rejoin",
        )
        .await
        .expect("leader publishes channel visibility invalidation");
    assert_eq!(visibility_revision, 15);
    wait_for_commit(&stores, 15).await;
    assert_eq!(
        tokio::time::timeout(WAIT, visibility_receiver.recv())
            .await
            .expect("channel visibility invalidation reaches lagged node")
            .expect("lagged visibility update")
            .kind,
        AuthorizationInvalidationKind::Visibility
    );

    let rolled_rate = admissions[lagged]
        .as_ref()
        .expect("lagged admission")
        .client()
        .consume_rate_at("principal", "community-a:alice:api", 60, 2, 120_001)
        .await
        .expect("next cluster rate window starts after recovery");
    assert!(rolled_rate.allowed);
    assert_eq!(rolled_rate.current, 1);
    wait_for_commit(&stores, 16).await;

    for admission in admissions.into_iter().flatten() {
        admission.stop().await.expect("stop admission runtime");
    }
    for lease in leases.into_iter().flatten() {
        lease.stop().await.expect("stop lease runtime");
    }
    for control in controls.into_iter().flatten() {
        control.stop().await.expect("stop control runtime");
    }
    for mesh in meshes {
        mesh.stop().await.expect("stop mesh runtime");
    }
    for boundary in boundaries {
        boundary.shutdown().await.expect("stop Nim boundary");
    }
    for boundary in control_boundaries {
        boundary
            .shutdown()
            .await
            .expect("stop control Nim boundary");
    }
}
