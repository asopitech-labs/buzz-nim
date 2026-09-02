#![cfg(feature = "test-hooks")]

use std::{path::PathBuf, process::Stdio, time::Duration};

use nimino_boundary::{
    AgentPolicyRequest, AgentPolicyResult, BoundaryConfig, BoundaryError, BoundaryRequest,
    BoundaryResponse, BoundaryResult, BoundaryRuntime, CallContext, CliPolicyRequest,
    CliPolicyResult, ClusterLifecycleError, ClusterLifecyclePolicyRequest,
    ClusterLifecyclePolicyResult, ClusterNodeState, CommunityPolicyRequest, CommunityPolicyResult,
    DmPolicyRequest, DmPolicyResult, EchoPayload, EventPolicyRequest, EventPolicyResult,
    LifecycleCommand, LifecycleEffect, LifecycleTransitionRequest, MembershipPolicyRequest,
    MembershipPolicyResult, ModerationPolicyRequest, ModerationPolicyResult, RemoteErrorCode,
    WorkflowPolicyRequest, WorkflowPolicyResult, MAX_FRAME_BYTES, MAX_INFLIGHT, PROTOCOL_NAME,
    PROTOCOL_VERSION, SCHEMA_HASH, WORKER_ROLE,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

fn worker_path() -> PathBuf {
    std::env::var_os("NIMINO_BOUNDARY_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_WORKER must point to the test worker")
}

fn mismatch_worker_path() -> PathBuf {
    std::env::var_os("NIMINO_BOUNDARY_MISMATCH_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_MISMATCH_WORKER must point to the mismatch worker")
}

fn production_worker_path() -> PathBuf {
    std::env::var_os("NIMINO_BOUNDARY_PRODUCTION_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_PRODUCTION_WORKER must point to the production worker")
}

async fn runtime(queue_capacity: usize) -> BoundaryRuntime {
    BoundaryRuntime::start(BoundaryConfig::new(worker_path()).with_queue_capacity(queue_capacity))
        .await
        .expect("worker starts and negotiates v1")
}

async fn raw_exchange(
    stdin: &mut tokio::process::ChildStdin,
    stdout: &mut tokio::process::ChildStdout,
    request: &Value,
) -> Value {
    let request = serde_json::to_vec(request).expect("encode raw request");
    stdin
        .write_all(&(request.len() as u32).to_be_bytes())
        .await
        .expect("write frame length");
    stdin.write_all(&request).await.expect("write frame body");
    stdin.flush().await.expect("flush request");

    let response_length = stdout.read_u32().await.expect("response frame length") as usize;
    let mut response = vec![0; response_length];
    stdout
        .read_exact(&mut response)
        .await
        .expect("response frame body");
    serde_json::from_slice(&response).expect("response JSON")
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn rust_and_nim_round_trip_and_preserve_typed_remote_failure() {
    let runtime = runtime(8).await;
    let client = runtime.client();

    let value = client
        .call(
            BoundaryRequest::echo(json!({"message": "round trip"})),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect("echo succeeds");
    assert_eq!(
        value,
        BoundaryResult::Echo(EchoPayload {
            data: json!({"message": "round trip"})
        })
    );

    let error = client
        .call(
            BoundaryRequest::test_remote_failure(),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect_err("unknown operation is typed");
    assert!(matches!(
        error,
        BoundaryError::Remote(ref fault) if fault.code == RemoteErrorCode::UnknownOperation
    ));

    runtime.shutdown().await.expect("clean shutdown");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventPolicyCorpus {
    schema_version: u16,
    cases: Vec<EventPolicyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventPolicyCase {
    name: String,
    standard: String,
    input: EventPolicyRequest,
    expected: EventPolicyResult,
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn event_policy_golden_corpus_crosses_the_real_worker_boundary() {
    let corpus: EventPolicyCorpus = serde_json::from_str(include_str!(
        "../../../contracts/nimino-event/v1/golden.json"
    ))
    .expect("valid event policy corpus");
    assert_eq!(corpus.schema_version, 1);

    let runtime = runtime(8).await;
    let client = runtime.client();
    for case in corpus.cases {
        assert!(
            !case.standard.is_empty(),
            "{} has no NIP reference",
            case.name
        );
        let result = client
            .call(
                BoundaryRequest::event_policy(case.input),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));
        assert_eq!(
            result,
            BoundaryResult::EventPolicy(case.expected),
            "{}",
            case.name
        );
    }
    runtime.shutdown().await.expect("clean shutdown");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CommunityPolicyCorpus {
    schema_version: u16,
    cases: Vec<CommunityPolicyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommunityPolicyCase {
    name: String,
    invariant: String,
    input: CommunityPolicyRequest,
    expected: CommunityPolicyResult,
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn community_policy_golden_corpus_crosses_the_real_worker_boundary() {
    let corpus: CommunityPolicyCorpus = serde_json::from_str(include_str!(
        "../../../contracts/nimino-community/v1/golden.json"
    ))
    .expect("valid community policy corpus");
    assert_eq!(corpus.schema_version, 1);

    let runtime = runtime(8).await;
    let client = runtime.client();
    for case in corpus.cases {
        assert!(!case.invariant.is_empty(), "{} has no invariant", case.name);
        let result = client
            .call(
                BoundaryRequest::community_policy(case.input),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));
        assert_eq!(
            result,
            BoundaryResult::CommunityPolicy(case.expected),
            "{}",
            case.name
        );
    }
    runtime.shutdown().await.expect("clean shutdown");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MembershipPolicyCorpus {
    schema_version: u16,
    cases: Vec<MembershipPolicyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MembershipPolicyCase {
    name: String,
    invariant: String,
    input: MembershipPolicyRequest,
    expected: MembershipPolicyResult,
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn membership_policy_golden_corpus_crosses_the_real_worker_boundary() {
    let corpus: MembershipPolicyCorpus = serde_json::from_str(include_str!(
        "../../../contracts/nimino-membership/v1/golden.json"
    ))
    .expect("valid membership policy corpus");
    assert_eq!(corpus.schema_version, 1);

    let runtime = runtime(8).await;
    let client = runtime.client();
    for case in corpus.cases {
        assert!(!case.invariant.is_empty(), "{} has no invariant", case.name);
        let result = client
            .call(
                BoundaryRequest::membership_policy(case.input),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));
        assert_eq!(
            result,
            BoundaryResult::MembershipPolicy(case.expected),
            "{}",
            case.name
        );
    }
    runtime.shutdown().await.expect("clean shutdown");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DmPolicyCorpus {
    schema_version: u16,
    cases: Vec<DmPolicyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DmPolicyCase {
    name: String,
    invariant: String,
    input: DmPolicyRequest,
    expected: DmPolicyResult,
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn dm_policy_golden_corpus_crosses_the_real_worker_boundary() {
    let corpus: DmPolicyCorpus =
        serde_json::from_str(include_str!("../../../contracts/nimino-dm/v1/golden.json"))
            .expect("valid DM policy corpus");
    assert_eq!(corpus.schema_version, 1);

    let runtime = runtime(8).await;
    let client = runtime.client();
    for case in corpus.cases {
        assert!(!case.invariant.is_empty(), "{} has no invariant", case.name);
        let result = client
            .call(
                BoundaryRequest::dm_policy(case.input),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));
        assert_eq!(
            result,
            BoundaryResult::DmPolicy(case.expected),
            "{}",
            case.name
        );
    }
    runtime.shutdown().await.expect("clean shutdown");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ModerationPolicyCorpus {
    schema_version: u16,
    cases: Vec<ModerationPolicyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModerationPolicyCase {
    name: String,
    invariant: String,
    input: ModerationPolicyRequest,
    expected: ModerationPolicyResult,
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn moderation_policy_golden_corpus_crosses_the_real_worker_boundary() {
    let corpus: ModerationPolicyCorpus = serde_json::from_str(include_str!(
        "../../../contracts/nimino-moderation/v1/golden.json"
    ))
    .expect("valid moderation policy corpus");
    assert_eq!(corpus.schema_version, 1);

    let runtime = runtime(8).await;
    let client = runtime.client();
    for case in corpus.cases {
        assert!(!case.invariant.is_empty(), "{} has no invariant", case.name);
        let result = client
            .call(
                BoundaryRequest::moderation_policy(case.input),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));
        assert_eq!(
            result,
            BoundaryResult::ModerationPolicy(case.expected),
            "{}",
            case.name
        );
    }
    runtime.shutdown().await.expect("clean shutdown");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WorkflowPolicyCorpus {
    schema_version: u16,
    cases: Vec<WorkflowPolicyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowPolicyCase {
    name: String,
    invariant: String,
    input: WorkflowPolicyRequest,
    expected: WorkflowPolicyResult,
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn workflow_policy_golden_corpus_crosses_the_real_worker_boundary() {
    let corpus: WorkflowPolicyCorpus = serde_json::from_str(include_str!(
        "../../../contracts/nimino-workflow/v1/golden.json"
    ))
    .expect("valid workflow policy corpus");
    assert_eq!(corpus.schema_version, 1);

    let runtime = runtime(8).await;
    let client = runtime.client();
    for case in corpus.cases {
        assert!(!case.invariant.is_empty(), "{} has no invariant", case.name);
        let result = client
            .call(
                BoundaryRequest::workflow_policy(case.input),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));
        assert_eq!(
            result,
            BoundaryResult::WorkflowPolicy(case.expected),
            "{}",
            case.name
        );
    }
    runtime.shutdown().await.expect("clean shutdown");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CliPolicyCorpus {
    schema_version: u16,
    cases: Vec<CliPolicyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CliPolicyCase {
    name: String,
    invariant: String,
    input: CliPolicyRequest,
    expected: CliPolicyResult,
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn cli_policy_golden_corpus_crosses_the_real_worker_boundary() {
    let corpus: CliPolicyCorpus =
        serde_json::from_str(include_str!("../../../contracts/nimino-cli/v1/golden.json"))
            .expect("valid CLI policy corpus");
    assert_eq!(corpus.schema_version, 1);

    let runtime = runtime(8).await;
    let client = runtime.client();
    for case in corpus.cases {
        assert!(!case.invariant.is_empty(), "{} has no invariant", case.name);
        let result = client
            .call(
                BoundaryRequest::cli_policy(case.input),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));
        assert_eq!(
            result,
            BoundaryResult::CliPolicy(case.expected),
            "{}",
            case.name
        );
    }
    runtime.shutdown().await.expect("clean shutdown");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AgentPolicyCorpus {
    schema_version: u16,
    cases: Vec<AgentPolicyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentPolicyCase {
    name: String,
    invariant: String,
    input: AgentPolicyRequest,
    expected: AgentPolicyResult,
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn agent_policy_golden_corpus_crosses_the_real_worker_boundary() {
    let corpus: AgentPolicyCorpus = serde_json::from_str(include_str!(
        "../../../contracts/nimino-agent/v1/golden.json"
    ))
    .expect("valid agent policy corpus");
    assert_eq!(corpus.schema_version, 1);

    let runtime = runtime(8).await;
    let client = runtime.client();
    for case in corpus.cases {
        assert!(!case.invariant.is_empty(), "{} has no invariant", case.name);
        let result = client
            .call(
                BoundaryRequest::agent_policy(case.input),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));
        assert_eq!(
            result,
            BoundaryResult::AgentPolicy(case.expected),
            "{}",
            case.name
        );
    }
    runtime.shutdown().await.expect("clean shutdown");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ClusterLifecycleCorpus {
    schema_version: u16,
    cases: Vec<ClusterLifecycleCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClusterLifecycleCase {
    name: String,
    invariant: String,
    input: ClusterLifecyclePolicyRequest,
    expected: ClusterLifecyclePolicyResult,
}

fn lifecycle_transition(
    command: LifecycleCommand,
    current_state: ClusterNodeState,
) -> ClusterLifecyclePolicyRequest {
    ClusterLifecyclePolicyRequest::Transition {
        request: LifecycleTransitionRequest {
            command,
            current_state,
            authenticated: true,
            revoked: false,
            identity_unique: true,
            product_capability: "nimino-v1".to_owned(),
            control_protocol_version: 1,
            data_protocol_version: 1,
            control_decision_committed: true,
            snapshot_installed: true,
            checkpoint_matches: true,
            required_voter_epoch: 2,
            observed_voter_epoch: 2,
            active_work: 0,
        },
    }
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn cluster_lifecycle_golden_corpus_crosses_the_real_worker_boundary() {
    let corpus: ClusterLifecycleCorpus = serde_json::from_str(include_str!(
        "../../../contracts/nimino-cluster/v1/golden.json"
    ))
    .expect("valid cluster lifecycle corpus");
    assert_eq!(corpus.schema_version, 1);

    let runtime = runtime(8).await;
    let client = runtime.client();
    for case in corpus.cases {
        assert!(!case.invariant.is_empty(), "{} has no invariant", case.name);
        let result = client
            .call(
                BoundaryRequest::cluster_lifecycle(case.input),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));
        assert_eq!(
            result,
            BoundaryResult::ClusterLifecycle(case.expected),
            "{}",
            case.name
        );
    }
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn sync_policy_fixture_crosses_the_real_worker_boundary() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/sync-policy.request.json"
    ))
    .expect("valid sync policy request");
    let expected: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/sync-policy.response.json"
    ))
    .expect("valid sync policy response");

    let runtime = runtime(8).await;
    let result = runtime
        .client()
        .call(request, CallContext::with_timeout(Duration::from_secs(2)))
        .await
        .expect("sync policy call succeeds");
    assert_eq!(
        result,
        expected.into_result().expect("fixture is successful")
    );
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn control_policy_fixture_crosses_the_real_worker_boundary() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/control-policy.request.json"
    ))
    .expect("valid control policy request");
    let expected: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/control-policy.response.json"
    ))
    .expect("valid control policy response");

    let runtime = runtime(8).await;
    let result = runtime
        .client()
        .call(request, CallContext::with_timeout(Duration::from_secs(2)))
        .await
        .expect("control policy call succeeds");
    assert_eq!(
        result,
        expected.into_result().expect("fixture is successful")
    );
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn admission_policy_fixture_crosses_the_real_worker_boundary() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/admission-policy.request.json"
    ))
    .expect("valid admission policy request");
    let expected: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/admission-policy.response.json"
    ))
    .expect("valid admission policy response");

    let runtime = runtime(8).await;
    let result = runtime
        .client()
        .call(request, CallContext::with_timeout(Duration::from_secs(2)))
        .await
        .expect("admission policy call succeeds");
    assert_eq!(result, expected.into_result().expect("fixture success"));
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn authorization_invalidation_fixture_crosses_the_real_worker_boundary() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/authorization-invalidation-policy.request.json"
    ))
    .expect("valid authorization invalidation request");
    let expected: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/authorization-invalidation-policy.response.json"
    ))
    .expect("valid authorization invalidation response");

    let runtime = runtime(8).await;
    let result = runtime
        .client()
        .call(request, CallContext::with_timeout(Duration::from_secs(2)))
        .await
        .expect("authorization invalidation policy call succeeds");
    assert_eq!(result, expected.into_result().expect("fixture success"));
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn ephemeral_policy_fixture_crosses_the_real_worker_boundary() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/ephemeral-policy.request.json"
    ))
    .expect("valid ephemeral policy request");
    let expected: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/ephemeral-policy.response.json"
    ))
    .expect("valid ephemeral policy response");

    let runtime = runtime(8).await;
    let result = runtime
        .client()
        .call(request, CallContext::with_timeout(Duration::from_secs(2)))
        .await
        .expect("ephemeral policy call succeeds");
    assert_eq!(result, expected.into_result().expect("fixture success"));
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn lease_policy_fixture_crosses_the_real_worker_boundary() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/lease-policy.request.json"
    ))
    .expect("valid lease policy request");
    let expected: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/lease-policy.response.json"
    ))
    .expect("valid lease policy response");

    let runtime = runtime(8).await;
    let result = runtime
        .client()
        .call(request, CallContext::with_timeout(Duration::from_secs(2)))
        .await
        .expect("lease policy call succeeds");
    assert_eq!(result, expected.into_result().expect("fixture success"));
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn effect_policy_fixture_crosses_the_real_worker_boundary() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/effect-policy.request.json"
    ))
    .expect("valid effect policy request");
    let expected: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/effect-policy.response.json"
    ))
    .expect("valid effect policy response");

    let runtime = runtime(8).await;
    let result = runtime
        .client()
        .call(request, CallContext::with_timeout(Duration::from_secs(2)))
        .await
        .expect("effect policy call succeeds");
    assert_eq!(result, expected.into_result().expect("fixture success"));
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn object_policy_fixture_crosses_the_real_worker_boundary() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/object-policy.request.json"
    ))
    .expect("valid object policy request");
    let expected: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/object-policy.response.json"
    ))
    .expect("valid object policy response");

    let runtime = runtime(8).await;
    let result = runtime
        .client()
        .call(request, CallContext::with_timeout(Duration::from_secs(2)))
        .await
        .expect("object policy call succeeds");
    assert_eq!(result, expected.into_result().expect("fixture success"));
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn projection_policy_fixture_crosses_the_real_worker_boundary() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/projection-policy.request.json"
    ))
    .expect("valid projection policy request");
    let expected: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/projection-policy.response.json"
    ))
    .expect("valid projection policy response");

    let runtime = runtime(8).await;
    let result = runtime
        .client()
        .call(request, CallContext::with_timeout(Duration::from_secs(2)))
        .await
        .expect("projection policy call succeeds");
    assert_eq!(result, expected.into_result().expect("fixture success"));
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn real_worker_completes_join_drain_and_rejoin_without_skips() {
    let runtime = runtime(8).await;
    let client = runtime.client();
    let mut state = ClusterNodeState::Offline;
    for (command, expected, effect) in [
        (
            LifecycleCommand::Join,
            ClusterNodeState::Joining,
            LifecycleEffect::EnterJoining,
        ),
        (
            LifecycleCommand::StartSync,
            ClusterNodeState::Syncing,
            LifecycleEffect::EnterSyncing,
        ),
        (
            LifecycleCommand::MarkReady,
            ClusterNodeState::Ready,
            LifecycleEffect::EnterReady,
        ),
        (
            LifecycleCommand::BeginDrain,
            ClusterNodeState::Draining,
            LifecycleEffect::EnterDraining,
        ),
        (
            LifecycleCommand::MarkOffline,
            ClusterNodeState::Offline,
            LifecycleEffect::EnterOffline,
        ),
        (
            LifecycleCommand::Join,
            ClusterNodeState::Joining,
            LifecycleEffect::EnterJoining,
        ),
    ] {
        let result = client
            .call(
                BoundaryRequest::cluster_lifecycle(lifecycle_transition(command, state)),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .expect("real worker lifecycle call");
        assert_eq!(
            result,
            BoundaryResult::ClusterLifecycle(ClusterLifecyclePolicyResult::Transition {
                effect,
                next_state: expected,
                error: ClusterLifecycleError::None,
            })
        );
        state = expected;
    }
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn timeout_and_cancel_recycle_the_worker_before_the_next_call() {
    let runtime = runtime(8).await;
    let client = runtime.client();

    let timeout = client
        .call(
            BoundaryRequest::test_sleep(5_000),
            CallContext::with_timeout(Duration::from_millis(50)),
        )
        .await
        .expect_err("sleep exceeds deadline");
    assert!(matches!(timeout, BoundaryError::DeadlineExceeded));

    let cancellation = CancellationToken::new();
    let cancel_from_test = cancellation.clone();
    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_from_test.cancel();
    });
    let cancelled = client
        .call(
            BoundaryRequest::test_sleep(5_000),
            CallContext::with_timeout_and_cancellation(Duration::from_secs(2), cancellation),
        )
        .await
        .expect_err("call is cancelled");
    cancel_task.await.expect("cancel task");
    assert!(matches!(cancelled, BoundaryError::Cancelled));

    let recovered = client
        .call(
            BoundaryRequest::echo(json!({"recovered": true})),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect("a fresh worker handles the next call");
    assert_eq!(
        recovered,
        BoundaryResult::Echo(EchoPayload {
            data: json!({"recovered": true})
        })
    );

    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn dropping_the_call_future_cancels_and_recycles_the_worker() {
    let runtime = runtime(8).await;
    let client = runtime.client();
    let abandoned_client = client.clone();
    let abandoned = tokio::spawn(async move {
        abandoned_client
            .call(
                BoundaryRequest::test_sleep(5_000),
                CallContext::with_timeout(Duration::from_secs(10)),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    abandoned.abort();
    let _ = abandoned.await;

    let started = std::time::Instant::now();
    let recovered = client
        .call(
            BoundaryRequest::echo(json!({"after": "abandon"})),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect("abandoned call is cancelled before the next call");
    assert_eq!(
        recovered,
        BoundaryResult::Echo(EchoPayload {
            data: json!({"after": "abandon"})
        })
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn queued_call_deadline_includes_time_waiting_for_the_worker() {
    let runtime = runtime(2).await;
    let client = runtime.client();
    let busy_client = client.clone();
    let busy = tokio::spawn(async move {
        busy_client
            .call(
                BoundaryRequest::test_sleep(500),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(30)).await;

    let started = std::time::Instant::now();
    let queued = client
        .call(
            BoundaryRequest::echo(json!({"tooLate": true})),
            CallContext::with_timeout(Duration::from_millis(50)),
        )
        .await
        .expect_err("queue wait consumes the call budget");
    assert!(matches!(queued, BoundaryError::DeadlineExceeded));
    assert!(started.elapsed() < Duration::from_millis(250));

    assert!(busy.await.expect("busy task").is_ok());
    runtime.shutdown().await.expect("clean shutdown");
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn dropping_runtime_reaps_the_worker_process() {
    let runtime = runtime(2).await;
    let result = runtime
        .client()
        .call(
            BoundaryRequest::test_pid(),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect("test worker returns its pid");
    let BoundaryResult::Test(payload) = result else {
        panic!("pid hook returned the wrong result variant");
    };
    let pid = payload["pid"].as_u64().expect("numeric worker pid");

    drop(runtime);
    let process_path = PathBuf::from(format!("/proc/{pid}"));
    for _ in 0..100 {
        if !process_path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("worker {pid} was not reaped after runtime drop");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn crash_is_isolated_and_queue_capacity_applies_backpressure() {
    let runtime = runtime(1).await;
    let client = runtime.client();

    let crash = client
        .call(
            BoundaryRequest::test_crash(),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect_err("test worker exits");
    assert!(matches!(crash, BoundaryError::WorkerExited { .. }));

    let first_client = client.clone();
    let first = tokio::spawn(async move {
        first_client
            .call(
                BoundaryRequest::test_sleep(500),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let second_client = client.clone();
    let second = tokio::spawn(async move {
        second_client
            .call(
                BoundaryRequest::echo(json!({"queued": true})),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let backpressure = client
        .call(
            BoundaryRequest::echo(json!({"overflow": true})),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect_err("bounded queue rejects overflow immediately");
    assert!(matches!(backpressure, BoundaryError::Backpressure));

    assert!(first.await.expect("first task").is_ok());
    assert!(second.await.expect("second task").is_ok());
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires both Nim test workers; run `just nim-boundary-test`"]
async fn handshake_rejects_schema_mismatch_without_fallback() {
    let error = BoundaryRuntime::start(BoundaryConfig::new(mismatch_worker_path()))
        .await
        .expect_err("schema mismatch prevents readiness");
    assert!(matches!(error, BoundaryError::ContractMismatch));
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn corrupted_or_mismatched_responses_recycle_before_the_next_call() {
    for request in [
        BoundaryRequest::test_garbage(),
        BoundaryRequest::test_malformed(),
        BoundaryRequest::test_wrong_id(),
    ] {
        let runtime = runtime(8).await;
        let client = runtime.client();
        let error = client
            .call(request, CallContext::with_timeout(Duration::from_secs(2)))
            .await
            .expect_err("invalid response is rejected");
        assert!(matches!(error, BoundaryError::ProtocolViolation(_)));

        let recovered = client
            .call(
                BoundaryRequest::echo(json!({"fresh": true})),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .expect("replacement worker is ready");
        assert_eq!(
            recovered,
            BoundaryResult::Echo(EchoPayload {
                data: json!({"fresh": true})
            })
        );
        runtime.shutdown().await.expect("clean shutdown");
    }
}

#[tokio::test]
#[ignore = "requires the production Nim worker; run `just nim-boundary-test`"]
async fn production_worker_excludes_failure_test_operations() {
    let runtime = BoundaryRuntime::start(BoundaryConfig::new(production_worker_path()))
        .await
        .expect("production worker reaches ready");
    let client = runtime.client();

    let unavailable = client
        .call(
            BoundaryRequest::test_crash(),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect_err("test operation is absent from production");
    assert!(matches!(
        unavailable,
        BoundaryError::Remote(ref fault) if fault.code == RemoteErrorCode::UnknownOperation
    ));

    let value = client
        .call(
            BoundaryRequest::echo(json!({"production": true})),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect("production worker remains ready");
    assert_eq!(
        value,
        BoundaryResult::Echo(EchoPayload {
            data: json!({"production": true})
        })
    );
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the production Nim worker; run `just nim-boundary-test`"]
async fn production_worker_bounds_generated_error_messages() {
    let mut child = tokio::process::Command::new(production_worker_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn production worker");
    let mut stdin = child.stdin.take().expect("worker stdin");
    let mut stdout = child.stdout.take().expect("worker stdout");
    let unknown_field = "界".repeat(2_000);
    for request in [
        json!({
            "protocol": "nimino.core.boundary",
            "version": 1,
            "requestId": "oversized-diagnostic",
            "operation": "boundary.echo",
            "payload": {"data": null},
            (unknown_field): true,
        }),
        json!({
            "protocol": "nimino.core.boundary",
            "version": 1,
            "requestId": "x".repeat(129),
            "operation": "boundary.echo",
            "payload": {"data": null},
        }),
        json!({
            "protocol": "nimino.core.boundary",
            "version": 1,
            "requestId": "",
            "operation": "",
            "payload": {},
        }),
        json!({
            "protocol": "nimino.core.boundary",
            "version": 1,
            "requestId": "invalid-operation",
            "operation": "x".repeat(129),
            "payload": {},
        }),
    ] {
        let value = raw_exchange(&mut stdin, &mut stdout, &request).await;
        assert_eq!(value["error"]["code"], "INVALID_REQUEST");
        assert!((1..=128).contains(
            &value["requestId"]
                .as_str()
                .expect("request id")
                .chars()
                .count()
        ));
        assert!((1..=128).contains(
            &value["operation"]
                .as_str()
                .expect("operation")
                .chars()
                .count()
        ));
        assert!(
            value["error"]["message"]
                .as_str()
                .expect("message")
                .chars()
                .count()
                <= 1_024
        );
        serde_json::from_value::<BoundaryResponse>(value).expect("schema-valid failure response");
    }

    child.start_kill().expect("kill worker");
    child.wait().await.expect("reap worker");
}

#[tokio::test]
#[ignore = "requires the production Nim worker; run `just nim-boundary-test`"]
async fn production_worker_converts_oversized_success_without_exiting() {
    let mut child = tokio::process::Command::new(production_worker_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn production worker");
    let mut stdin = child.stdin.take().expect("worker stdin");
    let mut stdout = child.stdout.take().expect("worker stdout");

    let hello = raw_exchange(
        &mut stdin,
        &mut stdout,
        &json!({
            "protocol": PROTOCOL_NAME,
            "version": PROTOCOL_VERSION,
            "requestId": "raw-hello",
            "operation": "system.hello",
            "payload": {
                "schemaHash": SCHEMA_HASH,
                "workerRole": WORKER_ROLE,
                "maxFrameBytes": MAX_FRAME_BYTES,
                "maxInflight": MAX_INFLIGHT,
            },
        }),
    )
    .await;
    serde_json::from_value::<BoundaryResponse>(hello).expect("valid ready response");

    let mut maximal = json!({
        "protocol": PROTOCOL_NAME,
        "version": PROTOCOL_VERSION,
        "requestId": "max-frame",
        "operation": "boundary.echo",
        "payload": {"data": {"blob": ""}},
    });
    let base_length = serde_json::to_vec(&maximal).expect("base request").len();
    maximal["payload"]["data"]["blob"] = Value::String("x".repeat(MAX_FRAME_BYTES - base_length));
    assert_eq!(
        serde_json::to_vec(&maximal).expect("max request").len(),
        MAX_FRAME_BYTES
    );
    let oversized = raw_exchange(&mut stdin, &mut stdout, &maximal).await;
    let fault = serde_json::from_value::<BoundaryResponse>(oversized)
        .expect("valid failure response")
        .into_result()
        .expect_err("response is too large");
    assert_eq!(fault.code, RemoteErrorCode::FrameTooLarge);

    let small = raw_exchange(
        &mut stdin,
        &mut stdout,
        &json!({
            "protocol": PROTOCOL_NAME,
            "version": PROTOCOL_VERSION,
            "requestId": "small-echo",
            "operation": "boundary.echo",
            "payload": {"data": "still-alive"},
        }),
    )
    .await;
    assert!(matches!(
        serde_json::from_value::<BoundaryResponse>(small)
            .expect("valid echo response")
            .into_result(),
        Ok(BoundaryResult::Echo(EchoPayload { data })) if data == "still-alive"
    ));

    child.start_kill().expect("kill worker");
    child.wait().await.expect("reap worker");
}
