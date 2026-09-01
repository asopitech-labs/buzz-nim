//! Supervised process boundary between Rust host adapters and the Nimino core.
//!
//! This crate owns framing, deadlines, cancellation, bounded queuing, and the
//! child-process lifecycle. It deliberately contains no product, database,
//! replication, or cluster-authority policy.

#![deny(missing_docs)]

mod agent;
mod cli;
mod cluster;
mod codec;
mod community;
mod contract;
mod control;
mod dm;
mod effect;
mod error;
mod lease;
mod membership;
mod moderation;
mod object;
mod projection;
mod runtime;
mod sync;
mod workflow;

pub use agent::{
    AgentEventFacts, AgentLifecycleAction, AgentLifecycleCommand, AgentLifecycleRequest,
    AgentLifecycleState, AgentPhase, AgentPolicyError, AgentPolicyRequest, AgentPolicyResult,
    AgentTriggerRule, PersonaBehavior, PersonaTriggers, ResolvedPersonaBehavior,
    ResolvedPersonaTriggers,
};
pub use cli::{
    CliCommandError, CliFailureCategory, CliFailureKind, CliIoMode, CliPolicyOperation,
    CliPolicyRequest, CliPolicyResult,
};
pub use cluster::{
    ClusterLane, ClusterLaneRequest, ClusterLifecycleError, ClusterLifecyclePolicyRequest,
    ClusterLifecyclePolicyResult, ClusterNodeState, LifecycleCommand, LifecycleEffect,
    LifecycleTransitionRequest,
};
pub use community::{
    CommunityAction, CommunityCommand, CommunityLifecycleRequest, CommunityPolicyError,
    CommunityPolicyRequest, CommunityPolicyResult, CommunityScopeRequest, CommunityState,
};
pub use contract::{
    BoundaryFault, BoundaryRequest, BoundaryResponse, BoundaryResult, DeletionAction,
    DeletionRequest, DeletionTargetFacts, EchoPayload, EventDisposition, EventPolicyError,
    EventPolicyRequest, EventPolicyResult, EventVersion, ReactionAction, ReactionRequest,
    ReadyPayload, RemoteErrorCode, ReplacementAction, RetryDisposition, ThreadMetadataFacts,
    ThreadParentFacts, ThreadPlan, ThreadRequest, MAX_FRAME_BYTES, MAX_INFLIGHT, PROTOCOL_NAME,
    PROTOCOL_VERSION, SCHEMA_HASH, WORKER_ROLE,
};
pub use control::{
    ControlAppendRequest, ControlCommitRequest, ControlDecision, ControlEffect,
    ControlElectionRequest, ControlEntry, ControlEntryKind, ControlPlan, ControlPolicyRequest,
    ControlPolicyResult, ControlQuorumDecision, ControlQuorumRequest, ControlRecovery,
    ControlRecoveryInput, ControlReplicationRequest, ControlSnapshotState, ControlState,
    ControlStateError, ControlStoreAction, ControlStoreActionKind, ControlVoteRequest,
    ControlVoterPhase,
};
pub use dm::{
    DmAccessOperation, DmAccessRequest, DmAction, DmCommand, DmMutationRequest, DmPolicyError,
    DmPolicyRequest, DmPolicyResult,
};
pub use effect::{
    EffectLedgerDecision, EffectLedgerEffect, EffectLedgerError, EffectLedgerPlan,
    EffectLedgerPortEffect, EffectLedgerState, EffectLedgerStatus, EffectPolicyRequest,
    EffectPolicyResult, EffectReceipt, EffectReceiptOutcome, EffectReconcileCommand,
    EffectReconcileRequest,
};
pub use error::{BoundaryError, HOST_ERROR_CODES};
pub use lease::{
    ActiveLease, CommittedLeaseFact, LeaseApplyMode, LeaseAuthority, LeaseCommand, LeaseDecision,
    LeaseEffect, LeaseFenceError, LeasePlan, LeasePolicyRequest, LeasePolicyResult, LeaseRoute,
    LeaseState, ServingLeaseFact, SingletonEffectAttempt, SingletonEffectDecision,
};
pub use membership::{
    AgentAddPolicy, ChannelMembershipRequest, ChannelVisibility, InviteCommand,
    InvitePolicyRequest, InviteState, MembershipAction, MembershipCommand, MembershipPolicyError,
    MembershipPolicyRequest, MembershipPolicyResult, MembershipRole, OwnershipTransferRequest,
    RelayMembershipRequest,
};
pub use moderation::{
    ModerationAuditAction, ModerationAuthority, ModerationEffect, ModerationEnforcementOperation,
    ModerationEnforcementRequest, ModerationPolicyError, ModerationPolicyRequest,
    ModerationPolicyResult, ModerationReportRequest, ModerationReportTarget, ModerationReportType,
    ModerationResolutionAction, ModerationResolutionRequest, ModerationResolutionStatus,
    ModerationRestrictionCommand, ModerationRestrictionRequest,
};
pub use object::{
    ObjectDescriptor, ObjectEffect, ObjectFetchAction, ObjectFetchMode, ObjectGcPlan,
    ObjectGcRequest, ObjectKind, ObjectLocalFact, ObjectManifest, ObjectOriginFact,
    ObjectPinDecision, ObjectPinRequest, ObjectPinState, ObjectPolicyError, ObjectPolicyRequest,
    ObjectPolicyResult, ObjectSyncPlan, ObjectSyncRequest,
};
pub use projection::{
    ProjectionBatchPlan, ProjectionBatchRequest, ProjectionBuildState, ProjectionBuildStatus,
    ProjectionCanonicalRecord, ProjectionDecision, ProjectionEffect, ProjectionKind,
    ProjectionLifecycleError, ProjectionPolicyRequest, ProjectionPolicyResult,
    ProjectionPublishPlan, ProjectionRow, ProjectionStageRow, ProjectionStartRequest,
};
pub use runtime::{BoundaryClient, BoundaryConfig, BoundaryRuntime, CallContext};
pub use sync::{
    DigestFrame, InventoryFact, InventoryMergeDecision, InventoryMergeEffect, InventoryMergeError,
    InventoryMergePair, RangeBatchFrame, RangeBatchPlan, RangeReadPlan, RangeRequestFrame,
    SyncCancelFrame, SyncDecision, SyncEffect, SyncEnvelope, SyncPhase, SyncPolicyError,
    SyncPolicyRequest, SyncPolicyResult, SyncRecord, SyncState,
};
pub use workflow::{
    WorkflowAction, WorkflowActionKind, WorkflowDefinition, WorkflowDirective, WorkflowPlanRequest,
    WorkflowPolicyError, WorkflowPolicyRequest, WorkflowPolicyResult, WorkflowPortEffect,
    WorkflowRunState, WorkflowRunStatus, WorkflowStep, WorkflowTransitionCommand,
    WorkflowTransitionRequest, WorkflowTrigger, WorkflowTriggerKind,
};
