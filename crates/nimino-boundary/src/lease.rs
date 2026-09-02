//! Typed lease, fencing, and singleton-routing policy boundary.

use serde::{Deserialize, Serialize};

/// Lease application context.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseApplyMode {
    /// A newly committed entry may activate a monotonic lease.
    Live,
    /// Recovery restores fences but never revives an old lease.
    Recovery,
}

/// Effect selected by Nim lease policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseEffect {
    /// Reject without mutation.
    Reject,
    /// Propose the returned command through replicated control.
    Propose,
    /// Activate a committed lease.
    Activate,
    /// Accept an exact idempotent replay.
    Replay,
    /// Route to the selected owner.
    Route,
    /// Authorize the fenced singleton effect.
    Authorize,
}

/// Stable lease and fencing failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseFenceError {
    /// No error.
    None,
    /// Persisted state is invalid.
    InvalidState,
    /// Transition identity is missing.
    TransitionRequired,
    /// Eligible owner facts are invalid.
    InvalidEligibleOwners,
    /// Lease duration is empty.
    LeaseDurationInvalid,
    /// No live quorum is available.
    QuorumUnavailable,
    /// Leader authority is incomplete.
    AuthorityInvalid,
    /// The control entry is not committed.
    ControlNotCommitted,
    /// Term or voter epoch differs from the grant.
    AuthorityStale,
    /// Control index regressed.
    ControlReplay,
    /// Fence is stale.
    StaleFence,
    /// Fence skips a generation.
    FutureFence,
    /// Attempted owner differs from the lease owner.
    OwnerMismatch,
    /// Resource identity differs.
    ResourceMismatch,
    /// No active live lease exists.
    NoActiveLease,
    /// Process monotonic-clock epoch differs.
    ClockEpochMismatch,
    /// Monotonic tick regressed.
    ClockRegression,
    /// Lease elapsed.
    LeaseExpired,
    /// Reused transition identity carries different facts.
    ReplayConflict,
    /// Lease expiry would overflow.
    TickOverflow,
}

/// Current quorum-backed leader facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseAuthority {
    /// Elected leader identity.
    pub leader_id: String,
    /// Election term.
    pub term: u64,
    /// Committed voter epoch.
    pub voter_epoch: u64,
    /// Whether fresh supporters satisfy Nim quorum policy.
    pub quorum_available: bool,
}

/// Replicated lease grant command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseCommand {
    /// Singleton resource identity.
    pub resource_id: String,
    /// Stable idempotency identity.
    pub transition_id: String,
    /// Deterministically selected owner.
    pub owner_id: String,
    /// Normalized owner candidates.
    pub eligible_owners: Vec<String>,
    /// Previous committed fence.
    pub expected_previous_fence: u64,
    /// New fence generation.
    pub fence_token: u64,
    /// Lease duration in adapter monotonic ticks.
    pub duration_ticks: u64,
    /// Granting leader.
    pub leader_id: String,
    /// Granting term.
    pub term: u64,
    /// Granting voter epoch.
    pub voter_epoch: u64,
}

/// One active process-bound lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActiveLease {
    /// Singleton resource identity.
    pub resource_id: String,
    /// Selected owner.
    pub owner_id: String,
    /// Fence generation.
    pub fence_token: u64,
    /// Granting leader.
    pub leader_id: String,
    /// Granting term.
    pub term: u64,
    /// Granting voter epoch.
    pub voter_epoch: u64,
    /// Process clock incarnation.
    pub clock_epoch: String,
    /// Activation tick.
    pub activated_at_tick: u64,
    /// Exclusive expiry tick.
    pub expires_at_tick: u64,
}

/// Durable lease state for one singleton resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseState {
    /// Whether state shape is valid.
    pub valid: bool,
    /// Singleton resource identity.
    pub resource_id: String,
    /// Highest committed fence.
    pub last_fence_token: u64,
    /// Highest applied control index.
    pub last_control_index: u64,
    /// Latest applied grant.
    pub last_command: Option<LeaseCommand>,
    /// Process-local active lease, absent after recovery.
    pub active_lease: Option<ActiveLease>,
}

/// Persistence-free grant plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeasePlan {
    /// Selected effect.
    pub effect: LeaseEffect,
    /// Stable failure.
    pub error: LeaseFenceError,
    /// State retained until control commit.
    pub before_state: LeaseState,
    /// Command to replicate when accepted.
    pub command: Option<LeaseCommand>,
}

/// Facts attached to a committed control entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommittedLeaseFact {
    /// Whether replicated control committed the entry.
    pub committed: bool,
    /// Committed control index.
    pub control_index: u64,
    /// Committing leader.
    pub leader_id: String,
    /// Committing term.
    pub term: u64,
    /// Committing voter epoch.
    pub voter_epoch: u64,
    /// Local process clock incarnation.
    pub clock_epoch: String,
    /// Local monotonic tick.
    pub now_tick: u64,
}

/// Current facts used by routing and singleton-effect gates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServingLeaseFact {
    /// Whether fresh supporters satisfy quorum.
    pub quorum_available: bool,
    /// Current leader.
    pub leader_id: String,
    /// Current term.
    pub term: u64,
    /// Current voter epoch.
    pub voter_epoch: u64,
    /// Local process clock incarnation.
    pub clock_epoch: String,
    /// Local monotonic tick.
    pub now_tick: u64,
}

/// One fenced singleton-effect attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SingletonEffectAttempt {
    /// Singleton resource identity.
    pub resource_id: String,
    /// Claimed owner.
    pub owner_id: String,
    /// Claimed fence generation.
    pub fence_token: u64,
}

/// Lease-state transition result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseDecision {
    /// Selected effect.
    pub effect: LeaseEffect,
    /// Stable failure.
    pub error: LeaseFenceError,
    /// Authoritative next state.
    pub state: LeaseState,
}

/// Singleton route decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LeaseRoute {
    /// Whether routing is allowed.
    pub allowed: bool,
    /// Stable failure.
    pub error: LeaseFenceError,
    /// Selected owner when allowed.
    pub owner_id: String,
    /// Fence generation when allowed.
    pub fence_token: u64,
}

/// Singleton effect authorization result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SingletonEffectDecision {
    /// Whether the side effect is allowed.
    pub allowed: bool,
    /// Stable failure.
    pub error: LeaseFenceError,
}

/// Typed lease policy request.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum LeasePolicyRequest {
    /// Select a grant command without activating it.
    PlanGrant {
        /// Current resource state.
        state: LeaseState,
        /// Current leader authority.
        authority: LeaseAuthority,
        /// Stable transition identity.
        transition_id: String,
        /// Ready owner candidates.
        eligible_owners: Vec<String>,
        /// Lease duration in monotonic ticks.
        duration_ticks: u64,
    },
    /// Apply one committed lease command.
    ApplyCommitted {
        /// Current resource state.
        state: LeaseState,
        /// Committed command.
        command: LeaseCommand,
        /// Replicated-control and clock facts.
        fact: CommittedLeaseFact,
        /// Live or recovery application.
        mode: LeaseApplyMode,
    },
    /// Route one singleton request.
    Route {
        /// Current resource state.
        state: LeaseState,
        /// Current serving facts.
        fact: ServingLeaseFact,
    },
    /// Authorize one fenced singleton side effect.
    Authorize {
        /// Current resource state.
        state: LeaseState,
        /// Claimed fence and owner.
        attempt: SingletonEffectAttempt,
        /// Current serving facts.
        fact: ServingLeaseFact,
    },
}

/// Typed lease policy result.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum LeasePolicyResult {
    /// Grant plan.
    PlanGrant {
        /// Nim-owned plan.
        result: LeasePlan,
    },
    /// Committed application result.
    ApplyCommitted {
        /// Nim-owned next state.
        result: LeaseDecision,
    },
    /// Route result.
    Route {
        /// Nim-owned route.
        result: LeaseRoute,
    },
    /// Effect authorization result.
    Authorize {
        /// Nim-owned authorization.
        result: SingletonEffectDecision,
    },
}
