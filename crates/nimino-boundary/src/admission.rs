//! Typed cluster-wide admission policy boundary.

use serde::{Deserialize, Serialize};

/// One committed NIP-98 replay claim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayClaimState {
    /// Community or deployment scope.
    pub scope: String,
    /// Canonical lowercase Nostr event id.
    pub event_id: String,
    /// Inclusive expiry instant in Unix milliseconds.
    pub expires_at_ms: u64,
    /// Control-log index that accepted this claim.
    pub last_control_index: u64,
}

/// Candidate NIP-98 replay claim carried by one committed control entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayClaimCommand {
    /// Community or deployment scope.
    pub scope: String,
    /// Canonical lowercase Nostr event id.
    pub event_id: String,
    /// Adapter-observed Unix time in milliseconds.
    pub observed_at_ms: u64,
    /// Requested replay window in seconds.
    pub ttl_secs: u64,
}

/// Stable replay admission failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdmissionPolicyError {
    /// No error; `allowed` distinguishes an initial claim from a replay.
    None,
    /// Existing replay state is internally inconsistent.
    InvalidState,
    /// Scope is empty or exceeds the v1 bound.
    InvalidScope,
    /// Event id is not canonical lowercase 32-byte hex.
    InvalidEventId,
    /// Observed or derived time is invalid.
    InvalidTime,
    /// Replay TTL falls outside the v1 safety bounds.
    TtlOutOfRange,
    /// The supplied control index does not advance committed state.
    ControlReplay,
    /// Admission namespace is empty or exceeds the v1 bound.
    InvalidNamespace,
    /// Admission key is empty or exceeds the v1 bound.
    InvalidKey,
    /// Fixed-window duration is zero or exceeds the v1 bound.
    InvalidWindow,
    /// A live window was presented with a different policy.
    PolicyConflict,
    /// Adapter time regressed into an already closed window.
    ClockRegression,
    /// The committed counter cannot advance without overflow.
    CounterOverflow,
    /// A rate batch is empty, oversized, or internally inconsistent.
    InvalidBatch,
}

/// Replay claim decision owned by Nimino.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayClaimDecision {
    /// Whether this committed command owns the first live claim.
    pub allowed: bool,
    /// Stable decision error.
    pub error: AdmissionPolicyError,
    /// Next replay state, absent only for invalid input.
    pub state: Option<ReplayClaimState>,
}

/// Deterministic replay prune decision owned by Nimino.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReplayPruneDecision {
    /// Claims whose inclusive expiry has not passed the cutoff.
    pub retained: Vec<ReplayClaimState>,
    /// Stable decision error.
    pub error: AdmissionPolicyError,
}

/// One quorum-owned fixed-window rate counter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RateLimitState {
    /// Policy namespace such as `principal` or `ip`.
    pub namespace: String,
    /// Canonical scoped limiter key.
    pub key: String,
    /// Epoch-aligned window start in Unix milliseconds.
    pub window_started_at_ms: u64,
    /// Window duration in seconds.
    pub window_secs: u64,
    /// Limit fixed for this window.
    pub limit: u64,
    /// Attempts consumed in this window, including denied attempts.
    pub count: u64,
    /// Control-log index of the latest consume.
    pub last_control_index: u64,
}

/// Candidate fixed-window consume carried by one committed control entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RateLimitCommand {
    /// Policy namespace such as `principal` or `ip`.
    pub namespace: String,
    /// Canonical scoped limiter key.
    pub key: String,
    /// Adapter-observed Unix time in milliseconds.
    pub observed_at_ms: u64,
    /// Requested window duration in seconds.
    pub window_secs: u64,
    /// Maximum allowed attempts in the window.
    pub limit: u64,
}

/// Cluster-wide fixed-window admission decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RateLimitDecision {
    /// Whether this attempt remains within the committed budget.
    pub allowed: bool,
    /// Counter value after this attempt.
    pub current: u64,
    /// Limit fixed for this window.
    pub limit: u64,
    /// Whole seconds until the epoch-aligned window rolls over.
    pub reset_in_secs: u64,
    /// Stable decision error.
    pub error: AdmissionPolicyError,
    /// Next durable counter state, absent only for invalid input.
    pub state: Option<RateLimitState>,
}

/// Atomic result for one committed batch of fixed-window consumes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RateLimitBatchDecision {
    /// Per-attempt decisions in request order; empty when the batch is rejected.
    pub results: Vec<RateLimitDecision>,
    /// Stable batch validation or policy error.
    pub error: AdmissionPolicyError,
}

/// Durable authorization surface whose cached/live view must be revalidated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationInvalidationKind {
    /// Community moderation restriction for one principal.
    Ban,
    /// One principal's membership in one channel.
    Membership,
    /// One channel's visibility/access surface.
    Visibility,
    /// One community's active/archive lifecycle.
    Community,
}

/// Latest quorum-committed invalidation revision for one authorization key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizationInvalidationState {
    /// Community scope.
    pub scope: String,
    /// Durable authorization surface.
    pub kind: AuthorizationInvalidationKind,
    /// Principal public key for ban/membership; empty otherwise.
    pub subject: String,
    /// Channel id for membership/visibility; empty otherwise.
    pub channel_id: String,
    /// Stable source fact identifier for replay detection.
    pub fact_id: String,
    /// Monotonic committed control-log index.
    pub revision: u64,
}

/// Candidate authorization invalidation carried by one control entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizationInvalidationCommand {
    /// Community scope.
    pub scope: String,
    /// Durable authorization surface.
    pub kind: AuthorizationInvalidationKind,
    /// Principal public key for ban/membership; empty otherwise.
    pub subject: String,
    /// Channel id for membership/visibility; empty otherwise.
    pub channel_id: String,
    /// Stable source fact identifier for replay detection.
    pub fact_id: String,
}

/// Stable invalidation validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationInvalidationError {
    /// The invalidation is valid.
    None,
    /// Current state is malformed or belongs to another key.
    InvalidState,
    /// Community scope is malformed.
    InvalidScope,
    /// Principal shape does not match the invalidation kind.
    InvalidSubject,
    /// Channel shape does not match the invalidation kind.
    InvalidChannel,
    /// Fact identifier is empty or exceeds the fixed bound.
    InvalidFact,
    /// Revision is zero or conflicts with a committed fact.
    InvalidRevision,
}

/// Nim-owned disposition for one invalidation revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationInvalidationEffect {
    /// Revision advances the projected invalidation state.
    Apply,
    /// Exact committed fact was replayed.
    Replay,
    /// Older revision was ignored.
    Stale,
    /// Facts were invalid.
    Reject,
}

/// Result of applying one authorization invalidation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AuthorizationInvalidationDecision {
    /// Deterministic revision effect.
    pub effect: AuthorizationInvalidationEffect,
    /// Stable validation failure.
    pub error: AuthorizationInvalidationError,
    /// Next projected revision.
    pub state: Option<AuthorizationInvalidationState>,
}

/// Typed cluster admission policy request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AdmissionPolicyRequest {
    /// Apply one quorum-committed replay claim.
    ApplyReplayClaim {
        /// Current state for this scoped event id.
        state: Option<ReplayClaimState>,
        /// Candidate claim facts.
        command: ReplayClaimCommand,
        /// Committed control-log index.
        control_index: u64,
    },
    /// Remove replay claims strictly older than a committed cutoff.
    PruneReplay {
        /// Bounded replay-state batch.
        states: Vec<ReplayClaimState>,
        /// Unix millisecond cutoff; equality remains live.
        before_ms: u64,
    },
    /// Apply one quorum-committed fixed-window consume.
    ApplyRateLimit {
        /// Current counter state for this namespace and key.
        state: Option<RateLimitState>,
        /// Candidate consume facts.
        command: RateLimitCommand,
        /// Committed control-log index.
        control_index: u64,
    },
    /// Atomically apply bounded fixed-window consumes from one control entry.
    ApplyRateLimitBatch {
        /// Current counters for the keys referenced by `commands`.
        states: Vec<RateLimitState>,
        /// Candidate consume facts in their committed order.
        commands: Vec<RateLimitCommand>,
        /// Committed control-log index shared by the batch.
        control_index: u64,
    },
    /// Apply one quorum-committed authorization invalidation.
    ApplyAuthorizationInvalidation {
        /// Current revision for this exact authorization key.
        state: Option<AuthorizationInvalidationState>,
        /// Candidate invalidation facts.
        command: AuthorizationInvalidationCommand,
        /// Committed control-log index used as the monotonic revision.
        revision: u64,
    },
}

/// Typed cluster admission policy result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AdmissionPolicyResult {
    /// Result of applying one replay claim.
    ApplyReplayClaim {
        /// Nim-owned decision.
        result: ReplayClaimDecision,
    },
    /// Result of pruning one bounded state batch.
    PruneReplay {
        /// Nim-owned decision.
        result: ReplayPruneDecision,
    },
    /// Result of one fixed-window consume.
    ApplyRateLimit {
        /// Nim-owned decision.
        result: RateLimitDecision,
    },
    /// Result of one atomic fixed-window consume batch.
    ApplyRateLimitBatch {
        /// Nim-owned decisions.
        result: RateLimitBatchDecision,
    },
    /// Result of one authorization invalidation.
    ApplyAuthorizationInvalidation {
        /// Nim-owned revision decision.
        result: AuthorizationInvalidationDecision,
    },
}
