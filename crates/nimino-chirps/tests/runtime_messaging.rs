use std::fs;
use std::net::{SocketAddr, UdpSocket};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nimino_chirps::{MeshRuntime, MeshRuntimeError, MeshRuntimeOptions, NodeConfig, NodeId};
use rcgen::generate_simple_self_signed;
use tempfile::TempDir;

struct ClusterMaterial {
    root: TempDir,
    certificate: PathBuf,
    private_key: PathBuf,
}

impl ClusterMaterial {
    fn new() -> Self {
        let root = TempDir::new().expect("cluster tempdir");
        let certificate_path = root.path().join("cluster.crt");
        let private_key_path = root.path().join("cluster.key");
        Self::write_generation(&certificate_path, &private_key_path, "generation-1");
        Self {
            root,
            certificate: certificate_path,
            private_key: private_key_path,
        }
    }

    fn rotate(&self) {
        Self::write_generation(&self.certificate, &self.private_key, "generation-2");
    }

    fn write_generation(certificate_path: &Path, private_key_path: &Path, generation: &str) {
        let certificate =
            generate_simple_self_signed(["alopex.local".to_owned(), generation.to_owned()])
                .expect("cluster certificate");
        fs::write(
            certificate_path,
            certificate.serialize_der().expect("certificate DER"),
        )
        .expect("write certificate");
        fs::write(private_key_path, certificate.serialize_private_key_der())
            .expect("write private key");
        #[cfg(unix)]
        fs::set_permissions(private_key_path, fs::Permissions::from_mode(0o600))
            .expect("secure private key");
    }

    fn config(&self, name: &str, bind_addr: SocketAddr, seeds: Vec<SocketAddr>) -> NodeConfig {
        NodeConfig::new(
            bind_addr,
            self.root.path().join(format!("{name}.identity")),
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

async fn wait_for_peer(client: &nimino_chirps::MeshClient, peer: NodeId, direction: &str) {
    for _ in 0..200 {
        if client.peers().await.expect("peer view").contains(&peer) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "{direction}: peer {peer:?} did not become reachable; final view: {:?}",
        client.peers().await.expect("final peer view")
    );
}

async fn wait_for_connection(client: &nimino_chirps::MeshClient, peer: NodeId) {
    for _ in 0..100 {
        if client
            .send(peer, b"connection probe".to_vec())
            .await
            .is_ok()
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("peer {peer:?} did not accept a connection probe");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn three_nodes_send_broadcast_and_report_slow_subscriber() {
    let material = ClusterMaterial::new();
    let first_addr = free_addr();
    let first = MeshRuntime::start(
        material.config("first", first_addr, vec![]),
        MeshRuntimeOptions::new(8, 8),
    )
    .await
    .expect("start first node");
    let second = MeshRuntime::start(
        material.config("second", free_addr(), vec![first_addr]),
        MeshRuntimeOptions::new(8, 8),
    )
    .await
    .expect("start second node");
    let third = MeshRuntime::start(
        material.config("third", free_addr(), vec![first_addr]),
        MeshRuntimeOptions::new(8, 1),
    )
    .await
    .expect("start third node");

    let first_client = first.client();
    let second_client = second.client();
    let third_client = third.client();
    wait_for_connection(&second_client, first.local_node_id()).await;
    wait_for_connection(&third_client, first.local_node_id()).await;
    wait_for_peer(&second_client, first.local_node_id(), "second -> first").await;
    wait_for_peer(&third_client, first.local_node_id(), "third -> first").await;
    wait_for_peer(&first_client, third.local_node_id(), "first -> third").await;
    let mut first_messages = first_client.subscribe();
    let mut second_messages = second_client.subscribe();
    let mut third_messages = third_client.subscribe();

    second_client
        .send(first.local_node_id(), b"direct".to_vec())
        .await
        .expect("direct send");
    let direct = tokio::time::timeout(Duration::from_secs(2), first_messages.recv())
        .await
        .expect("direct timeout")
        .expect("direct receive");
    assert_eq!(direct.from(), second.local_node_id());
    assert_eq!(direct.payload(), b"direct");

    assert_eq!(
        first_client
            .broadcast(b"broadcast".to_vec())
            .await
            .expect("broadcast"),
        2
    );
    for receiver in [&mut second_messages, &mut third_messages] {
        let message = tokio::time::timeout(Duration::from_secs(2), receiver.recv())
            .await
            .expect("broadcast timeout")
            .expect("broadcast receive");
        assert_eq!(message.payload(), b"broadcast");
    }

    let mut slow = third_client.subscribe();
    for sequence in 0..3 {
        first_client
            .send(third.local_node_id(), vec![sequence])
            .await
            .expect("slow-subscriber send");
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(matches!(
        slow.recv().await,
        Err(MeshRuntimeError::SubscriberLagged { skipped }) if skipped >= 2
    ));

    third.stop().await.expect("stop third");
    second.stop().await.expect("stop second");
    first.stop().await.expect("stop first");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stop_releases_socket_and_restart_preserves_identity() {
    let material = ClusterMaterial::new();
    let bind_addr = free_addr();
    let config = material.config("restart", bind_addr, vec![]);
    let first = MeshRuntime::start(config.clone(), MeshRuntimeOptions::default())
        .await
        .expect("first start");
    let node_id = first.local_node_id();
    let incarnation = first.local_incarnation();
    let client = first.client();
    let mut subscription = client.subscribe();
    assert_eq!(
        client
            .broadcast(b"single node".to_vec())
            .await
            .expect("single-node broadcast"),
        0
    );
    first.stop().await.expect("first stop");
    assert!(matches!(
        client.peers().await,
        Err(MeshRuntimeError::Stopped)
    ));
    assert!(matches!(
        subscription.recv().await,
        Err(MeshRuntimeError::Stopped)
    ));
    let probe = UdpSocket::bind(bind_addr).expect("socket released after stop");
    drop(probe);

    let restarted = MeshRuntime::start(config, MeshRuntimeOptions::default())
        .await
        .expect("restart");
    assert_eq!(restarted.local_node_id(), node_id);
    assert_eq!(restarted.local_incarnation(), incarnation + 1);
    restarted.stop().await.expect("second stop");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn certificate_rotation_and_bind_change_preserve_identity_and_rejoin() {
    let material = ClusterMaterial::new();
    let first_reservation = UdpSocket::bind("127.0.0.1:0").expect("reserve anchor socket");
    let first_addr = first_reservation.local_addr().expect("anchor address");
    let second_reservation = UdpSocket::bind("127.0.0.1:0").expect("reserve peer socket");
    let second_addr = second_reservation.local_addr().expect("peer address");
    drop(first_reservation);
    let first = MeshRuntime::start(
        material.config("rebind-first", first_addr, vec![]),
        MeshRuntimeOptions::default(),
    )
    .await
    .expect("start first generation anchor");
    drop(second_reservation);
    let second = MeshRuntime::start(
        material.config("rebind-second", second_addr, vec![first_addr]),
        MeshRuntimeOptions::default(),
    )
    .await
    .expect("start first generation peer");
    wait_for_connection(&second.client(), first.local_node_id()).await;
    let identities = [first.local_node_id(), second.local_node_id()];
    let incarnations = [first.local_incarnation(), second.local_incarnation()];
    second.stop().await.expect("stop first generation peer");
    first.stop().await.expect("stop first generation anchor");

    let old_first = UdpSocket::bind(first_addr).expect("old anchor socket released");
    let old_second = UdpSocket::bind(second_addr).expect("old peer socket released");
    let first_reservation = UdpSocket::bind("127.0.0.1:0").expect("reserve new anchor socket");
    let rotated_first_addr = first_reservation.local_addr().expect("new anchor address");
    let second_reservation = UdpSocket::bind("127.0.0.1:0").expect("reserve new peer socket");
    let rotated_second_addr = second_reservation.local_addr().expect("new peer address");
    assert_ne!(rotated_first_addr, first_addr);
    assert_ne!(rotated_second_addr, second_addr);
    material.rotate();

    drop(first_reservation);
    let restarted_first = MeshRuntime::start(
        material.config("rebind-first", rotated_first_addr, vec![]),
        MeshRuntimeOptions::default(),
    )
    .await
    .expect("start rotated anchor");
    drop(second_reservation);
    let restarted_second = MeshRuntime::start(
        material.config(
            "rebind-second",
            rotated_second_addr,
            vec![rotated_first_addr],
        ),
        MeshRuntimeOptions::default(),
    )
    .await
    .expect("start rotated peer");
    assert_eq!(restarted_first.local_node_id(), identities[0]);
    assert_eq!(restarted_second.local_node_id(), identities[1]);
    assert_eq!(restarted_first.local_incarnation(), incarnations[0] + 1);
    assert_eq!(restarted_second.local_incarnation(), incarnations[1] + 1);
    wait_for_connection(&restarted_second.client(), identities[0]).await;

    let mut messages = restarted_first.client().subscribe();
    restarted_second
        .client()
        .send(identities[0], b"rejoined after network change".to_vec())
        .await
        .expect("send after rejoin");
    let message = tokio::time::timeout(Duration::from_secs(2), messages.recv())
        .await
        .expect("rejoin receive timeout")
        .expect("rejoin receive");
    assert_eq!(message.from(), identities[1]);
    assert_eq!(message.payload(), b"rejoined after network change");

    restarted_second.stop().await.expect("stop rotated peer");
    restarted_first.stop().await.expect("stop rotated anchor");
    UdpSocket::bind(rotated_first_addr).expect("rotated anchor socket released");
    UdpSocket::bind(rotated_second_addr).expect("rotated peer socket released");
    drop((old_first, old_second));
}
