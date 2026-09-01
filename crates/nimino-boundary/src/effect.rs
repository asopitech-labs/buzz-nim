//! Typed workflow effect-ledger policy boundary.

use serde::{Deserialize, Serialize};

use crate::{LeaseFenceError, LeaseState, ServingLeaseFact};

/// Durable workflow effect state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectLedgerStatus {
    /// Eligible for a fenced claim.
    Pending,
    /// Claimed but not yet marked executing.
    Claimed,
    /// External execution may have started.
    Executing,
    /// A success receipt is durable.
    Succeeded,
    /// A failure receipt is durable.
    Failed,
    /// Execution outcome is unknown and requires an operator.
    Unknown,
}

/// External receipt outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectReceiptOutcome {
    /// The external effect succeeded.
    Succeeded,
    /// The external effect failed.
    Failed,
}

/// Durable external-effect receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectReceipt {
    /// Terminal outcome.
    pub outcome: EffectReceiptOutcome,
    /// Adapter-provided receipt identity.
    pub receipt_id: String,
    /// Lowercase SHA-256 of the observed result.
    pub result_digest: String,
}

/// Canonical state for one workflow effect identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectLedgerState {
    /// Whether the state shape is valid.
    pub valid: bool,
    /// Community identity.
    pub community_id: String,
    /// Workflow identity.
    pub workflow_id: String,
    /// Run identity.
    pub run_id: String,
    /// Step identity.
    pub step_id: String,
    /// Stable external idempotency key.
    pub idempotency_key: String,
    /// Lowercase SHA-256 of the resolved effect.
    pub effect_digest: String,
    /// Singleton lease resource.
    pub lease_resource_id: String,
    /// Monotonic state revision.
    pub revision: u64,
    /// Manual/automatic attempt generation.
    pub attempt: u32,
    /// Current lifecycle status.
    pub status: EffectLedgerStatus,
    /// Claimed owner.
    pub owner_node_id: String,
    /// Claimed fence.
    pub fence_token: u64,
    /// Terminal receipt, if any.
    pub receipt: Option<EffectReceipt>,
    /// Operator who reconciled an unknown result.
    pub reconciled_by: String,
    /// Operator reconciliation reason.
    pub reconcile_reason: String,
}

/// Stable effect-ledger failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectLedgerError {
    /// No error.
    None,
    /// Persisted state is invalid.
    InvalidState,
    /// Effect identity is invalid.
    InvalidIdentity,
    /// Receipt is invalid.
    InvalidReceipt,
    /// Lease policy denied authority.
    LeaseRejected,
    /// Claimed owner differs.
    OwnerMismatch,
    /// Claimed fence differs.
    FenceMismatch,
    /// Another claim owns the effect.
    ClaimConflict,
    /// Unknown outcome requires manual reconciliation.
    ManualReconcileRequired,
    /// Operator lacks reconciliation authority.
    ReconcileUnauthorized,
    /// Reconciliation reason is missing.
    ReconcileReasonRequired,
    /// Terminal state conflicts with the receipt.
    TerminalConflict,
    /// Canonical persistence failed.
    PersistenceFailure,
    /// Revision exhausted.
    RevisionOverflow,
    /// Attempt generation exhausted.
    AttemptOverflow,
}

/// Host action requested by an effect plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectLedgerPortEffect {
    /// No persistence.
    None,
    /// Commit the returned state to canonical storage.
    CommitCanonical,
}

/// Effect-ledger transition selected by Nim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectLedgerEffect {
    /// Reject without mutation.
    Reject,
    /// Accept an exact replay.
    Replay,
    /// Persist a claim.
    Claimed,
    /// Persist the execution marker before external I/O.
    ExecuteExternal,
    /// Persist a terminal receipt.
    ReceiptRecorded,
    /// Release a claim that lost authority.
    ClaimRecovered,
    /// Mark an executing effect unknown after recovery.
    Unknown,
    /// Return an unknown effect to pending by operator command.
    ManualRetry,
    /// Settle an unknown effect from an operator-supplied receipt.
    Reconciled,
}

/// Persistence-free effect transition plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectLedgerPlan {
    /// Selected transition.
    pub effect: EffectLedgerEffect,
    /// Stable ledger error.
    pub error: EffectLedgerError,
    /// Nested lease error.
    pub lease_error: LeaseFenceError,
    /// Required host action.
    pub port_effect: EffectLedgerPortEffect,
    /// State retained on failure.
    pub before_state: EffectLedgerState,
    /// State persisted on success.
    pub next_state: EffectLedgerState,
}

/// Settled effect transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectLedgerDecision {
    /// Settled transition.
    pub effect: EffectLedgerEffect,
    /// Stable ledger error.
    pub error: EffectLedgerError,
    /// Nested lease error.
    pub lease_error: LeaseFenceError,
    /// Authoritative state.
    pub state: EffectLedgerState,
}

/// Manual unknown-result command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectReconcileCommand {
    /// Record a success receipt.
    MarkSucceeded,
    /// Record a failure receipt.
    MarkFailed,
    /// Permit a new fenced attempt.
    Retry,
}

/// Authorized manual reconciliation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectReconcileRequest {
    /// Whether the adapter verified operator authority.
    pub operator_authorized: bool,
    /// Stable operator identity.
    pub operator_id: String,
    /// Human audit reason.
    pub reason: String,
    /// Requested resolution.
    pub command: EffectReconcileCommand,
    /// Required terminal receipt or absent for retry.
    pub receipt: Option<EffectReceipt>,
}

/// Typed effect-ledger policy request.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum EffectPolicyRequest {
    /// Plan a fenced claim.
    Claim {
        /// Current state.
        state: EffectLedgerState,
        /// Claimed owner.
        owner_node_id: String,
        /// Claimed fence.
        fence_token: u64,
        /// Current lease state.
        lease_state: LeaseState,
        /// Current serving facts.
        fact: ServingLeaseFact,
    },
    /// Plan the persisted execution marker.
    Execute {
        /// Current state.
        state: EffectLedgerState,
        /// Claimed owner.
        owner_node_id: String,
        /// Claimed fence.
        fence_token: u64,
        /// Current lease state.
        lease_state: LeaseState,
        /// Current serving facts.
        fact: ServingLeaseFact,
    },
    /// Plan a terminal receipt.
    Receipt {
        /// Current state.
        state: EffectLedgerState,
        /// Claimed owner.
        owner_node_id: String,
        /// Claimed fence.
        fence_token: u64,
        /// External receipt.
        receipt: EffectReceipt,
    },
    /// Plan crash recovery.
    Recover {
        /// Current state.
        state: EffectLedgerState,
        /// Current lease state.
        lease_state: LeaseState,
        /// Current serving facts.
        fact: ServingLeaseFact,
    },
    /// Plan an operator reconciliation.
    Reconcile {
        /// Current state.
        state: EffectLedgerState,
        /// Authorized request.
        request: EffectReconcileRequest,
    },
    /// Settle a plan after its required store action.
    Settle {
        /// Exact plan.
        plan: EffectLedgerPlan,
        /// Whether canonical persistence succeeded.
        persistence_succeeded: bool,
    },
}

/// Typed effect-ledger policy result.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum EffectPolicyResult {
    /// Claim plan.
    Claim {
        /// Nim-owned plan.
        result: EffectLedgerPlan,
    },
    /// Execution-marker plan.
    Execute {
        /// Nim-owned plan.
        result: EffectLedgerPlan,
    },
    /// Receipt plan.
    Receipt {
        /// Nim-owned plan.
        result: EffectLedgerPlan,
    },
    /// Recovery plan.
    Recover {
        /// Nim-owned plan.
        result: EffectLedgerPlan,
    },
    /// Reconciliation plan.
    Reconcile {
        /// Nim-owned plan.
        result: EffectLedgerPlan,
    },
    /// Settled transition.
    Settle {
        /// Authoritative decision.
        result: EffectLedgerDecision,
    },
}
