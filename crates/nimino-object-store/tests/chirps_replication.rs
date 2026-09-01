use std::{
    fs,
    net::{SocketAddr, UdpSocket},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use nimino_boundary::{
    BoundaryConfig, BoundaryRuntime, ObjectDescriptor, ObjectFetchMode, ObjectGcRequest,
    ObjectKind, ObjectLocalFact, ObjectManifest, ObjectOriginFact, ObjectPinRequest,
    ObjectPinState, ObjectPolicyError, ObjectSyncRequest,
};
use nimino_chirps::{MeshClient, MeshRuntime, MeshRuntimeOptions, NodeConfig, NodeId};
use nimino_object_store::{
    LocalObjectStore, ObjectStoreError, ObjectSyncError, ObjectSyncRuntime, MAX_CHUNK_BYTES,
};
use rcgen::generate_simple_self_signed;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

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

    fn object_store(&self, index: usize) -> Arc<LocalObjectStore> {
        Arc::new(
            LocalObjectStore::open(self.root.path().join(format!("objects-{index}")))
                .expect("open object store"),
        )
    }
}

fn free_addr() -> SocketAddr {
    UdpSocket::bind("127.0.0.1:0")
        .expect("reserve UDP port")
        .local_addr()
        .expect("read UDP port")
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn install(store: &LocalObjectStore, transfer: &str, bytes: &[u8]) -> String {
    let expected = digest(bytes);
    let mut offset = 0;
    for chunk in bytes.chunks(MAX_CHUNK_BYTES) {
        offset = store
            .append_partial(transfer, &expected, bytes.len() as u64, offset, chunk)
            .expect("append source object")
            .offset;
    }
    store
        .finish_partial(transfer, &expected, bytes.len() as u64)
        .expect("install source object");
    expected
}

fn request(
    digest: &str,
    size: u64,
    source: NodeId,
    mode: ObjectFetchMode,
    pinned_digests: Vec<String>,
    local_facts: Vec<ObjectLocalFact>,
) -> ObjectSyncRequest {
    ObjectSyncRequest {
        community_id: "community-a".to_owned(),
        manifest: ObjectManifest {
            community_id: "community-a".to_owned(),
            manifest_id: "f".repeat(64),
            generation: 1,
            objects: vec![ObjectDescriptor {
                digest: digest.to_owned(),
                size,
                kind: ObjectKind::Media,
            }],
        },
        manifest_digest_verified: true,
        lifecycle_allows_sync: true,
        cancelled: false,
        mode,
        requested_digest: String::new(),
        pinned_digests,
        local_facts,
        origins: vec![ObjectOriginFact {
            node_id: hex::encode(source.as_bytes()),
            available: true,
            digests: vec![digest.to_owned()],
        }],
        max_fetches: 8,
    }
}

async fn wait_for_connection(client: &MeshClient, peer: NodeId) {
    for _ in 0..240 {
        if client.send(peer, b"object-probe".to_vec()).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("peer did not become reachable");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires the production Nim worker; run `just nimino-object-scenarios`"]
async fn resumes_and_verifies_objects_over_real_chirps_with_nim_policy() {
    let worker = std::env::var_os("NIMINO_BOUNDARY_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_WORKER must point to the production worker");
    let boundary = BoundaryRuntime::start(BoundaryConfig::new(worker))
        .await
        .expect("start Nim boundary");
    let material = ClusterMaterial::new();
    let addresses = [free_addr(), free_addr()];
    let source_mesh = MeshRuntime::start(
        material.config(0, addresses[0], Vec::new()),
        MeshRuntimeOptions::default(),
    )
    .await
    .expect("start source mesh");
    let target_mesh = MeshRuntime::start(
        material.config(1, addresses[1], vec![addresses[0]]),
        MeshRuntimeOptions::default(),
    )
    .await
    .expect("start target mesh");
    wait_for_connection(&target_mesh.client(), source_mesh.local_node_id()).await;
    wait_for_connection(&source_mesh.client(), target_mesh.local_node_id()).await;

    let source_store = material.object_store(0);
    let target_store = material.object_store(1);
    let source = ObjectSyncRuntime::start(
        source_mesh.client(),
        boundary.client(),
        source_store.clone(),
        Duration::from_secs(5),
    )
    .expect("start source object runtime");
    let target = ObjectSyncRuntime::start(
        target_mesh.client(),
        boundary.client(),
        target_store.clone(),
        Duration::from_secs(5),
    )
    .expect("start target object runtime");

    let bytes = (0..(MAX_CHUNK_BYTES * 2 + 73))
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let expected = install(source_store.as_ref(), "source-large", &bytes);
    let resume = MAX_CHUNK_BYTES as u64 + 31;
    for chunk in bytes[..resume as usize].chunks(MAX_CHUNK_BYTES) {
        let offset = target_store
            .begin_partial(&expected, &expected, bytes.len() as u64)
            .expect("partial state")
            .offset;
        target_store
            .append_partial(&expected, &expected, bytes.len() as u64, offset, chunk)
            .expect("seed resumable target");
    }
    let local = ObjectLocalFact {
        digest: expected.clone(),
        size: bytes.len() as u64,
        present: false,
        verified: false,
        partial: true,
        partial_offset: resume,
        unreferenced_since_epoch: None,
    };
    target.shutdown().await.expect("stop target before rejoin");
    let target = ObjectSyncRuntime::start(
        target_mesh.client(),
        boundary.client(),
        target_store.clone(),
        Duration::from_secs(5),
    )
    .expect("rejoin target object runtime");
    let installed = target
        .client()
        .sync(request(
            &expected,
            bytes.len() as u64,
            source_mesh.local_node_id(),
            ObjectFetchMode::Eager,
            Vec::new(),
            vec![local],
        ))
        .await
        .expect("resume and install over Chirps");
    assert_eq!(installed.len(), 1);
    assert_eq!(
        target_store.read(&expected, bytes.len() as u64).unwrap(),
        bytes
    );

    let missing = ObjectSyncRequest {
        origins: Vec::new(),
        ..request(
            &"1".repeat(64),
            1,
            source_mesh.local_node_id(),
            ObjectFetchMode::Eager,
            Vec::new(),
            Vec::new(),
        )
    };
    assert!(matches!(
        target.client().sync(missing).await,
        Err(ObjectSyncError::Policy(ObjectPolicyError::MissingOrigin))
    ));

    let pinned_bytes = b"pinned lazy object";
    let pinned_digest = install(source_store.as_ref(), "source-pinned", pinned_bytes);
    let pin = target
        .client()
        .decide_pin(
            ObjectPinState {
                valid: true,
                community_id: "community-a".to_owned(),
                revision: 0,
                digests: Vec::new(),
            },
            ObjectPinRequest {
                community_id: "community-a".to_owned(),
                expected_revision: 0,
                digest: pinned_digest.clone(),
                pin: true,
            },
        )
        .await
        .expect("Nim pin decision");
    target
        .client()
        .sync(request(
            &pinned_digest,
            pinned_bytes.len() as u64,
            source_mesh.local_node_id(),
            ObjectFetchMode::Lazy,
            pin.state.digests.clone(),
            Vec::new(),
        ))
        .await
        .expect("pin forces lazy materialization");

    let garbage = b"unreferenced object";
    let garbage_digest = install(target_store.as_ref(), "target-garbage", garbage);
    let deleted = target
        .client()
        .gc(ObjectGcRequest {
            community_id: "all-communities".to_owned(),
            current_epoch: 10,
            grace_epochs: 5,
            referenced_digests: Vec::new(),
            pinned_digests: pin.state.digests,
            objects: vec![
                ObjectLocalFact {
                    digest: pinned_digest.clone(),
                    size: pinned_bytes.len() as u64,
                    present: true,
                    verified: true,
                    partial: false,
                    partial_offset: 0,
                    unreferenced_since_epoch: Some(1),
                },
                ObjectLocalFact {
                    digest: garbage_digest.clone(),
                    size: garbage.len() as u64,
                    present: true,
                    verified: true,
                    partial: false,
                    partial_offset: 0,
                    unreferenced_since_epoch: Some(1),
                },
            ],
            max_deletes: 8,
        })
        .await
        .expect("Nim-planned GC");
    assert_eq!(deleted, vec![garbage_digest.clone()]);
    target_store
        .verify(&pinned_digest, pinned_bytes.len() as u64)
        .expect("pin survives GC");
    assert!(matches!(
        target_store.verify(&garbage_digest, garbage.len() as u64),
        Err(ObjectStoreError::NotFound)
    ));

    let corrupt = b"source content";
    let corrupt_digest = install(source_store.as_ref(), "source-corrupt", corrupt);
    fs::write(
        material
            .root
            .path()
            .join("objects-0/objects")
            .join(&corrupt_digest[..2])
            .join(&corrupt_digest),
        b"tampered bytes",
    )
    .expect("corrupt source bytes");
    assert!(matches!(
        target
            .client()
            .sync(request(
                &corrupt_digest,
                corrupt.len() as u64,
                source_mesh.local_node_id(),
                ObjectFetchMode::Eager,
                Vec::new(),
                Vec::new(),
            ))
            .await,
        Err(ObjectSyncError::Store(
            ObjectStoreError::DigestMismatch { .. }
        ))
    ));
    assert!(matches!(
        target_store.verify(&corrupt_digest, corrupt.len() as u64),
        Err(ObjectStoreError::NotFound)
    ));

    target.shutdown().await.expect("stop target object runtime");
    source.shutdown().await.expect("stop source object runtime");
    target_mesh.stop().await.expect("stop target mesh");
    source_mesh.stop().await.expect("stop source mesh");
    boundary.shutdown().await.expect("stop boundary");
}
