//! Typed rebuild lifecycle for search, thread, and feed projections.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Rebuildable projection kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionKind {
    /// Full-text search rows.
    Search,
    /// Thread counters.
    Thread,
    /// Feed ordering rows.
    Feed,
}

/// Durable lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionBuildStatus {
    /// Rows are being staged.
    Building,
    /// All source rows are staged.
    Ready,
    /// The exact staged epoch was published.
    Published,
    /// The owner cancelled the build.
    Cancelled,
}

/// Policy-selected operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionEffect {
    /// Reject without mutation.
    Reject,
    /// Begin a new epoch.
    Start,
    /// Persist a partial batch.
    Stage,
    /// Persist the final batch.
    Ready,
    /// Publish staged rows.
    Publish,
    /// Cancel an epoch.
    Cancel,
}

/// Stable projection lifecycle rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionLifecycleError {
    /// No error.
    None,
    /// State shape or status is invalid.
    InvalidState,
    /// Community scope differs.
    ScopeMismatch,
    /// Caller is not the epoch owner.
    OwnerMismatch,
    /// Epoch differs.
    EpochMismatch,
    /// Canonical source changed during the build.
    SourceChanged,
    /// Durable revision differs.
    RevisionConflict,
    /// Durable cursor differs.
    CursorConflict,
    /// Batch exceeds bounds or has no progress.
    BatchInvalid,
    /// Canonical source record is invalid.
    RecordInvalid,
    /// Existing staged row is invalid.
    CurrentRowInvalid,
    /// Adapter failed to stage the planned batch.
    StageFailure,
    /// Build is not ready to publish.
    PublishUnavailable,
    /// Adapter failed to publish the planned epoch.
    PublishFailure,
    /// Revision exhausted.
    RevisionOverflow,
}

/// Exact resumable projection identity and cursor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionBuildState {
    /// Whether the state shape is initialized.
    pub valid: bool,
    /// Derived row family.
    pub projection: ProjectionKind,
    /// Tenant scope.
    pub community_id: String,
    /// Fixed canonical checkpoint.
    pub source_checkpoint: u64,
    /// Digest of the fixed canonical prefix.
    pub source_digest: String,
    /// Unique build epoch.
    pub epoch: String,
    /// Sole builder identity.
    pub owner_node_id: String,
    /// Exact durable CAS revision.
    pub revision: u64,
    /// Last inclusive canonical key consumed.
    pub cursor: String,
    /// Lifecycle status.
    pub status: ProjectionBuildStatus,
}

/// Facts required to start one rebuild.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionStartRequest {
    /// Derived row family.
    pub projection: ProjectionKind,
    /// Tenant scope.
    pub community_id: String,
    /// Fixed canonical checkpoint.
    pub source_checkpoint: u64,
    /// Digest of the fixed canonical prefix.
    pub source_digest: String,
    /// Unique build epoch.
    pub epoch: String,
    /// Sole builder identity.
    pub owner_node_id: String,
}

/// One ordered canonical source record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionCanonicalRecord {
    /// Canonical sequence.
    pub sequence: u64,
    /// Canonical record type.
    pub record_type: String,
    /// Stable record key.
    pub key: String,
    /// Tombstone flag.
    pub deleted: bool,
    /// Opaque canonical value interpreted by Nim.
    pub value: Value,
}

/// Existing staged row supplied as a policy fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionRow {
    /// Cache record type.
    pub record_type: String,
    /// Stable row key.
    pub key: String,
    /// Opaque derived value.
    pub value: Value,
}

/// Nim-planned staged row mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionStageRow {
    /// Cache record type.
    pub record_type: String,
    /// Stable row key.
    pub key: String,
    /// True to remove the staged row.
    pub deleted: bool,
    /// Opaque derived value or JSON null for a deletion.
    pub value: Value,
}

/// Exact facts for one bounded staging step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionBatchRequest {
    /// Tenant scope.
    pub community_id: String,
    /// Active build epoch.
    pub epoch: String,
    /// Active owner.
    pub owner_node_id: String,
    /// Required state revision.
    pub expected_revision: u64,
    /// Required inclusive cursor.
    pub expected_cursor: String,
    /// Adapter verification that the fixed checkpoint still matches.
    pub source_checkpoint_matches: bool,
    /// Whether this batch reached source EOF.
    pub complete: bool,
    /// Key-ordered canonical records.
    pub records: Vec<ProjectionCanonicalRecord>,
    /// Current staged rows required for incremental derivation.
    pub current_rows: Vec<ProjectionRow>,
}

/// Nim-owned staging plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionBatchPlan {
    /// Selected effect.
    pub effect: ProjectionEffect,
    /// Stable error.
    pub error: ProjectionLifecycleError,
    /// State before persistence.
    pub before_state: ProjectionBuildState,
    /// State after successful persistence.
    pub next_state: ProjectionBuildState,
    /// Exact staged row mutations.
    pub rows: Vec<ProjectionStageRow>,
}

/// State-only lifecycle result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionDecision {
    /// Selected effect.
    pub effect: ProjectionEffect,
    /// Stable error.
    pub error: ProjectionLifecycleError,
    /// Authoritative state.
    pub state: ProjectionBuildState,
}

/// Exact atomic publish instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectionPublishPlan {
    /// Selected effect.
    pub effect: ProjectionEffect,
    /// Stable error.
    pub error: ProjectionLifecycleError,
    /// State before publish.
    pub before_state: ProjectionBuildState,
    /// State after successful publish.
    pub next_state: ProjectionBuildState,
    /// Idempotent cache replacement intent.
    pub intent_id: String,
    /// Exact cache record type.
    pub record_type: String,
    /// Fixed source checkpoint.
    pub source_checkpoint: u64,
    /// Fixed source digest.
    pub source_digest: String,
}

/// Typed projection lifecycle request.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProjectionPolicyRequest {
    /// Start one epoch.
    Start {
        /// Start facts.
        request: ProjectionStartRequest,
    },
    /// Plan one bounded batch.
    Batch {
        /// Current state.
        state: ProjectionBuildState,
        /// Batch facts.
        request: ProjectionBatchRequest,
    },
    /// Settle staging persistence.
    SettleBatch {
        /// Prior Nim plan.
        plan: ProjectionBatchPlan,
        /// Adapter persistence result.
        stage_succeeded: bool,
    },
    /// Plan atomic publish.
    Publish {
        /// Ready state.
        state: ProjectionBuildState,
        /// Calling owner.
        owner_node_id: String,
    },
    /// Settle atomic publish.
    SettlePublish {
        /// Prior Nim plan.
        plan: ProjectionPublishPlan,
        /// Adapter publish result.
        publish_succeeded: bool,
    },
    /// Cancel an owned epoch.
    Cancel {
        /// Active state.
        state: ProjectionBuildState,
        /// Calling owner.
        owner_node_id: String,
    },
}

/// Typed projection lifecycle result.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ProjectionPolicyResult {
    /// Start decision.
    Start {
        /// Nim-owned decision.
        result: ProjectionDecision,
    },
    /// Batch plan.
    Batch {
        /// Nim-owned plan.
        result: ProjectionBatchPlan,
    },
    /// Stage settlement.
    SettleBatch {
        /// Nim-owned settlement.
        result: ProjectionDecision,
    },
    /// Publish plan.
    Publish {
        /// Nim-owned plan.
        result: ProjectionPublishPlan,
    },
    /// Publish settlement.
    SettlePublish {
        /// Nim-owned settlement.
        result: ProjectionDecision,
    },
    /// Cancel decision.
    Cancel {
        /// Nim-owned decision.
        result: ProjectionDecision,
    },
}
