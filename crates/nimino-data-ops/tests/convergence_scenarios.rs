use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use nimino_data_ops::{repair_replica, verify_replica, ObjectRepairRoots, ObjectSpec};
use nimino_object_store::{LocalObjectStore, MAX_CHUNK_BYTES};
use nimino_store::{CacheReplacement, CanonicalCommit, NodeStorePort, RecordWrite, RedbNodeStore};
use serde_json::json;
use sha2::{Digest, Sha256};
use uuid::Uuid;

struct TempTree(PathBuf);

impl TempTree {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("nimino-data-ops-{name}-{}", Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn seed_store(path: &Path, records: usize, content_bytes: usize) {
    let store = RedbNodeStore::open(path).unwrap();
    let content = "x".repeat(content_bytes);
    let mut checkpoint = 0_u64;
    for (batch, start) in (0..records).step_by(1_000).enumerate() {
        let end = records.min(start + 1_000);
        let writes = (start..end)
            .map(|index| RecordWrite {
                record_type: "event".into(),
                key: format!("event-{index:08}"),
                deleted: false,
                value: json!({
                    "content": content,
                    "createdAt": index,
                    "eventId": format!("event-{index:08}"),
                }),
            })
            .collect();
        checkpoint = store
            .commit_canonical(CanonicalCommit {
                intent_id: format!("seed-{batch}"),
                community_id: "community-a".into(),
                expected_checkpoint: checkpoint,
                writes,
            })
            .unwrap()
            .checkpoint;
    }
    for record_type in ["feed_index", "search_index", "thread_index"] {
        let rows = (0..10)
            .map(|index| RecordWrite {
                record_type: record_type.into(),
                key: format!("row-{index:02}"),
                deleted: false,
                value: json!({"sourceCheckpoint": checkpoint, "index": index}),
            })
            .collect();
        store
            .replace_cache(CacheReplacement {
                intent_id: format!("projection-{record_type}"),
                community_id: "community-a".into(),
                source_checkpoint: checkpoint,
                record_type: record_type.into(),
                rows,
            })
            .unwrap();
    }
}

fn install_object(root: &Path, bytes: &[u8]) -> ObjectSpec {
    let digest = hex::encode(Sha256::digest(bytes));
    let size = u64::try_from(bytes.len()).unwrap();
    let store = LocalObjectStore::open(root).unwrap();
    store.begin_partial("seed", &digest, size).unwrap();
    let mut offset = 0_u64;
    for chunk in bytes.chunks(MAX_CHUNK_BYTES) {
        offset = store
            .append_partial("seed", &digest, size, offset, chunk)
            .unwrap()
            .offset;
    }
    store.finish_partial("seed", &digest, size).unwrap();
    ObjectSpec { digest, size }
}

#[test]
fn large_backlog_corruption_and_repeat_repair_converge() {
    let root = TempTree::new("converge");
    let source_store = root.0.join("source.redb");
    let target_store = root.0.join("target.redb");
    let quarantine_store = root.0.join("quarantine/target.redb");
    let source_objects = root.0.join("source-objects");
    let target_objects = root.0.join("target-objects");
    let object_quarantine = root.0.join("object-quarantine");
    seed_store(&source_store, 3_005, 8);
    seed_store(&target_store, 1, 8);

    let bytes = vec![0x5a; MAX_CHUNK_BYTES + 37];
    let object = install_object(&source_objects, &bytes);
    let corrupt_path = target_objects
        .join("objects")
        .join(&object.digest[..2])
        .join(&object.digest);
    fs::create_dir_all(corrupt_path.parent().unwrap()).unwrap();
    fs::write(&corrupt_path, b"corrupt chunk").unwrap();

    let source = verify_replica(
        &source_store,
        "community-a",
        Some(&source_objects),
        std::slice::from_ref(&object),
    )
    .unwrap();
    let cli_output = Command::new(env!("CARGO_BIN_EXE_nimino-data-ops"))
        .arg("verify")
        .arg("--store")
        .arg(&source_store)
        .arg("--community")
        .arg("community-a")
        .arg("--object-root")
        .arg(&source_objects)
        .arg("--object")
        .arg(format!("{}:{}", object.digest, object.size))
        .output()
        .unwrap();
    assert!(cli_output.status.success());
    let cli_inventory: serde_json::Value = serde_json::from_slice(&cli_output.stdout).unwrap();
    assert_eq!(cli_inventory["canonicalDigest"], source.canonical_digest);
    assert_eq!(cli_inventory["projectionDigest"], source.projection_digest);
    assert_eq!(cli_inventory["objectDigest"], source.object_digest);
    let first = repair_replica(
        &source_store,
        &target_store,
        &quarantine_store,
        "community-a",
        ObjectRepairRoots {
            source: Some(&source_objects),
            target: Some(&target_objects),
            quarantine: Some(&object_quarantine),
        },
        std::slice::from_ref(&object),
    )
    .unwrap();
    assert!(first.applied);
    assert_eq!(first.inventory, source);
    assert!(quarantine_store.exists());
    assert!(object_quarantine.join(&object.digest).exists());

    let repeated = repair_replica(
        &source_store,
        &target_store,
        &quarantine_store,
        "community-a",
        ObjectRepairRoots {
            source: Some(&source_objects),
            target: Some(&target_objects),
            quarantine: Some(&object_quarantine),
        },
        &[object],
    )
    .unwrap();
    assert!(!repeated.applied);
    assert_eq!(repeated.inventory, source);
}

#[test]
fn capacity_failure_and_batch_kill_leave_the_old_target_authoritative() {
    let root = TempTree::new("failure");
    let source_store = root.0.join("source.redb");
    let target_store = root.0.join("target.redb");
    let quarantine_store = root.0.join("quarantine/target.redb");
    seed_store(&source_store, 32, 256 * 1_024);
    seed_store(&target_store, 1, 0);
    let before = verify_replica(&target_store, "community-a", None, &[]).unwrap();

    #[cfg(unix)]
    {
        let output = Command::new("bash")
            .arg("-c")
            .arg("ulimit -f 8; exec \"$1\" repair --source-store \"$2\" --target-store \"$3\" --quarantine-store \"$4\" --community community-a")
            .arg("nimino-capacity-test")
            .arg(env!("CARGO_BIN_EXE_nimino-data-ops"))
            .arg(&source_store)
            .arg(&target_store)
            .arg(&quarantine_store)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            verify_replica(&target_store, "community-a", None, &[]).unwrap(),
            before
        );
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_nimino-data-ops"))
        .args([
            "repair",
            "--source-store",
            source_store.to_str().unwrap(),
            "--target-store",
            target_store.to_str().unwrap(),
            "--quarantine-store",
            quarantine_store.to_str().unwrap(),
            "--community",
            "community-a",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let parent = target_store.parent().unwrap();
    let mut killed = false;
    for _ in 0..60_000 {
        let copy_started = fs::read_dir(parent).unwrap().flatten().any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.contains(".nimino-tmp-"))
        });
        if copy_started {
            child.kill().unwrap();
            killed = true;
            break;
        }
        if child.try_wait().unwrap().is_some() {
            panic!("repair completed before the kill point");
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    assert!(
        killed,
        "repair copy did not reach the observable kill point"
    );
    assert!(!child.wait().unwrap().success());
    assert_eq!(
        verify_replica(&target_store, "community-a", None, &[]).unwrap(),
        before
    );

    let repaired = repair_replica(
        &source_store,
        &target_store,
        &quarantine_store,
        "community-a",
        ObjectRepairRoots {
            source: None,
            target: None,
            quarantine: None,
        },
        &[],
    )
    .unwrap();
    assert!(repaired.applied);
    assert_eq!(
        repaired.inventory,
        verify_replica(&source_store, "community-a", None, &[]).unwrap()
    );
}
