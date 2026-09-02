use nimino_boundary::{
    BoundaryFault, BoundaryRequest, BoundaryResponse, BoundaryResult, CliCommandError, CliIoMode,
    CliPolicyOperation, CliPolicyResult, ClusterLifecycleError, ClusterLifecyclePolicyResult,
    ClusterNodeState, CommunityAction, CommunityPolicyError, CommunityPolicyResult,
    ControlPolicyResult, ControlStateError, ControlVoterPhase, DmPolicyError, DmPolicyResult,
    EchoPayload, EffectLedgerEffect, EffectLedgerError, EffectPolicyResult, EventDisposition,
    EventPolicyError, EventPolicyResult, LeaseEffect, LeaseFenceError, LeasePolicyResult,
    LifecycleEffect, MembershipAction, MembershipPolicyError, MembershipPolicyResult,
    MembershipRole, ModerationAuditAction, ModerationAuthority, ModerationEffect,
    ModerationPolicyError, ModerationPolicyResult, ObjectEffect, ObjectPolicyError,
    ObjectPolicyResult, ProjectionEffect, ProjectionLifecycleError, ProjectionPolicyResult,
    RemoteErrorCode, RetryDisposition, SyncEffect, SyncPhase, SyncPolicyError, SyncPolicyResult,
    WorkflowPolicyError, WorkflowPolicyResult, WorkflowPortEffect, WorkflowRunState,
    WorkflowRunStatus, HOST_ERROR_CODES, PROTOCOL_NAME, PROTOCOL_VERSION,
};
use serde_json::json;

#[test]
fn request_fixture_round_trips_without_losing_contract_fields() {
    let fixture =
        include_str!("../../../contracts/nim-rust-boundary/v1/fixtures/echo.request.json");
    let request: BoundaryRequest = serde_json::from_str(fixture).expect("valid request fixture");

    assert_eq!(request.protocol(), PROTOCOL_NAME);
    assert_eq!(request.version(), PROTOCOL_VERSION);
    assert_eq!(request.request_id(), "018f5e5a-9b7d-7c01-a7bb-46fbe46d0001");
    assert_eq!(request.operation_name(), "boundary.echo");
    assert_eq!(
        request.echo_data(),
        Some(&json!({"message": "hello from Rust"}))
    );
}

#[test]
fn community_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/community-policy.request.json"
    ))
    .expect("valid community policy request");
    assert_eq!(request.operation_name(), "domain.community.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/community-policy.response.json"
    ))
    .expect("valid community policy response");
    assert_eq!(
        response.into_result().expect("policy success"),
        BoundaryResult::CommunityPolicy(CommunityPolicyResult::Lifecycle {
            action: CommunityAction::Archive,
            error: CommunityPolicyError::None,
        })
    );
}

#[test]
fn membership_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/membership-policy.request.json"
    ))
    .expect("valid membership policy request");
    assert_eq!(request.operation_name(), "domain.membership.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/membership-policy.response.json"
    ))
    .expect("valid membership policy response");
    assert_eq!(
        response.into_result().expect("policy success"),
        BoundaryResult::MembershipPolicy(MembershipPolicyResult::Relay {
            action: MembershipAction::Insert,
            error: MembershipPolicyError::None,
            effective_role: MembershipRole::Admin,
        })
    );
}

#[test]
fn dm_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/dm-policy.request.json"
    ))
    .expect("valid DM policy request");
    assert_eq!(request.operation_name(), "domain.dm.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/dm-policy.response.json"
    ))
    .expect("valid DM policy response");
    assert_eq!(
        response.into_result().expect("policy success"),
        BoundaryResult::DmPolicy(DmPolicyResult::Access {
            allowed: true,
            error: DmPolicyError::None,
        })
    );
}

#[test]
fn moderation_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/moderation-policy.request.json"
    ))
    .expect("valid moderation policy request");
    assert_eq!(request.operation_name(), "domain.moderation.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/moderation-policy.response.json"
    ))
    .expect("valid moderation policy response");
    assert_eq!(
        response.into_result().expect("policy success"),
        BoundaryResult::ModerationPolicy(ModerationPolicyResult::Restriction {
            effect: ModerationEffect::ApplyTimeout,
            authority: ModerationAuthority::CommunityOwner,
            audit_action: ModerationAuditAction::Timeout,
            error: ModerationPolicyError::None,
        })
    );
}

#[test]
fn workflow_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/workflow-policy.request.json"
    ))
    .expect("valid workflow policy request");
    assert_eq!(request.operation_name(), "domain.workflow.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/workflow-policy.response.json"
    ))
    .expect("valid workflow policy response");
    assert_eq!(
        response.into_result().expect("policy success"),
        BoundaryResult::WorkflowPolicy(WorkflowPolicyResult::Transition {
            allowed: true,
            error: WorkflowPolicyError::None,
            next_state: WorkflowRunState {
                status: WorkflowRunStatus::Running,
                current_step: 1,
                revision: 4,
            },
            port_effect: WorkflowPortEffect::PersistTransition,
        })
    );
}

#[test]
fn cli_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/cli-policy.request.json"
    ))
    .expect("valid CLI policy request");
    assert_eq!(request.operation_name(), "domain.cli.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/cli-policy.response.json"
    ))
    .expect("valid CLI policy response");
    assert_eq!(
        response.into_result().expect("policy success"),
        BoundaryResult::CliPolicy(CliPolicyResult::Command {
            accepted: true,
            error: CliCommandError::None,
            io_mode: CliIoMode::RelayWrite,
            requires_auth: true,
            output_contract: "nimino.cli-output/v1".to_owned(),
            policy_operation: CliPolicyOperation::Workflow,
        })
    );
}

#[test]
fn agent_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/agent-policy.request.json"
    ))
    .expect("agent policy request fixture");
    assert_eq!(request.operation_name(), "domain.agent.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/agent-policy.response.json"
    ))
    .expect("agent policy response fixture");
    assert!(matches!(
        response.into_result().expect("policy success"),
        BoundaryResult::AgentPolicy(_)
    ));
}

#[test]
fn cluster_lifecycle_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/cluster-lifecycle.request.json"
    ))
    .expect("valid cluster lifecycle request");
    assert_eq!(request.operation_name(), "domain.cluster.lifecycle");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/cluster-lifecycle.response.json"
    ))
    .expect("valid cluster lifecycle response");
    assert_eq!(
        response.into_result().expect("policy success"),
        BoundaryResult::ClusterLifecycle(ClusterLifecyclePolicyResult::Transition {
            effect: LifecycleEffect::EnterReady,
            next_state: ClusterNodeState::Ready,
            error: ClusterLifecycleError::None,
        })
    );
}

#[test]
fn control_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/control-policy.request.json"
    ))
    .expect("valid control policy request");
    assert_eq!(request.operation_name(), "domain.control.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/control-policy.response.json"
    ))
    .expect("valid control policy response");
    let BoundaryResult::ControlPolicy(ControlPolicyResult::Recover { result }) =
        response.into_result().expect("control policy success")
    else {
        panic!("unexpected control policy result");
    };
    assert_eq!(result.error, ControlStateError::None);
    assert!(result.state.valid);
    assert_eq!(result.state.phase, ControlVoterPhase::StableOld);
    assert_eq!(result.state.old_voters, ["node-a"]);
}

#[test]
fn lease_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/lease-policy.request.json"
    ))
    .expect("valid lease policy request");
    assert_eq!(request.operation_name(), "domain.lease.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/lease-policy.response.json"
    ))
    .expect("valid lease policy response");
    let BoundaryResult::LeasePolicy(LeasePolicyResult::PlanGrant { result }) =
        response.into_result().expect("lease policy success")
    else {
        panic!("unexpected lease policy result");
    };
    assert_eq!(result.effect, LeaseEffect::Propose);
    assert_eq!(result.error, LeaseFenceError::None);
    assert_eq!(result.command.expect("grant command").owner_id, "node-a");
}

#[test]
fn effect_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/effect-policy.request.json"
    ))
    .expect("valid effect policy request");
    assert_eq!(request.operation_name(), "domain.effect.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/effect-policy.response.json"
    ))
    .expect("valid effect policy response");
    let BoundaryResult::EffectPolicy(EffectPolicyResult::Claim { result }) =
        response.into_result().expect("effect policy success")
    else {
        panic!("unexpected effect policy result");
    };
    assert_eq!(result.effect, EffectLedgerEffect::Claimed);
    assert_eq!(result.error, EffectLedgerError::None);
    assert_eq!(result.next_state.fence_token, 1);
}

#[test]
fn object_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/object-policy.request.json"
    ))
    .expect("valid object policy request");
    assert_eq!(request.operation_name(), "domain.object.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/object-policy.response.json"
    ))
    .expect("valid object policy response");
    let BoundaryResult::ObjectPolicy(ObjectPolicyResult::Sync { result }) =
        response.into_result().expect("object policy success")
    else {
        panic!("unexpected object policy result");
    };
    assert_eq!(result.effect, ObjectEffect::Fetch);
    assert_eq!(result.error, ObjectPolicyError::None);
    assert_eq!(result.actions[0].source_node_id, "node-a");
}

#[test]
fn projection_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/projection-policy.request.json"
    ))
    .expect("valid projection policy request");
    assert_eq!(request.operation_name(), "domain.projection.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/projection-policy.response.json"
    ))
    .expect("valid projection policy response");
    let BoundaryResult::ProjectionPolicy(ProjectionPolicyResult::Batch { result }) =
        response.into_result().expect("projection policy success")
    else {
        panic!("unexpected projection policy result");
    };
    assert_eq!(result.effect, ProjectionEffect::Ready);
    assert_eq!(result.error, ProjectionLifecycleError::None);
    assert_eq!(result.rows[0].key, "event-a");
}

#[test]
fn sync_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/sync-policy.request.json"
    ))
    .expect("valid sync policy request");
    assert_eq!(request.operation_name(), "domain.sync.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/sync-policy.response.json"
    ))
    .expect("valid sync policy response");
    let BoundaryResult::SyncPolicy(SyncPolicyResult::AcceptDigest { result }) =
        response.into_result().expect("sync policy success")
    else {
        panic!("unexpected sync policy result");
    };
    assert_eq!(result.effect, SyncEffect::RequestSnapshot);
    assert_eq!(result.error, SyncPolicyError::None);
    assert_eq!(result.state.phase, SyncPhase::WaitingBatch);
    assert_eq!(result.state.deadline_tick, 15);
}

#[test]
fn event_policy_fixtures_use_the_typed_operation_variant() {
    let request: BoundaryRequest = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/event-policy.request.json"
    ))
    .expect("valid event policy request");
    assert_eq!(request.operation_name(), "domain.event.policy");

    let response: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/event-policy.response.json"
    ))
    .expect("valid event policy response");
    assert_eq!(
        response.into_result().expect("policy success"),
        BoundaryResult::EventPolicy(EventPolicyResult::Classify {
            disposition: EventDisposition::Parameterized,
            error: EventPolicyError::None,
        })
    );
}

#[test]
fn response_fixtures_are_typed_success_or_failure() {
    let success: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/echo.response.json"
    ))
    .expect("valid success fixture");
    assert_eq!(
        success.into_result().expect("success response"),
        BoundaryResult::Echo(EchoPayload {
            data: json!({"message": "hello from Rust"})
        })
    );

    let failure: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/unknown-operation.response.json"
    ))
    .expect("valid failure fixture");
    assert_eq!(
        failure.into_result().expect_err("remote failure"),
        BoundaryFault {
            code: RemoteErrorCode::UnknownOperation,
            message: "operation is not supported".to_owned(),
            retry: RetryDisposition::Never,
        }
    );
}

#[test]
fn malformed_response_metadata_and_fault_bounds_are_rejected() {
    let wrong_version = json!({
        "protocol": PROTOCOL_NAME,
        "version": 2,
        "requestId": "request-1",
        "operation": "boundary.echo",
        "status": "ok",
        "result": {}
    });
    assert!(serde_json::from_value::<BoundaryResponse>(wrong_version).is_err());

    let oversized_message = json!({
        "protocol": PROTOCOL_NAME,
        "version": PROTOCOL_VERSION,
        "requestId": "request-1",
        "operation": "boundary.echo",
        "status": "error",
        "error": {
            "code": "INTERNAL_ERROR",
            "message": "x".repeat(1_025),
            "retry": "idempotent_only"
        }
    });
    assert!(serde_json::from_value::<BoundaryResponse>(oversized_message).is_err());

    let invalid_retry = json!({
        "protocol": PROTOCOL_NAME,
        "version": PROTOCOL_VERSION,
        "requestId": "request-1",
        "operation": "unknown.operation",
        "status": "error",
        "error": {
            "code": "UNKNOWN_OPERATION",
            "message": "not supported",
            "retry": "after_refresh"
        }
    });
    assert!(serde_json::from_value::<BoundaryResponse>(invalid_retry).is_err());
}

#[test]
fn unknown_contract_fields_are_rejected() {
    let malformed = json!({
        "protocol": PROTOCOL_NAME,
        "version": PROTOCOL_VERSION,
        "requestId": "request-1",
        "operation": "boundary.echo",
        "payload": {"data": {}},
        "legacyMode": true
    });

    assert!(serde_json::from_value::<BoundaryRequest>(malformed).is_err());

    let duplicate = r#"{
        "protocol":"nimino.core.boundary",
        "version":1,
        "requestId":"request-1",
        "requestId":"request-2",
        "operation":"boundary.echo",
        "payload":{"data":{}}
    }"#;
    assert!(serde_json::from_str::<BoundaryRequest>(duplicate).is_err());
}

#[test]
fn host_error_inventory_matches_the_versioned_manifest() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/errors.json"
    ))
    .expect("valid error manifest");
    let host_codes: Vec<&str> = manifest["host"]
        .as_array()
        .expect("host error array")
        .iter()
        .map(|entry| entry["code"].as_str().expect("host error code"))
        .collect();
    assert_eq!(host_codes, HOST_ERROR_CODES);
}
