use std::{fs, path::PathBuf};

use nimino_store::{
    CacheReplacement, NodeStorePort, ProjectionStageBatch, ProjectionStageSpec, RecordClass,
    RecordWrite, RedbNodeStore,
};
use serde_json::json;
use uuid::Uuid;

struct TestPath(PathBuf);

impl TestPath {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("nimino-projection-stage-{}.redb", Uuid::new_v4())))
    }
}

impl Drop for TestPath {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn row(key: &str, value: i64) -> RecordWrite {
    RecordWrite {
        record_type: "search_index".into(),
        key: key.into(),
        deleted: false,
        value: json!({"rank": value}),
    }
}

#[test]
fn stages_resumes_and_publishes_with_existing_atomic_cache_replace() {
    let path = TestPath::new();
    let store = RedbNodeStore::open(&path.0).unwrap();
    store
        .replace_cache(CacheReplacement {
            intent_id: "old-search".into(),
            community_id: "community-a".into(),
            source_checkpoint: 0,
            record_type: "search_index".into(),
            rows: vec![row("stale", 0)],
        })
        .unwrap();
    let spec = ProjectionStageSpec {
        community_id: "community-a".into(),
        projection: "search".into(),
        epoch: "epoch-1".into(),
        owner_node_id: "node-a".into(),
        source_checkpoint: 0,
        source_digest: "a".repeat(64),
        target_record_type: "search_index".into(),
    };
    assert_eq!(
        store.begin_projection_stage(spec.clone()).unwrap().revision,
        0
    );
    let first = store
        .stage_projection_batch(ProjectionStageBatch {
            community_id: "community-a".into(),
            projection: "search".into(),
            epoch: "epoch-1".into(),
            expected_revision: 0,
            expected_cursor: String::new(),
            next_cursor: "event-1".into(),
            complete: false,
            rows: vec![row("event-1", 1)],
        })
        .unwrap();
    assert_eq!(first.revision, 1);
    drop(store);

    let reopened = RedbNodeStore::open(&path.0).unwrap();
    let recovery = reopened
        .recover_projection_stage("community-a", "search")
        .unwrap();
    assert_eq!(recovery.metadata.cursor, "event-1");
    assert_eq!(recovery.rows, vec![row("event-1", 1)]);

    let complete = reopened
        .stage_projection_batch(ProjectionStageBatch {
            community_id: "community-a".into(),
            projection: "search".into(),
            epoch: "epoch-1".into(),
            expected_revision: 1,
            expected_cursor: "event-1".into(),
            next_cursor: "event-2".into(),
            complete: true,
            rows: vec![row("event-2", 2)],
        })
        .unwrap();
    assert!(complete.complete);
    let staged = reopened
        .recover_projection_stage("community-a", "search")
        .unwrap();
    let publish = CacheReplacement {
        intent_id: "projection-search-epoch-1".into(),
        community_id: "community-a".into(),
        source_checkpoint: 0,
        record_type: "search_index".into(),
        rows: staged.rows,
    };
    assert!(reopened.replace_cache(publish.clone()).unwrap().applied);
    assert!(!reopened.replace_cache(publish).unwrap().applied);
    let queried = reopened
        .page(RecordClass::Cache, "community-a", "search_index", None, 10)
        .unwrap();
    assert_eq!(queried.len(), 2);
    assert_eq!(queried[0].key, "event-1");
    assert_eq!(queried[0].value, json!({"rank": 1}));
    assert_eq!(queried[1].key, "event-2");
    assert_eq!(queried[1].value, json!({"rank": 2}));
    reopened
        .discard_projection_stage("community-a", "search", "epoch-1")
        .unwrap();
    assert!(reopened
        .recover_projection_stage("community-a", "search")
        .is_err());
}
