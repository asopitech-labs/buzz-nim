//! Supervised process boundary between Rust host adapters and the Nimino core.
//!
//! This crate owns framing, deadlines, cancellation, bounded queuing, and the
//! child-process lifecycle. It deliberately contains no product, database,
//! replication, or cluster-authority policy.

#![deny(missing_docs)]

mod cli;
mod cluster;
mod codec;
mod community;
mod contract;
mod dm;
mod error;
mod membership;
mod moderation;
mod runtime;
mod workflow;

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
pub use dm::{
    DmAccessOperation, DmAccessRequest, DmAction, DmCommand, DmMutationRequest, DmPolicyError,
    DmPolicyRequest, DmPolicyResult,
};
pub use error::{BoundaryError, HOST_ERROR_CODES};
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
pub use runtime::{BoundaryClient, BoundaryConfig, BoundaryRuntime, CallContext};
pub use workflow::{
    WorkflowAction, WorkflowActionKind, WorkflowDefinition, WorkflowDirective, WorkflowPlanRequest,
    WorkflowPolicyError, WorkflowPolicyRequest, WorkflowPolicyResult, WorkflowPortEffect,
    WorkflowRunState, WorkflowRunStatus, WorkflowStep, WorkflowTransitionCommand,
    WorkflowTransitionRequest, WorkflowTrigger, WorkflowTriggerKind,
};
