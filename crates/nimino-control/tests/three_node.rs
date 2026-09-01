use std::{
    fs,
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use nimino_boundary::{BoundaryConfig, BoundaryRuntime, LeaseFenceError, SingletonEffectAttempt};
use nimino_chirps::{MeshClient, MeshRuntime, MeshRuntimeOptions, NodeConfig, NodeId};
use nimino_control::{
    ControlClient, ControlRuntime, ControlRuntimeError, ControlRuntimeOptions, LeaseClient,
    LeaseRuntime, LeaseRuntimeError,
};
use nimino_store::{ControlLogStorePort, RedbNodeStore};
use rcgen::generate_simple_self_signed;
use tempfile::TempDir;

const WAIT: Duration = Duration::from_secs(15);

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
    .expect("cluster elects one live-quorum leader")
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the production Nim worker; run `just nimino-control-scenarios`"]
async fn three_nodes_elect_commit_fail_closed_and_catch_up_over_real_chirps() {
    let worker = std::env::var_os("NIMINO_BOUNDARY_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_WORKER must point to the production worker");
    let boundary = BoundaryRuntime::start(BoundaryConfig::new(worker))
        .await
        .expect("start Nim boundary");
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
        Duration::from_millis(100),
        Duration::from_secs(1),
        Duration::from_secs(2),
        16,
    );
    let mut controls = Vec::new();
    for index in 0..3 {
        controls.push(Some(
            ControlRuntime::start(
                meshes[index].client(),
                boundary.client(),
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
    let mut leases = Vec::new();
    for index in 0..3 {
        leases.push(Some(
            LeaseRuntime::start(
                clients[index].clone(),
                boundary.client(),
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
    let route = lease_clients[leader]
        .route("workflow-scheduler")
        .await
        .expect("route decision");
    assert!(route.allowed);
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

    let entry = clients[leader]
        .propose("command-1", r#"{"operation":"first"}"#)
        .await
        .expect("quorum commits first command");
    assert_eq!(entry.index, 2);
    wait_for_commit(&stores, 2).await;

    let stopped = (0..3).filter(|index| *index != leader).collect::<Vec<_>>();
    for index in &stopped {
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
            boundary.client(),
            stores[resume].clone(),
            voters.clone(),
            options,
        )
        .await
        .expect("restart one follower control"),
    );
    leases[resume] = Some(
        LeaseRuntime::start(
            controls[resume].as_ref().expect("resumed control").client(),
            boundary.client(),
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
            boundary.client(),
            stores[lagged].clone(),
            voters,
            options,
        )
        .await
        .expect("restart lagged follower control"),
    );
    leases[lagged] = Some(
        LeaseRuntime::start(
            controls[lagged].as_ref().expect("lagged control").client(),
            boundary.client(),
            stores[lagged].clone(),
            Duration::from_secs(2),
        )
        .await
        .expect("restart lagged follower lease"),
    );
    wait_for_commit(&stores, 3).await;
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

    for lease in leases.into_iter().flatten() {
        lease.stop().await.expect("stop lease runtime");
    }
    for control in controls.into_iter().flatten() {
        control.stop().await.expect("stop control runtime");
    }
    for mesh in meshes {
        mesh.stop().await.expect("stop mesh runtime");
    }
    boundary.shutdown().await.expect("stop Nim boundary");
}
