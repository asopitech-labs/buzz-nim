//! Typed cluster admission and node-lifecycle policy boundary.

use serde::{Deserialize, Serialize};

/// Durable node lifecycle state owned by the Nimino control plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterNodeState {
    /// Not admitted and permitted no cluster lane.
    Offline,
    /// Authenticated and negotiating admission only.
    Joining,
    /// Installing a snapshot and catching up canonical state.
    Syncing,
    /// Eligible for serving and authority decisions.
    Ready,
    /// Refusing new serving/authority work while catching up and draining.
    Draining,
}

/// Requested lifecycle edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleCommand {
    /// Authenticate and enter admission negotiation.
    Join,
    /// Begin state synchronization after a committed admission decision.
    StartSync,
    /// Become ready after synchronized facts and a committed decision.
    MarkReady,
    /// Stop accepting serving and authority work.
    BeginDrain,
    /// Finish a clean drain and become offline.
    MarkOffline,
}

/// Cluster traffic or authority lane subject to lifecycle gating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterLane {
    /// Admission/version negotiation.
    Negotiation,
    /// Replicated control-log traffic.
    Control,
    /// Canonical data synchronization.
    DataSync,
    /// Client reads.
    ClientRead,
    /// Client writes.
    ClientWrite,
    /// Lease or ownership authority.
    Lease,
}

/// Verified facts for one requested lifecycle transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LifecycleTransitionRequest {
    /// Requested edge.
    pub command: LifecycleCommand,
    /// Current durable state.
    pub current_state: ClusterNodeState,
    /// Whether mTLS authenticated the claimed Chirps node identity.
    pub authenticated: bool,
    /// Whether admission policy has revoked that identity.
    pub revoked: bool,
    /// Whether the identity is unambiguously bound to this node.
    pub identity_unique: bool,
    /// Advertised Nimino product capability.
    pub product_capability: String,
    /// Advertised control protocol version.
    pub control_protocol_version: u16,
    /// Advertised data protocol version.
    pub data_protocol_version: u16,
    /// Whether the controlling state-machine decision is committed.
    pub control_decision_committed: bool,
    /// Whether the required snapshot is installed.
    pub snapshot_installed: bool,
    /// Whether the canonical synchronization checkpoint matches.
    pub checkpoint_matches: bool,
    /// Voter epoch required by the committed control state.
    pub required_voter_epoch: u64,
    /// Voter epoch installed by this node.
    pub observed_voter_epoch: u64,
    /// Work still owned by a draining node.
    pub active_work: u32,
}

/// Facts for one lane authorization decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClusterLaneRequest {
    /// Current durable node state.
    pub state: ClusterNodeState,
    /// Requested lane.
    pub lane: ClusterLane,
}

/// Typed cluster-lifecycle policy request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ClusterLifecyclePolicyRequest {
    /// Decide a strict lifecycle edge.
    Transition {
        /// Verified transition facts.
        request: LifecycleTransitionRequest,
    },
    /// Decide whether a state may use a lane.
    Lane {
        /// State and requested lane.
        request: ClusterLaneRequest,
    },
}

/// Port effect selected by the lifecycle policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleEffect {
    /// Reject without persistence or I/O.
    Reject,
    /// Persist joining.
    EnterJoining,
    /// Persist syncing.
    EnterSyncing,
    /// Persist ready.
    EnterReady,
    /// Persist draining.
    EnterDraining,
    /// Persist offline.
    EnterOffline,
    /// Permit the requested lane.
    AllowLane,
    /// Deny the requested lane.
    DenyLane,
}

/// Stable cluster lifecycle policy failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterLifecycleError {
    /// No error.
    None,
    /// mTLS identity was not authenticated.
    Unauthenticated,
    /// Identity was revoked.
    Revoked,
    /// Identity binding was ambiguous or duplicated.
    IdentityConflict,
    /// Product capability was not exactly Nimino v1.
    CapabilityMismatch,
    /// Control protocol version was incompatible.
    ControlVersionMismatch,
    /// Data protocol version was incompatible.
    DataVersionMismatch,
    /// Requested state edge skipped or reversed the lifecycle.
    InvalidTransition,
    /// Required control decision was not committed.
    TransitionUncommitted,
    /// Snapshot or canonical checkpoint synchronization was incomplete.
    SyncIncomplete,
    /// Installed voter epoch did not match committed control state.
    EpochMismatch,
    /// Work or synchronization remained during drain.
    DrainIncomplete,
    /// Current state cannot use the requested lane.
    LaneNotAllowed,
    /// Supplied facts were internally invalid.
    FactConflict,
}

/// Typed lifecycle or lane decision returned by Nimino.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ClusterLifecyclePolicyResult {
    /// Lifecycle transition decision.
    Transition {
        /// Port effect.
        effect: LifecycleEffect,
        /// State to persist, unchanged on rejection.
        next_state: ClusterNodeState,
        /// Stable decision error.
        error: ClusterLifecycleError,
    },
    /// Lane authorization decision.
    Lane {
        /// Allow or deny effect.
        effect: LifecycleEffect,
        /// Unchanged current state.
        next_state: ClusterNodeState,
        /// Stable decision error.
        error: ClusterLifecycleError,
    },
}
