//! Typed cluster presence and typing convergence policy boundary.

use serde::{Deserialize, Serialize};

/// Ephemeral product state carried over the Chirps message lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EphemeralKind {
    /// One user's community-wide presence status.
    Presence,
    /// One user's channel-scoped typing status.
    Typing,
}

/// One converged presence or typing value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EphemeralState {
    /// Community scope.
    pub scope: String,
    /// Ephemeral state family.
    pub kind: EphemeralKind,
    /// Canonical lowercase subject public key.
    pub subject: String,
    /// Empty for presence; channel id for typing.
    pub context: String,
    /// Presence/typing value; empty for a tombstone.
    pub value: String,
    /// Whether the value is live rather than a tombstone.
    pub active: bool,
    /// Origin-observed Unix time in milliseconds.
    pub observed_at_ms: u64,
    /// Inclusive expiry instant in Unix milliseconds.
    pub expires_at_ms: u64,
    /// Authenticated origin Chirps node id.
    pub origin_node_id: String,
    /// Stable transition id used for replay detection and total ordering.
    pub transition_id: String,
}

/// Candidate presence or typing transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EphemeralCommand {
    /// Community scope.
    pub scope: String,
    /// Ephemeral state family.
    pub kind: EphemeralKind,
    /// Canonical lowercase subject public key.
    pub subject: String,
    /// Empty for presence; channel id for typing.
    pub context: String,
    /// Presence/typing value; empty for a tombstone.
    pub value: String,
    /// Whether the value is live rather than a tombstone.
    pub active: bool,
    /// Origin-observed Unix time in milliseconds.
    pub observed_at_ms: u64,
    /// Transition lifetime in seconds.
    pub ttl_secs: u64,
    /// Authenticated origin Chirps node id.
    pub origin_node_id: String,
    /// Stable transition id.
    pub transition_id: String,
}

/// Stable ephemeral policy validation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EphemeralPolicyError {
    /// The transition is valid.
    None,
    /// Existing projected state is malformed or belongs to another key.
    InvalidState,
    /// Community scope is malformed.
    InvalidScope,
    /// Subject public key is malformed.
    InvalidSubject,
    /// Presence/typing context is malformed.
    InvalidContext,
    /// Value or tombstone shape is invalid.
    InvalidValue,
    /// Observed time or derived expiry is invalid.
    InvalidTime,
    /// Transition TTL falls outside the fixed v1 bound.
    TtlOutOfRange,
    /// Origin node id is malformed.
    InvalidOrigin,
    /// Transition id is malformed.
    InvalidTransition,
}

/// Nim-owned disposition for one transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EphemeralEffect {
    /// Candidate replaced the projected value.
    Apply,
    /// Candidate exactly repeated the projected value.
    Replay,
    /// Candidate was older than the projected value.
    Stale,
    /// Candidate had already expired at receipt time.
    Expired,
    /// Candidate or current state was invalid.
    Reject,
}

/// Result of applying one presence or typing transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EphemeralDecision {
    /// Deterministic state-machine effect.
    pub effect: EphemeralEffect,
    /// Stable validation failure.
    pub error: EphemeralPolicyError,
    /// Next live projection, absent after expiry or rejection without state.
    pub state: Option<EphemeralState>,
}

/// Result of pruning one bounded projection batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EphemeralPruneDecision {
    /// States whose inclusive expiry has not passed the cutoff.
    pub retained: Vec<EphemeralState>,
    /// Stable validation failure.
    pub error: EphemeralPolicyError,
}

/// Typed cluster ephemeral policy request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)] // Exact wire-schema mirror serialized once per transition.
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum EphemeralPolicyRequest {
    /// Apply one local or authenticated remote transition.
    Apply {
        /// Current state for the scoped key.
        state: Option<EphemeralState>,
        /// Candidate transition facts.
        command: EphemeralCommand,
        /// Adapter receipt time in Unix milliseconds.
        now_ms: u64,
    },
    /// Remove states strictly older than a cutoff.
    Prune {
        /// Bounded projection batch.
        states: Vec<EphemeralState>,
        /// Unix millisecond cutoff; equality remains live.
        before_ms: u64,
    },
}

/// Typed cluster ephemeral policy result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum EphemeralPolicyResult {
    /// Result of one transition.
    Apply {
        /// Nim-owned decision.
        result: EphemeralDecision,
    },
    /// Result of one bounded prune batch.
    Prune {
        /// Nim-owned decision.
        result: EphemeralPruneDecision,
    },
}
