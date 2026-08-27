use std::{fs, path::PathBuf, process::Command};

use nimino_store::{
    CanonicalCommit, ControlLogEntry, ControlLogStorePort, ControlMetadata, NodeStorePort,
    RecordClass, RecordWrite, RedbNodeStore, StoreError, StoredRecord, VersionedControlMetadata,
    SCHEMA_VERSION,
};
use redb::{Database, TableDefinition, TableHandle};
use serde_json::json;
use uuid::Uuid;

const META: TableDefinition<&str, u64> = TableDefinition::new("nimino_meta_v1");
const CANONICAL: TableDefinition<&[u8], &[u8]> = TableDefinition::new("nimino_canonical_v1");
const CONTROL_METADATA: TableDefinition<&str, &[u8]> =
    TableDefinition::new("nimino_control_metadata_v1");
const CONTROL_LOG: TableDefinition<u64, &[u8]> = TableDefinition::new("nimino_control_log_v1");

struct TestPath(PathBuf);

impl TestPath {
    fn new(name: &str) -> Self {
        Self(std::env::temp_dir().join(format!("nimino-store-{name}-{}.redb", Uuid::new_v4())))
    }
}

impl Drop for TestPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn committed_store(path: &TestPath) {
    let store = RedbNodeStore::open(&path.0).unwrap();
    store
        .commit_canonical(CanonicalCommit {
            intent_id: "stable".into(),
            community_id: "community-a".into(),
            expected_checkpoint: 0,
            writes: vec![RecordWrite {
                record_type: "event".into(),
                key: "stable".into(),
                deleted: false,
                value: json!({"state": "committed"}),
            }],
        })
        .unwrap();
}

#[test]
fn crash_writer_helper() {
    let Ok(path) = std::env::var("NIMINO_CRASH_STORE") else {
        return;
    };
    let database = Database::open(path).unwrap();
    let mut transaction = database.begin_write().unwrap();
    transaction.set_quick_repair(true);
    let record = serde_json::to_vec(&StoredRecord {
        sequence: 2,
        record_type: "event".into(),
        key: "uncommitted".into(),
        deleted: false,
        value: json!({"state": "must disappear"}),
    })
    .unwrap();
    transaction
        .open_table(CANONICAL)
        .unwrap()
        .insert(
            b"community-a\0event\0uncommitted".as_slice(),
            record.as_slice(),
        )
        .unwrap();

    // `exit` deliberately skips Rust destructors, modelling an abrupt process loss.
    std::process::exit(86);
}

#[test]
fn recovers_the_last_commit_after_an_abrupt_process_exit() {
    let path = TestPath::new("crash");
    committed_store(&path);

    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "crash_writer_helper", "--nocapture"])
        .env("NIMINO_CRASH_STORE", &path.0)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(86));

    let recovered = RedbNodeStore::open(&path.0).unwrap();
    assert_eq!(recovered.canonical_checkpoint("community-a").unwrap(), 1);
    assert!(recovered
        .get(
            RecordClass::Canonical,
            "community-a",
            "event",
            "uncommitted"
        )
        .unwrap()
        .is_none());
    assert!(recovered
        .get(RecordClass::Canonical, "community-a", "event", "stable")
        .unwrap()
        .is_some());
}

#[test]
fn torn_control_writer_helper() {
    let Ok(path) = std::env::var("NIMINO_CONTROL_CRASH_STORE") else {
        return;
    };
    let database = Database::open(path).unwrap();
    let mut transaction = database.begin_write().unwrap();
    transaction.set_quick_repair(true);
    let entry = serde_json::to_vec(&ControlLogEntry {
        index: 2,
        term: 2,
        voter_epoch: 1,
        kind: "command".into(),
        payload: b"uncommitted".to_vec(),
    })
    .unwrap();
    transaction
        .open_table(CONTROL_LOG)
        .unwrap()
        .insert(2, entry.as_slice())
        .unwrap();
    let metadata = serde_json::to_vec(&VersionedControlMetadata {
        revision: 2,
        state: ControlMetadata {
            term: 2,
            voted_for: Some("node-b".into()),
            commit_index: 2,
            applied_index: 2,
        },
    })
    .unwrap();
    transaction
        .open_table(CONTROL_METADATA)
        .unwrap()
        .insert("state", metadata.as_slice())
        .unwrap();
    std::process::exit(87);
}

#[test]
fn rejects_a_torn_control_log_and_metadata_transaction() {
    let path = TestPath::new("control-crash");
    {
        let store = RedbNodeStore::open(&path.0).unwrap();
        store
            .replace_control_suffix(
                0,
                vec![ControlLogEntry {
                    index: 1,
                    term: 1,
                    voter_epoch: 1,
                    kind: "command".into(),
                    payload: b"committed".to_vec(),
                }],
            )
            .unwrap();
        store
            .compare_and_set_control_metadata(
                0,
                ControlMetadata {
                    term: 1,
                    voted_for: Some("node-a".into()),
                    commit_index: 1,
                    applied_index: 1,
                },
            )
            .unwrap();
    }

    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "torn_control_writer_helper", "--nocapture"])
        .env("NIMINO_CONTROL_CRASH_STORE", &path.0)
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(87));

    let recovered = RedbNodeStore::open(&path.0)
        .unwrap()
        .recover_control_state()
        .unwrap();
    assert_eq!(recovered.metadata.revision, 1);
    assert_eq!(recovered.metadata.state.commit_index, 1);
    assert_eq!(recovered.entries.len(), 1);
    assert_eq!(recovered.entries[0].payload, b"committed");
}

#[test]
fn rejects_unknown_schema_and_bootstraps_separate_tables() {
    let path = TestPath::new("schema");
    {
        let store = RedbNodeStore::open(&path.0).unwrap();
        drop(store);
        let database = Database::open(&path.0).unwrap();
        let transaction = database.begin_write().unwrap();
        let names: Vec<_> = transaction
            .list_tables()
            .unwrap()
            .map(|table| table.name().to_owned())
            .collect();
        assert!(names.contains(&"nimino_canonical_v1".into()));
        assert!(names.contains(&"nimino_cache_v1".into()));
        assert!(names.contains(&"nimino_log_v1".into()));
        assert!(names.contains(&"nimino_control_metadata_v1".into()));
        assert!(names.contains(&"nimino_control_log_v1".into()));
        assert!(names.contains(&"nimino_control_snapshot_v1".into()));
        transaction
            .open_table(META)
            .unwrap()
            .insert("schema_version", SCHEMA_VERSION + 1)
            .unwrap();
        transaction.commit().unwrap();
    }

    assert!(matches!(
        RedbNodeStore::open(&path.0),
        Err(StoreError::UnsupportedSchema {
            found: 2,
            supported: 1
        })
    ));
}

#[test]
fn backup_and_restore_refuse_to_overwrite_existing_files() {
    let source = TestPath::new("backup-source");
    let target = TestPath::new("backup-target");
    committed_store(&source);
    fs::write(&target.0, b"operator-owned").unwrap();

    let store = RedbNodeStore::open(&source.0).unwrap();
    assert!(matches!(
        store.backup_to(&target.0),
        Err(StoreError::TargetExists)
    ));
    assert_eq!(fs::read(&target.0).unwrap(), b"operator-owned");
    assert!(matches!(
        RedbNodeStore::restore_backup(&source.0, &target.0),
        Err(StoreError::TargetExists)
    ));
}
