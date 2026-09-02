//! Typed anti-entropy policy boundary owned by the Nimino core.

use serde::{Deserialize, Serialize};

/// Durable phase of one bounded synchronization session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    /// No remote digest has been accepted.
    Idle,
    /// A bounded range batch is expected.
    WaitingBatch,
    /// One approved batch is being committed.
    Applying,
    /// Local and remote checkpoints match.
    Complete,
    /// The session cannot accept more work.
    Cancelled,
}

/// Effect selected by the Nim synchronization policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncEffect {
    /// Reject the supplied facts.
    Reject,
    /// Leave the session unchanged.
    Noop,
    /// Request the next bounded range.
    RequestRange,
    /// Request a bounded logical-state inventory for divergent histories.
    RequestSnapshot,
    /// Commit the approved records at the exact checkpoint.
    ApplyBatch,
    /// A repeated batch needs no second commit.
    AcknowledgeDuplicate,
    /// The session has converged.
    Complete,
    /// Stop the session.
    Cancel,
}

/// Sequence-independent canonical identity fact supplied to Nim convergence policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryFact {
    /// Versioned canonical record type.
    pub record_type: String,
    /// Stable key within the type and community.
    pub key: String,
    /// Whether the record is a permanent tombstone.
    pub deleted: bool,
    /// Lowercase SHA-256 identity derived from `(record_type, key)`.
    pub identity: String,
    /// Lowercase SHA-256 of sequence-independent record content.
    pub content_digest: String,
}

/// Local and incoming facts for one logical canonical key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryMergePair {
    /// Current local record, absent when the key is unknown.
    pub current: Option<InventoryFact>,
    /// Authenticated and digest-verified incoming record.
    pub incoming: InventoryFact,
}

/// Stable logical inventory merge effect selected by Nim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryMergeEffect {
    /// Supplied facts are invalid.
    Reject,
    /// Insert the previously unknown incoming record.
    Insert,
    /// Keep the current record.
    Keep,
    /// Replace the current record.
    Replace,
    /// Current and incoming records are identical.
    Duplicate,
    /// Preserve neither conflicting payload as canonical truth.
    Quarantine,
}

/// Stable logical inventory merge failure selected by Nim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryMergeError {
    /// No error.
    None,
    /// Record identity is invalid.
    IdentityInvalid,
    /// Content digest is invalid.
    DigestInvalid,
    /// Version facts are invalid.
    VersionInvalid,
    /// Community or logical-key scope differs.
    ScopeMismatch,
    /// The same identity names different content.
    IdentityCollision,
    /// Supplied facts contradict each other.
    FactConflict,
    /// Retention facts are invalid.
    RetentionInvalid,
}

/// One indexed logical inventory decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryMergeDecision {
    /// Selected convergence effect.
    pub effect: InventoryMergeEffect,
    /// Stable convergence error.
    pub error: InventoryMergeError,
}

/// Stable synchronization policy failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPolicyError {
    /// No error.
    None,
    /// Durable session facts are invalid.
    InvalidState,
    /// The frame protocol or version differs.
    ProtocolMismatch,
    /// Session, community, or node scope differs.
    ScopeMismatch,
    /// A digest is not lowercase SHA-256.
    DigestInvalid,
    /// Verified digests disagree.
    DigestMismatch,
    /// The selected source is behind the target.
    RemoteBehind,
    /// The requested operation is invalid in this phase.
    PhaseInvalid,
    /// Another batch is already applying.
    Backpressure,
    /// A range exceeds fixed bounds.
    BatchBounds,
    /// Records do not form a contiguous sequence.
    SequenceGap,
    /// The peer exceeded its session deadline.
    PeerTimeout,
    /// The session was cancelled.
    Cancelled,
    /// The storage adapter failed.
    StoreFailure,
    /// The storage adapter committed a different checkpoint.
    StoreCheckpointMismatch,
    /// Settlement does not match the inflight state.
    StaleSettlement,
    /// A cancellation reason is required.
    ReasonRequired,
    /// A monotonic deadline overflowed.
    TickOverflow,
    /// A state revision overflowed.
    RevisionOverflow,
}

/// Scope attached to every synchronization frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncEnvelope {
    /// Fixed `nimino.sync` protocol name.
    pub protocol: String,
    /// Fixed protocol version.
    pub version: u16,
    /// Stable session identifier.
    pub session_id: String,
    /// Mandatory community scope.
    pub community_id: String,
    /// Authenticated sender node.
    pub sender_node_id: String,
    /// Intended receiver node.
    pub receiver_node_id: String,
}

/// Advertised canonical checkpoint and prefix digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DigestFrame {
    /// Frame scope.
    pub envelope: SyncEnvelope,
    /// Source canonical checkpoint.
    pub checkpoint: u64,
    /// Digest through `checkpoint`.
    pub prefix_digest: String,
}

/// Bounded range request selected by Nim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RangeRequestFrame {
    /// Frame scope.
    pub envelope: SyncEnvelope,
    /// Exclusive canonical sequence cursor.
    pub after_checkpoint: u64,
    /// Maximum number of records.
    pub limit_records: u16,
    /// Maximum encoded response bytes.
    pub max_encoded_bytes: u32,
}

/// One canonical record carried by a range batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncRecord {
    /// Canonical sequence.
    pub sequence: u64,
    /// Versioned canonical record type.
    pub record_type: String,
    /// Stable key within the type and community.
    pub key: String,
    /// Whether this record is a tombstone.
    pub deleted: bool,
    /// Canonical JSON encoded as UTF-8 text.
    pub payload: String,
    /// Lowercase SHA-256 of the canonical record.
    pub content_digest: String,
}

/// Verified bounded canonical range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RangeBatchFrame {
    /// Frame scope.
    pub envelope: SyncEnvelope,
    /// Stable idempotency key for this batch.
    pub batch_id: String,
    /// Checkpoint before the first record.
    pub base_checkpoint: u64,
    /// Digest through the base checkpoint.
    pub base_digest: String,
    /// Checkpoint after the last record.
    pub through_checkpoint: u64,
    /// Digest through the final record.
    pub result_digest: String,
    /// Encoded frame byte count.
    pub encoded_bytes: u32,
    /// Rust digest adapter verification result.
    pub digest_verified: bool,
    /// Ordered canonical records.
    pub records: Vec<SyncRecord>,
}

/// Peer-requested cancellation frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncCancelFrame {
    /// Frame scope.
    pub envelope: SyncEnvelope,
    /// Non-empty cancellation reason.
    pub reason: String,
}

/// Durable facts for one synchronization session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncState {
    /// Whether construction passed the Nim invariants.
    pub valid: bool,
    /// Monotonic state revision.
    pub revision: u64,
    /// Current session phase.
    pub phase: SyncPhase,
    /// Stable session identifier.
    pub session_id: String,
    /// Mandatory community scope.
    pub community_id: String,
    /// Local node identity.
    pub local_node_id: String,
    /// Selected remote node identity.
    pub remote_node_id: String,
    /// Durable local canonical checkpoint.
    pub checkpoint: u64,
    /// Digest through the local checkpoint.
    pub checkpoint_digest: String,
    /// Accepted remote checkpoint.
    pub remote_checkpoint: u64,
    /// Accepted remote digest.
    pub remote_digest: String,
    /// Session record bound.
    pub max_records: u16,
    /// Session byte bound.
    pub max_encoded_bytes: u32,
    /// Timeout measured in caller-owned monotonic ticks.
    pub timeout_ticks: u64,
    /// Current monotonic deadline.
    pub deadline_tick: u64,
    /// Batch being committed, or empty.
    pub pending_batch_id: String,
}

/// One terminal or continuing session decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SyncDecision {
    /// Selected effect.
    pub effect: SyncEffect,
    /// Stable error.
    pub error: SyncPolicyError,
    /// Resulting durable state.
    pub state: SyncState,
}

/// Adapter read authorization for one source range.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RangeReadPlan {
    /// Whether the exact bounded read may run.
    pub allowed: bool,
    /// Stable error.
    pub error: SyncPolicyError,
    /// Authorized community.
    pub community_id: String,
    /// Exclusive sequence cursor.
    pub after_checkpoint: u64,
    /// Record bound.
    pub limit_records: u16,
    /// Byte bound.
    pub max_encoded_bytes: u32,
}

/// Two-phase plan for an exact-checkpoint store commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RangeBatchPlan {
    /// Selected effect.
    pub effect: SyncEffect,
    /// Stable error.
    pub error: SyncPolicyError,
    /// State before the batch.
    pub before_state: SyncState,
    /// State to persist while the adapter commits.
    pub inflight_state: SyncState,
    /// State selected after a successful commit.
    pub next_state: SyncState,
    /// Exact local checkpoint precondition.
    pub expected_checkpoint: u64,
    /// Required committed checkpoint.
    pub through_checkpoint: u64,
    /// Records approved for the adapter.
    pub records: Vec<SyncRecord>,
}

/// Typed synchronization policy request.
#[allow(clippy::large_enum_variant)] // Exact wire-schema mirror serialized once per bounded batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SyncPolicyRequest {
    /// Accept a remote prefix digest.
    AcceptDigest {
        /// Current state.
        state: SyncState,
        /// Remote digest frame.
        frame: DigestFrame,
        /// Caller-owned monotonic tick.
        now_tick: u64,
    },
    /// Materialize the next bounded range request.
    NextRange {
        /// Current state.
        state: SyncState,
    },
    /// Authorize a bounded source read.
    PlanRangeRead {
        /// Received range request.
        frame: RangeRequestFrame,
        /// Expected session.
        session_id: String,
        /// Expected community.
        community_id: String,
        /// Local source node.
        source_node_id: String,
        /// Remote target node.
        target_node_id: String,
        /// Durable source checkpoint.
        source_checkpoint: u64,
    },
    /// Validate and plan one received batch.
    PlanBatch {
        /// Current state.
        state: SyncState,
        /// Received range batch.
        frame: RangeBatchFrame,
        /// Caller-owned monotonic tick.
        now_tick: u64,
    },
    /// Settle one exact-checkpoint commit.
    SettleBatch {
        /// Previously returned plan.
        plan: RangeBatchPlan,
        /// Persisted inflight state.
        current_state: SyncState,
        /// Whether the storage adapter succeeded.
        store_succeeded: bool,
        /// Checkpoint reported by the adapter.
        committed_checkpoint: u64,
    },
    /// Cancel locally with an explicit reason.
    Stop {
        /// Current state.
        state: SyncState,
        /// Non-empty reason.
        reason: String,
    },
    /// Accept a peer cancellation.
    Cancel {
        /// Current state.
        state: SyncState,
        /// Received cancel frame.
        frame: SyncCancelFrame,
    },
    /// Apply the caller-owned monotonic deadline.
    CheckDeadline {
        /// Current state.
        state: SyncState,
        /// Caller-owned monotonic tick.
        now_tick: u64,
    },
    /// Merge one bounded logical-state inventory through Nim policy.
    MergeInventory {
        /// Mandatory community scope.
        community_id: String,
        /// Current/incoming pairs in transport order.
        records: Vec<InventoryMergePair>,
    },
}

/// Typed synchronization policy response.
#[allow(clippy::large_enum_variant)] // Exact wire-schema mirror decoded once per bounded batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SyncPolicyResult {
    /// Remote digest decision.
    AcceptDigest {
        /// Resulting decision.
        result: SyncDecision,
    },
    /// Next range frame, absent when state is not waiting.
    NextRange {
        /// Optional range request.
        frame: Option<RangeRequestFrame>,
    },
    /// Source read plan.
    PlanRangeRead {
        /// Resulting plan.
        plan: RangeReadPlan,
    },
    /// Received batch plan.
    PlanBatch {
        /// Resulting plan.
        plan: RangeBatchPlan,
    },
    /// Store settlement decision.
    SettleBatch {
        /// Resulting decision.
        result: SyncDecision,
    },
    /// Local stop decision.
    Stop {
        /// Resulting decision.
        result: SyncDecision,
    },
    /// Peer cancellation decision.
    Cancel {
        /// Resulting decision.
        result: SyncDecision,
    },
    /// Deadline decision.
    CheckDeadline {
        /// Resulting decision.
        result: SyncDecision,
    },
    /// Indexed logical-state merge decisions.
    MergeInventory {
        /// One decision for each request pair.
        results: Vec<InventoryMergeDecision>,
    },
}
