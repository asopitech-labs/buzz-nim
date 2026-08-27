use std::fs;
use std::io::Write;
use std::net::{SocketAddr, UdpSocket};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use nimino_boundary::{
    BoundaryConfig, BoundaryRequest, BoundaryResult, BoundaryRuntime, CallContext, ClusterLane,
    ClusterLaneRequest, ClusterLifecycleError, ClusterLifecyclePolicyRequest,
    ClusterLifecyclePolicyResult, ClusterNodeState, LifecycleCommand, LifecycleEffect,
    LifecycleTransitionRequest,
};
use nimino_chirps::{MeshClient, MeshRuntime, MeshRuntimeOptions, NodeConfig, NodeId};
use rcgen::generate_simple_self_signed;
use serde::{Deserialize, Serialize};
use tempfile::TempDir;

const STEP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioContract {
    schema_version: u16,
    contract: String,
    version: u16,
    compatibility_mode: bool,
    failure_seed: u64,
    transport: String,
    lifecycle_contract: String,
    scenarios: Vec<ScenarioSpec>,
    required_stages: Vec<String>,
    resource_proofs: Vec<String>,
    data_convergence_owner: u16,
    cutover_owner: u16,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioSpec {
    node_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ScenarioEvidence {
    node_count: usize,
    failure_node_index: usize,
    partition_observed: bool,
    stable_identity: bool,
    incarnation_before: u64,
    incarnation_after: u64,
    drained_nodes: usize,
    released_udp_sockets: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Evidence {
    schema_version: u16,
    contract: String,
    contract_version: u16,
    failure_seed: u64,
    transport: String,
    nim_worker_reaped: bool,
    scenarios: Vec<ScenarioEvidence>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct KillReady {
    node_id: [u8; 16],
    incarnation: u64,
}

struct ClusterMaterial {
    root: TempDir,
    certificate: PathBuf,
    private_key: PathBuf,
}

impl ClusterMaterial {
    fn new() -> Self {
        let root = TempDir::new().expect("cluster tempdir");
        let certificate =
            generate_simple_self_signed(["alopex.local".to_owned()]).expect("cluster certificate");
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
            .expect("secure private key");
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
}

fn free_addr() -> SocketAddr {
    UdpSocket::bind("127.0.0.1:0")
        .expect("reserve UDP port")
        .local_addr()
        .expect("read UDP port")
}

fn failure_index(seed: u64, node_count: usize) -> usize {
    if node_count == 1 {
        return 0;
    }
    let mixed = seed
        .wrapping_add(node_count as u64)
        .wrapping_mul(0x9e37_79b9_7f4a_7c15);
    1 + (mixed as usize % (node_count - 1))
}

async fn wait_for_connection(client: &MeshClient, peer: NodeId) {
    for _ in 0..120 {
        if client.send(peer, b"scenario probe".to_vec()).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("peer {peer:?} did not become reachable");
}

async fn wait_for_socket(addr: SocketAddr) {
    for _ in 0..120 {
        if UdpSocket::bind(addr).is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("UDP socket was not released: {addr}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "process helper for the real-mesh kill scenario"]
async fn killed_mesh_process_helper() {
    if std::env::var_os("NIMINO_KILL_HELPER").is_none() {
        return;
    }
    let bind_addr = std::env::var("NIMINO_KILL_BIND")
        .expect("kill bind address")
        .parse()
        .expect("valid kill bind address");
    let config = NodeConfig::new(
        bind_addr,
        PathBuf::from(std::env::var_os("NIMINO_KILL_IDENTITY").expect("kill identity path")),
        PathBuf::from(std::env::var_os("NIMINO_KILL_CERT").expect("kill certificate path")),
        PathBuf::from(std::env::var_os("NIMINO_KILL_KEY").expect("kill key path")),
        vec![PathBuf::from(
            std::env::var_os("NIMINO_KILL_TRUST").expect("kill trust path"),
        )],
    );
    let runtime = MeshRuntime::start(config, MeshRuntimeOptions::default())
        .await
        .expect("start kill helper mesh");
    let ready = KillReady {
        node_id: runtime.local_node_id().as_bytes(),
        incarnation: runtime.local_incarnation(),
    };
    let ready_path =
        PathBuf::from(std::env::var_os("NIMINO_KILL_READY").expect("kill helper ready path"));
    let mut file = fs::File::create(ready_path).expect("create kill readiness");
    file.write_all(&serde_json::to_vec(&ready).expect("encode kill readiness"))
        .expect("write kill readiness");
    file.sync_all().expect("sync kill readiness");
    std::future::pending::<()>().await;
}

async fn start_kill_node(
    config: &NodeConfig,
    ready_path: &PathBuf,
) -> (tokio::process::Child, KillReady) {
    let mut command = tokio::process::Command::new(
        std::env::current_exe().expect("locate cluster scenario test binary"),
    );
    command
        .args([
            "--exact",
            "killed_mesh_process_helper",
            "--ignored",
            "--nocapture",
        ])
        .env("NIMINO_KILL_HELPER", "1")
        .env("NIMINO_KILL_BIND", config.bind_addr().to_string())
        .env("NIMINO_KILL_IDENTITY", config.identity_path())
        .env("NIMINO_KILL_CERT", config.certificate_path())
        .env("NIMINO_KILL_KEY", config.private_key_path())
        .env("NIMINO_KILL_TRUST", &config.trust_anchor_paths()[0])
        .env("NIMINO_KILL_READY", ready_path)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    let mut child = command.spawn().expect("spawn kill helper process");
    for _ in 0..120 {
        if let Ok(ready) = fs::read(ready_path).and_then(|bytes| {
            serde_json::from_slice(&bytes)
                .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        }) {
            return (child, ready);
        }
        assert!(
            child.try_wait().expect("inspect kill helper").is_none(),
            "kill helper exited before readiness"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("kill helper did not become ready");
}

fn transition(
    command: LifecycleCommand,
    current_state: ClusterNodeState,
    voter_epoch: u64,
    active_work: u32,
) -> ClusterLifecyclePolicyRequest {
    ClusterLifecyclePolicyRequest::Transition {
        request: LifecycleTransitionRequest {
            command,
            current_state,
            authenticated: true,
            revoked: false,
            identity_unique: true,
            product_capability: "nimino-v1".to_owned(),
            control_protocol_version: 1,
            data_protocol_version: 1,
            control_decision_committed: true,
            snapshot_installed: true,
            checkpoint_matches: true,
            required_voter_epoch: voter_epoch,
            observed_voter_epoch: voter_epoch,
            active_work,
        },
    }
}

async fn policy_call(
    client: &nimino_boundary::BoundaryClient,
    request: ClusterLifecyclePolicyRequest,
) -> BoundaryResult {
    client
        .call(
            BoundaryRequest::cluster_lifecycle(request),
            CallContext::with_timeout(STEP_TIMEOUT),
        )
        .await
        .expect("Nim cluster lifecycle call")
}

async fn verify_lifecycle(client: &nimino_boundary::BoundaryClient, voter_epoch: u64) {
    let mut state = ClusterNodeState::Offline;
    for (command, expected_state, expected_effect) in [
        (
            LifecycleCommand::Join,
            ClusterNodeState::Joining,
            LifecycleEffect::EnterJoining,
        ),
        (
            LifecycleCommand::StartSync,
            ClusterNodeState::Syncing,
            LifecycleEffect::EnterSyncing,
        ),
        (
            LifecycleCommand::MarkReady,
            ClusterNodeState::Ready,
            LifecycleEffect::EnterReady,
        ),
    ] {
        assert_eq!(
            policy_call(client, transition(command, state, voter_epoch, 0)).await,
            BoundaryResult::ClusterLifecycle(ClusterLifecyclePolicyResult::Transition {
                effect: expected_effect,
                next_state: expected_state,
                error: ClusterLifecycleError::None,
            })
        );
        state = expected_state;
    }

    assert_eq!(
        policy_call(
            client,
            transition(LifecycleCommand::BeginDrain, state, voter_epoch, 0),
        )
        .await,
        BoundaryResult::ClusterLifecycle(ClusterLifecyclePolicyResult::Transition {
            effect: LifecycleEffect::EnterDraining,
            next_state: ClusterNodeState::Draining,
            error: ClusterLifecycleError::None,
        })
    );
    assert_eq!(
        policy_call(
            client,
            ClusterLifecyclePolicyRequest::Lane {
                request: ClusterLaneRequest {
                    state: ClusterNodeState::Draining,
                    lane: ClusterLane::ClientWrite,
                },
            },
        )
        .await,
        BoundaryResult::ClusterLifecycle(ClusterLifecyclePolicyResult::Lane {
            effect: LifecycleEffect::DenyLane,
            next_state: ClusterNodeState::Draining,
            error: ClusterLifecycleError::LaneNotAllowed,
        })
    );
    assert_eq!(
        policy_call(
            client,
            transition(
                LifecycleCommand::MarkOffline,
                ClusterNodeState::Draining,
                voter_epoch,
                1,
            ),
        )
        .await,
        BoundaryResult::ClusterLifecycle(ClusterLifecyclePolicyResult::Transition {
            effect: LifecycleEffect::Reject,
            next_state: ClusterNodeState::Draining,
            error: ClusterLifecycleError::DrainIncomplete,
        })
    );
    assert_eq!(
        policy_call(
            client,
            transition(
                LifecycleCommand::MarkOffline,
                ClusterNodeState::Draining,
                voter_epoch,
                0,
            ),
        )
        .await,
        BoundaryResult::ClusterLifecycle(ClusterLifecyclePolicyResult::Transition {
            effect: LifecycleEffect::EnterOffline,
            next_state: ClusterNodeState::Offline,
            error: ClusterLifecycleError::None,
        })
    );
}

async fn run_scenario(
    spec: ScenarioSpec,
    seed: u64,
    policy: &nimino_boundary::BoundaryClient,
) -> ScenarioEvidence {
    let node_count = spec.node_count;
    let victim = failure_index(seed, node_count);
    let primary = 0;
    let material = ClusterMaterial::new();
    let addresses = (0..node_count).map(|_| free_addr()).collect::<Vec<_>>();
    let configs = (0..node_count)
        .map(|index| {
            let seeds = if index == primary || index == victim {
                Vec::new()
            } else {
                vec![addresses[primary]]
            };
            material.config(index, addresses[index], seeds)
        })
        .collect::<Vec<_>>();
    let mut runtimes = std::iter::repeat_with(|| None)
        .take(node_count)
        .collect::<Vec<Option<MeshRuntime>>>();

    if primary != victim {
        runtimes[primary] = Some(
            MeshRuntime::start(configs[primary].clone(), MeshRuntimeOptions::default())
                .await
                .expect("start primary"),
        );
    }
    for index in 1..node_count {
        if index == victim {
            continue;
        }
        runtimes[index] = Some(
            MeshRuntime::start(configs[index].clone(), MeshRuntimeOptions::default())
                .await
                .expect("start peer"),
        );
    }
    let primary_client = runtimes[primary].as_ref().map(MeshRuntime::client);
    if let Some(primary_client) = &primary_client {
        for (index, runtime) in runtimes.iter().enumerate() {
            if index == primary || index == victim {
                continue;
            }
            wait_for_connection(
                &runtime.as_ref().expect("peer").client(),
                primary_client.local_node_id(),
            )
            .await;
        }
    }

    let ready_path = material.root.path().join(format!("kill-{node_count}.json"));
    let (mut victim_process, ready) = start_kill_node(&configs[victim], &ready_path).await;
    let stable_node_id = NodeId::from_bytes(ready.node_id);
    let incarnation_before = ready.incarnation;
    let partition_observed = if let Some(client) = &primary_client {
        client
            .send(stable_node_id, b"partition probe".to_vec())
            .await
            .is_err()
    } else {
        false
    };
    assert_eq!(partition_observed, node_count > 1);

    victim_process
        .kill()
        .await
        .expect("kill failure-node process");
    victim_process
        .wait()
        .await
        .expect("reap failure-node process");
    wait_for_socket(addresses[victim]).await;
    let restart_seeds = if node_count == 1 {
        Vec::new()
    } else {
        vec![addresses[primary]]
    };
    let restarted = MeshRuntime::start(
        material.config(victim, addresses[victim], restart_seeds),
        MeshRuntimeOptions::default(),
    )
    .await
    .expect("restart failure node");
    let incarnation_after = restarted.local_incarnation();
    assert_eq!(restarted.local_node_id(), stable_node_id);
    assert_eq!(incarnation_after, incarnation_before + 1);
    if let Some(primary_client) = &primary_client {
        wait_for_connection(&restarted.client(), primary_client.local_node_id()).await;
        let mut messages = primary_client.subscribe();
        restarted
            .client()
            .send(primary_client.local_node_id(), b"rejoined".to_vec())
            .await
            .expect("rejoin message");
        let received = tokio::time::timeout(STEP_TIMEOUT, messages.recv())
            .await
            .expect("rejoin receive timeout")
            .expect("rejoin receive");
        assert_eq!(received.from(), stable_node_id);
        assert_eq!(received.payload(), b"rejoined");
    }
    runtimes[victim] = Some(restarted);

    for runtime in runtimes.iter().flatten() {
        verify_lifecycle(policy, runtime.local_incarnation()).await;
    }
    for runtime in runtimes.into_iter().flatten() {
        runtime.stop().await.expect("clean mesh shutdown");
    }
    for address in &addresses {
        UdpSocket::bind(address).expect("all mesh sockets released");
    }

    ScenarioEvidence {
        node_count,
        failure_node_index: victim,
        partition_observed,
        stable_identity: true,
        incarnation_before,
        incarnation_after,
        drained_nodes: node_count,
        released_udp_sockets: node_count,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the Nim worker; run `just nimino-cluster-scenarios`"]
async fn one_three_and_five_node_contract_runs_on_real_chirps_mesh() {
    let contract: ScenarioContract = serde_json::from_str(include_str!(
        "../../../contracts/nimino-cluster-scenarios/v1/contract.json"
    ))
    .expect("valid cluster scenario contract");
    assert_eq!(contract.schema_version, 1);
    assert_eq!(contract.contract, "nimino.cluster-scenarios");
    assert_eq!(contract.version, 1);
    assert!(!contract.compatibility_mode);
    assert_eq!(contract.transport, "alopex-chirps/0.6.3-udp-quic-mtls");
    assert_eq!(contract.lifecycle_contract, "nimino.cluster-lifecycle/v1");
    assert_eq!(contract.required_stages.len(), 6);
    assert_eq!(contract.resource_proofs.len(), 2);
    assert_eq!(contract.data_convergence_owner, 59);
    assert_eq!(contract.cutover_owner, 12);
    assert_eq!(
        contract
            .scenarios
            .iter()
            .map(|scenario| scenario.node_count)
            .collect::<Vec<_>>(),
        vec![1, 3, 5]
    );

    let worker = std::env::var_os("NIMINO_BOUNDARY_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_WORKER must point to the production Nim worker");
    let boundary = BoundaryRuntime::start(BoundaryConfig::new(worker))
        .await
        .expect("start Nim worker boundary");
    let policy = boundary.client();
    let mut scenarios = Vec::new();
    for scenario in &contract.scenarios {
        scenarios.push(run_scenario(*scenario, contract.failure_seed, &policy).await);
    }
    boundary.shutdown().await.expect("Nim worker reaped");

    let evidence = Evidence {
        schema_version: 1,
        contract: contract.contract,
        contract_version: contract.version,
        failure_seed: contract.failure_seed,
        transport: contract.transport,
        nim_worker_reaped: true,
        scenarios,
    };
    let output = std::env::var_os("NIMINO_CLUSTER_EVIDENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/nim/nimino-cluster-scenarios.json"));
    if let Some(parent) = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).expect("create evidence directory");
    }
    fs::write(
        output,
        serde_json::to_vec_pretty(&evidence).expect("encode scenario evidence"),
    )
    .expect("write scenario evidence");
}
