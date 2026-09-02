use std::path::PathBuf;

use nimino_boundary::{
    EffectLedgerState, EffectLedgerStatus, EffectReceipt, EffectReceiptOutcome,
    EffectReconcileCommand, EffectReconcileRequest,
};
use nimino_data_ops::reconcile_effect;
use nimino_store::{CanonicalCommit, NodeStorePort, RecordWrite, RedbNodeStore};
use tempfile::TempDir;

fn unknown(community: &str, key: &str) -> EffectLedgerState {
    EffectLedgerState {
        valid: true,
        community_id: community.to_owned(),
        workflow_id: "workflow-a".to_owned(),
        run_id: "run-a".to_owned(),
        step_id: key.to_owned(),
        idempotency_key: key.to_owned(),
        effect_digest: "11".repeat(32),
        lease_resource_id: format!("workflow-effect/{key}"),
        revision: 1,
        attempt: 1,
        status: EffectLedgerStatus::Unknown,
        owner_node_id: "node-a".to_owned(),
        fence_token: 1,
        receipt: None,
        reconciled_by: String::new(),
        reconcile_reason: String::new(),
    }
}

fn seed(path: &std::path::Path, community: &str, key: &str) {
    let store = RedbNodeStore::open(path).expect("open store");
    store
        .commit_canonical(CanonicalCommit {
            intent_id: format!("seed-{key}"),
            community_id: community.to_owned(),
            expected_checkpoint: 0,
            writes: vec![RecordWrite {
                record_type: "workflow_effect".to_owned(),
                key: key.to_owned(),
                deleted: false,
                value: serde_json::to_value(unknown(community, key)).expect("encode state"),
            }],
        })
        .expect("seed unknown effect");
}

#[tokio::test]
#[ignore = "requires the production Nim worker; run `just nimino-effect-scenarios`"]
async fn unknown_effects_require_explicit_retry_or_receipt() {
    let worker = std::env::var_os("NIMINO_BOUNDARY_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_WORKER is required");
    let root = TempDir::new().expect("tempdir");
    let retry_store = root.path().join("retry.redb");
    seed(&retry_store, "community-a", "effect-retry");
    let retried = reconcile_effect(
        &retry_store,
        "community-a",
        "effect-retry",
        &worker,
        EffectReconcileRequest {
            operator_authorized: true,
            operator_id: "operator-a".to_owned(),
            reason: "verified no external call".to_owned(),
            command: EffectReconcileCommand::Retry,
            receipt: None,
        },
    )
    .await
    .expect("manual retry");
    assert_eq!(retried.status, EffectLedgerStatus::Pending);
    assert_eq!(retried.idempotency_key, "effect-retry");

    let receipt_store = root.path().join("receipt.redb");
    seed(&receipt_store, "community-a", "effect-receipt");
    let settled = reconcile_effect(
        &receipt_store,
        "community-a",
        "effect-receipt",
        &worker,
        EffectReconcileRequest {
            operator_authorized: true,
            operator_id: "operator-a".to_owned(),
            reason: "verified provider receipt".to_owned(),
            command: EffectReconcileCommand::MarkSucceeded,
            receipt: Some(EffectReceipt {
                outcome: EffectReceiptOutcome::Succeeded,
                receipt_id: "provider-receipt-a".to_owned(),
                result_digest: "22".repeat(32),
            }),
        },
    )
    .await
    .expect("manual receipt");
    assert_eq!(settled.status, EffectLedgerStatus::Succeeded);
    assert_eq!(settled.reconciled_by, "operator-a");
}
