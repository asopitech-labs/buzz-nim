//! Supervised process boundary between Rust host adapters and the Nimino core.
//!
//! This crate owns framing, deadlines, cancellation, bounded queuing, and the
//! child-process lifecycle. It deliberately contains no product, database,
//! replication, or cluster-authority policy.

#![deny(missing_docs)]

mod codec;
mod community;
mod contract;
mod error;
mod runtime;

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
pub use error::{BoundaryError, HOST_ERROR_CODES};
pub use runtime::{BoundaryClient, BoundaryConfig, BoundaryRuntime, CallContext};
