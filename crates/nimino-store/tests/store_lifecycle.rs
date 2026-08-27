use std::{fs, path::PathBuf};

use nimino_store::{
    CacheReplacement, CanonicalCommit, LogAppend, NodeStorePort, RecordClass, RecordWrite,
    RedbNodeStore, StoreError, StoredRecord,
};
use serde_json::json;
use uuid::Uuid;

struct TestPath(PathBuf);

impl TestPath {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("nimino-store-{name}-{}.redb", Uuid::new_v4()));
        Self(path)
    }
}

impl Drop for TestPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn write(record_type: &str, key: &str, value: serde_json::Value) -> RecordWrite {
    RecordWrite {
        record_type: record_type.into(),
        key: key.into(),
        deleted: false,
        value,
    }
}

#[test]
fn separates_classes_and_commits_canonical_writes_atomically() {
    let path = TestPath::new("classes");
    let store = RedbNodeStore::open(&path.0).unwrap();

    let result = store
        .commit_canonical(CanonicalCommit {
            intent_id: "commit-1".into(),
            community_id: "community-a".into(),
            expected_checkpoint: 0,
            writes: vec![
                write("event", "event-1", json!({"content": "one"})),
                write("channel", "channel-1", json!({"name": "general"})),
            ],
        })
        .unwrap();
    assert_eq!(result.checkpoint, 2);
    assert!(result.applied);

    store
        .replace_cache(CacheReplacement {
            intent_id: "cache-1".into(),
            community_id: "community-a".into(),
            source_checkpoint: 2,
            record_type: "thread_index".into(),
            rows: vec![write("thread_index", "event-1", json!({"replies": 0}))],
        })
        .unwrap();
    store
        .append_log(LogAppend {
            intent_id: "log-1".into(),
            community_id: "community-a".into(),
            entries: vec![write("audit_entry", "audit-1", json!({"action": "write"}))],
        })
        .unwrap();

    assert_eq!(
        store
            .get(RecordClass::Canonical, "community-a", "event", "event-1")
            .unwrap()
            .unwrap()
            .value,
        json!({"content": "one"})
    );
    assert!(store
        .get(RecordClass::Cache, "community-a", "event", "event-1")
        .unwrap()
        .is_none());
    assert!(store
        .get(RecordClass::Log, "community-a", "event", "event-1")
        .unwrap()
        .is_none());

    let failure = store.commit_canonical(CanonicalCommit {
        intent_id: "commit-invalid".into(),
        community_id: "community-a".into(),
        expected_checkpoint: 2,
        writes: vec![
            write("event", "event-2", json!({"content": "two"})),
            write("event", "", json!({"content": "invalid"})),
        ],
    });
    assert!(matches!(failure, Err(StoreError::InvalidInput(_))));
    assert_eq!(store.canonical_checkpoint("community-a").unwrap(), 2);
    assert!(store
        .get(RecordClass::Canonical, "community-a", "event", "event-2")
        .unwrap()
        .is_none());
}

#[test]
fn enforces_checkpoint_idempotency_and_cache_replace_scope() {
    let path = TestPath::new("intent");
    let store = RedbNodeStore::open(&path.0).unwrap();
    let commit = CanonicalCommit {
        intent_id: "commit-1".into(),
        community_id: "community-a".into(),
        expected_checkpoint: 0,
        writes: vec![write("event", "event-1", json!({"version": 1}))],
    };
    assert!(store.commit_canonical(commit.clone()).unwrap().applied);
    assert!(!store.commit_canonical(commit).unwrap().applied);

    let conflict = store.commit_canonical(CanonicalCommit {
        intent_id: "commit-2".into(),
        community_id: "community-a".into(),
        expected_checkpoint: 0,
        writes: vec![write("event", "event-2", json!({}))],
    });
    assert!(matches!(
        conflict,
        Err(StoreError::CheckpointConflict {
            expected: 0,
            actual: 1
        })
    ));

    store
        .replace_cache(CacheReplacement {
            intent_id: "cache-1".into(),
            community_id: "community-a".into(),
            source_checkpoint: 1,
            record_type: "thread_index".into(),
            rows: vec![write("thread_index", "event-1", json!({"count": 1}))],
        })
        .unwrap();
    store
        .replace_cache(CacheReplacement {
            intent_id: "cache-2".into(),
            community_id: "community-a".into(),
            source_checkpoint: 1,
            record_type: "thread_index".into(),
            rows: vec![],
        })
        .unwrap();
    assert!(store
        .get(RecordClass::Cache, "community-a", "thread_index", "event-1")
        .unwrap()
        .is_none());
}

#[test]
fn reopens_queries_changes_and_restores_a_verified_backup() {
    let path = TestPath::new("recovery");
    let backup = TestPath::new("backup");
    let restored = TestPath::new("restored");
    let _ = fs::remove_file(&restored.0);

    {
        let store = RedbNodeStore::open(&path.0).unwrap();
        store
            .commit_canonical(CanonicalCommit {
                intent_id: "commit-1".into(),
                community_id: "community-a".into(),
                expected_checkpoint: 0,
                writes: vec![
                    write("event", "event-1", json!({"version": 1})),
                    write("event", "event-2", json!({"version": 1})),
                ],
            })
            .unwrap();
        store.backup_to(&backup.0).unwrap();
    }

    let reopened = RedbNodeStore::open(&path.0).unwrap();
    assert_eq!(reopened.canonical_checkpoint("community-a").unwrap(), 2);
    assert_eq!(reopened.changes("community-a", 0, 10).unwrap().len(), 2);
    assert_eq!(
        reopened
            .page(RecordClass::Canonical, "community-a", "event", None, 1)
            .unwrap(),
        vec![StoredRecord {
            sequence: 1,
            record_type: "event".into(),
            key: "event-1".into(),
            deleted: false,
            value: json!({"version": 1}),
        }]
    );

    RedbNodeStore::restore_backup(&backup.0, &restored.0).unwrap();
    let restored_store = RedbNodeStore::open(&restored.0).unwrap();
    assert_eq!(
        restored_store.canonical_checkpoint("community-a").unwrap(),
        2
    );
}
