use std::{fs, path::PathBuf};

use nimino_store::{
    canonical_prefix_digest, canonical_prefix_digest_at, canonical_state_digest,
    empty_prefix_digest, extend_prefix_digest, verify_range_digest, CanonicalCommit, NodeStorePort,
    RecordWrite, RedbNodeStore, StoreError,
};
use serde_json::json;
use uuid::Uuid;

struct TestPath(PathBuf);

impl TestPath {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("nimino-sync-digest-{}.redb", Uuid::new_v4())))
    }
}

impl Drop for TestPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn write(key: &str, content: &str) -> RecordWrite {
    RecordWrite {
        record_type: "event".into(),
        key: key.into(),
        deleted: false,
        value: json!({"content": content}),
    }
}

#[test]
fn recomputes_and_verifies_bounded_prefix_ranges() {
    let path = TestPath::new();
    let store = RedbNodeStore::open(&path.0).unwrap();
    store
        .commit_canonical(CanonicalCommit {
            intent_id: "seed".into(),
            community_id: "community-a".into(),
            expected_checkpoint: 0,
            writes: vec![
                write("event-1", "one"),
                write("event-2", "two"),
                write("event-3", "three"),
                write("event-4", "four"),
            ],
        })
        .unwrap();

    let first = store.changes("community-a", 0, 2).unwrap();
    let second = store.changes("community-a", 2, 2).unwrap();
    let first_digest = extend_prefix_digest(empty_prefix_digest(), &first).unwrap();
    let final_digest = extend_prefix_digest(first_digest, &second).unwrap();
    assert!(verify_range_digest(first_digest, &second, final_digest).unwrap());

    let recomputed = canonical_prefix_digest(&store, "community-a", 2, || false).unwrap();
    assert_eq!(recomputed.checkpoint, 4);
    assert_eq!(recomputed.digest, final_digest);
    assert_eq!(recomputed.hex().len(), 64);
    assert_eq!(
        canonical_prefix_digest_at(&store, "community-a", 2, 1, || false)
            .unwrap()
            .digest,
        first_digest
    );

    drop(store);
    let reopened = RedbNodeStore::open(&path.0).unwrap();
    assert_eq!(
        canonical_prefix_digest(&reopened, "community-a", 2, || false)
            .unwrap()
            .digest,
        final_digest
    );

    let mut tampered = second;
    tampered[0].value = json!({"content": "changed"});
    assert!(!verify_range_digest(first_digest, &tampered, final_digest).unwrap());
}

#[test]
fn prefix_recompute_is_cancelable_between_bounded_pages() {
    let path = TestPath::new();
    let store = RedbNodeStore::open(&path.0).unwrap();
    store
        .commit_canonical(CanonicalCommit {
            intent_id: "seed".into(),
            community_id: "community-a".into(),
            expected_checkpoint: 0,
            writes: vec![write("event-1", "one"), write("event-2", "two")],
        })
        .unwrap();

    let mut pages = 0;
    let error = canonical_prefix_digest(&store, "community-a", 1, || {
        pages += 1;
        pages > 1
    })
    .unwrap_err();
    assert!(matches!(error, StoreError::SyncCancelled));
    assert_eq!(pages, 2);
}

#[test]
fn state_digest_ignores_node_local_commit_order() {
    let first_path = TestPath::new();
    let second_path = TestPath::new();
    let first = RedbNodeStore::open(&first_path.0).unwrap();
    let second = RedbNodeStore::open(&second_path.0).unwrap();
    let a = write("a", "one");
    let b = write("b", "two");
    first
        .commit_canonical(CanonicalCommit {
            intent_id: "first".into(),
            community_id: "community-a".into(),
            expected_checkpoint: 0,
            writes: vec![a.clone(), b.clone()],
        })
        .unwrap();
    second
        .commit_canonical(CanonicalCommit {
            intent_id: "second".into(),
            community_id: "community-a".into(),
            expected_checkpoint: 0,
            writes: vec![b, a],
        })
        .unwrap();

    assert_ne!(
        canonical_prefix_digest(&first, "community-a", 1, || false).unwrap(),
        canonical_prefix_digest(&second, "community-a", 1, || false).unwrap()
    );
    assert_eq!(
        canonical_state_digest(&first, "community-a", 1, || false)
            .unwrap()
            .digest,
        canonical_state_digest(&second, "community-a", 1, || false)
            .unwrap()
            .digest
    );
}
