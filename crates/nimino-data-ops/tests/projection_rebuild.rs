use nimino_data_ops::{rebuild_projections, verify_replica};
use nimino_store::{
    canonical_state_digest, CacheReplacement, CanonicalCommit, NodeStorePort, ProjectionStageBatch,
    ProjectionStageSpec, RecordClass, RecordWrite, RedbNodeStore, MAX_PAGE_SIZE,
};
use serde_json::{json, Value};

fn event(key: &str, content: &str, created_at: i64, parent: &str, root: &str) -> RecordWrite {
    RecordWrite {
        record_type: "event".into(),
        key: key.into(),
        deleted: false,
        value: json!({
            "event": {"content": content, "created_at": created_at},
            "parentId": parent,
            "rootId": root,
        }),
    }
}

fn rows(store: &RedbNodeStore, record_type: &str) -> Vec<(String, Value)> {
    store
        .page(RecordClass::Cache, "community-a", record_type, None, 100)
        .unwrap()
        .into_iter()
        .map(|row| (row.key, row.value))
        .collect()
}

#[tokio::test]
#[ignore = "requires the real Nim boundary worker"]
async fn resumes_partial_stage_then_drop_and_rebuild_is_query_equivalent() {
    let worker = std::env::var("NIMINO_BOUNDARY_PRODUCTION_WORKER").unwrap();
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("projection.redb");
    let store = RedbNodeStore::open(&path).unwrap();
    store
        .commit_canonical(CanonicalCommit {
            intent_id: "seed-events".into(),
            community_id: "community-a".into(),
            expected_checkpoint: 0,
            writes: vec![
                event("a-root", "root", 10, "", ""),
                event("b-reply", "reply", 11, "a-root", "a-root"),
                event("c-reply", "nested", 12, "b-reply", "a-root"),
                event("d-large", &"x".repeat(256 * 1024), 13, "", ""),
            ],
        })
        .unwrap();
    let source = canonical_state_digest(&store, "community-a", MAX_PAGE_SIZE, || false).unwrap();
    let digest = hex::encode(source.digest);
    store
        .begin_projection_stage(ProjectionStageSpec {
            community_id: "community-a".into(),
            projection: "thread".into(),
            epoch: "first-thread".into(),
            owner_node_id: "node-a".into(),
            source_checkpoint: source.checkpoint,
            source_digest: digest,
            target_record_type: "thread_index".into(),
        })
        .unwrap();
    store
        .stage_projection_batch(ProjectionStageBatch {
            community_id: "community-a".into(),
            projection: "thread".into(),
            epoch: "first-thread".into(),
            expected_revision: 0,
            expected_cursor: String::new(),
            next_cursor: "a-root".into(),
            complete: false,
            rows: vec![RecordWrite {
                record_type: "thread_index".into(),
                key: "a-root".into(),
                deleted: false,
                value: json!({"replyCount": 0, "descendantCount": 0}),
            }],
        })
        .unwrap();
    drop(store);

    let first = rebuild_projections(&path, "community-a", worker.as_ref(), "node-a", "first")
        .await
        .unwrap();
    assert_eq!(first.projections.iter().filter(|p| p.resumed).count(), 1);
    assert!(first.projections[1].resumed);

    let store = RedbNodeStore::open(&path).unwrap();
    let expected = [
        ("search_index", rows(&store, "search_index")),
        ("thread_index", rows(&store, "thread_index")),
        ("feed_index", rows(&store, "feed_index")),
    ];
    assert_eq!(expected[0].1[1].1["content"], "reply");
    assert_eq!(expected[1].1[0].1["replyCount"], 1);
    assert_eq!(expected[1].1[0].1["descendantCount"], 2);
    assert_eq!(expected[1].1[1].1["replyCount"], 1);
    drop(store);
    let before = verify_replica(&path, "community-a", None, &[]).unwrap();
    let store = RedbNodeStore::open(&path).unwrap();
    for (record_type, _) in &expected {
        store
            .replace_cache(CacheReplacement {
                intent_id: format!("drop-{record_type}"),
                community_id: "community-a".into(),
                source_checkpoint: source.checkpoint,
                record_type: (*record_type).into(),
                rows: vec![],
            })
            .unwrap();
    }
    drop(store);

    rebuild_projections(&path, "community-a", worker.as_ref(), "node-a", "second")
        .await
        .unwrap();
    let store = RedbNodeStore::open(&path).unwrap();
    for (record_type, expected_rows) in expected {
        assert_eq!(rows(&store, record_type), expected_rows);
    }
    drop(store);
    assert_eq!(
        verify_replica(&path, "community-a", None, &[])
            .unwrap()
            .projection_digest,
        before.projection_digest
    );
}
