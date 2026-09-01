use std::{fs, path::PathBuf};

use nimino_store::{
    CanonicalCommit, ControlLogEntry, ControlLogStorePort, ControlMetadata, ControlSnapshot,
    NodeStorePort, RecordWrite, RedbNodeStore, StoreError,
};
use serde_json::json;
use uuid::Uuid;

struct TestPath(PathBuf);

impl TestPath {
    fn new(name: &str) -> Self {
        Self(std::env::temp_dir().join(format!("nimino-control-{name}-{}.redb", Uuid::new_v4())))
    }
}

impl Drop for TestPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn entry(index: u64, term: u64, command: &str) -> ControlLogEntry {
    ControlLogEntry {
        index,
        term,
        voter_epoch: 1,
        kind: "command".into(),
        command_id: format!("command-{index}"),
        payload: command.as_bytes().to_vec(),
        target_voters: Vec::new(),
    }
}

#[test]
fn appends_fsyncs_and_recovers_atomic_metadata() {
    let path = TestPath::new("append");
    {
        let store = RedbNodeStore::open(&path.0).unwrap();
        assert_eq!(
            store
                .replace_control_suffix(0, vec![entry(1, 1, "one"), entry(2, 1, "two")])
                .unwrap(),
            2
        );
        let metadata = store
            .compare_and_set_control_metadata(
                0,
                ControlMetadata {
                    term: 2,
                    voted_for: Some("node-a".into()),
                    commit_index: 2,
                    applied_index: 1,
                },
            )
            .unwrap();
        assert_eq!(metadata.revision, 1);
        assert!(matches!(
            store.compare_and_set_control_metadata(0, metadata.state.clone()),
            Err(StoreError::ControlMetadataConflict {
                expected: 0,
                actual: 1
            })
        ));
    }

    let recovered = RedbNodeStore::open(&path.0)
        .unwrap()
        .recover_control_state()
        .unwrap();
    assert_eq!(recovered.metadata.revision, 1);
    assert_eq!(recovered.metadata.state.term, 2);
    assert_eq!(recovered.entries.len(), 2);
    assert_eq!(recovered.entries[1].payload, b"two");
}

#[test]
fn replaces_only_the_uncommitted_suffix() {
    let path = TestPath::new("suffix");
    let store = RedbNodeStore::open(&path.0).unwrap();
    store
        .replace_control_suffix(0, vec![entry(1, 1, "one"), entry(2, 1, "stale")])
        .unwrap();
    store
        .compare_and_set_control_metadata(
            0,
            ControlMetadata {
                term: 1,
                voted_for: None,
                commit_index: 1,
                applied_index: 1,
            },
        )
        .unwrap();

    store
        .replace_control_suffix(1, vec![entry(2, 2, "replacement")])
        .unwrap();
    let recovered = store.recover_control_state().unwrap();
    assert_eq!(recovered.entries[1].term, 2);
    assert_eq!(recovered.entries[1].payload, b"replacement");
    assert!(matches!(
        store.replace_control_suffix(0, vec![entry(1, 3, "forbidden")]),
        Err(StoreError::CommittedControlPrefix { committed: 1 })
    ));
    assert!(matches!(
        store.replace_control_suffix(3, vec![entry(4, 3, "gap")]),
        Err(StoreError::ControlLogGap {
            expected: 2,
            actual: 3
        })
    ));
}

#[test]
fn installs_snapshot_then_recovers_the_suffix() {
    let path = TestPath::new("snapshot");
    let store = RedbNodeStore::open(&path.0).unwrap();
    store
        .replace_control_suffix(
            0,
            vec![entry(1, 1, "one"), entry(2, 1, "two"), entry(3, 2, "three")],
        )
        .unwrap();
    store
        .compare_and_set_control_metadata(
            0,
            ControlMetadata {
                term: 2,
                voted_for: Some("node-b".into()),
                commit_index: 3,
                applied_index: 3,
            },
        )
        .unwrap();
    let snapshot = ControlSnapshot {
        last_included_index: 2,
        last_included_term: 1,
        voter_epoch: 1,
        voter_phase: "stable-old".into(),
        old_voters: vec!["node-a".into()],
        new_voters: Vec::new(),
        state: b"nim-state-through-two".to_vec(),
    };
    assert!(store.install_control_snapshot(1, snapshot.clone()).unwrap());
    assert!(!store.install_control_snapshot(2, snapshot.clone()).unwrap());
    let mut conflicting = snapshot.clone();
    conflicting.state = b"different-state".to_vec();
    assert!(matches!(
        store.install_control_snapshot(2, conflicting),
        Err(StoreError::ControlSnapshotConflict { index: 2 })
    ));

    let recovered = store.recover_control_state().unwrap();
    assert_eq!(recovered.snapshot, Some(snapshot));
    assert_eq!(recovered.entries, vec![entry(3, 2, "three")]);

    let incoming = ControlSnapshot {
        last_included_index: 5,
        last_included_term: 3,
        voter_epoch: 2,
        voter_phase: "joint".into(),
        old_voters: vec!["node-a".into()],
        new_voters: vec!["node-b".into()],
        state: b"remote-state-through-five".to_vec(),
    };
    assert!(store.install_control_snapshot(2, incoming.clone()).unwrap());
    let recovered = store.recover_control_state().unwrap();
    assert_eq!(recovered.snapshot, Some(incoming));
    assert!(recovered.entries.is_empty());
    assert_eq!(recovered.metadata.revision, 3);
    assert_eq!(recovered.metadata.state.commit_index, 5);
    assert_eq!(recovered.metadata.state.applied_index, 5);
}

#[test]
fn keeps_control_log_out_of_canonical_anti_entropy() {
    let path = TestPath::new("isolation");
    let store = RedbNodeStore::open(&path.0).unwrap();
    store
        .commit_canonical(CanonicalCommit {
            intent_id: "data-1".into(),
            community_id: "community-a".into(),
            expected_checkpoint: 0,
            writes: vec![RecordWrite {
                record_type: "event".into(),
                key: "event-1".into(),
                deleted: false,
                value: json!({"content": "canonical"}),
            }],
        })
        .unwrap();
    store
        .replace_control_suffix(0, vec![entry(1, 1, "control-only")])
        .unwrap();

    let changes = store.changes("community-a", 0, 10).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].key, "event-1");
    assert_eq!(store.recover_control_state().unwrap().entries.len(), 1);
}
