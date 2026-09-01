use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// On-disk schema understood by this adapter.
pub const SCHEMA_VERSION: u64 = 2;
/// Largest accepted serialized JSON record.
pub const MAX_RECORD_BYTES: usize = 1_048_576;
/// Largest atomic write batch accepted by the adapter.
pub const MAX_TRANSACTION_WRITES: usize = 1_000;
/// Largest page or change-feed request accepted by the adapter.
pub const MAX_PAGE_SIZE: usize = 1_000;

/// Physically separated storage lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecordClass {
    /// Durable product or lifecycle truth.
    Canonical,
    /// Replaceable derived state.
    Cache,
    /// Append-only operational evidence.
    Log,
}

/// One validated record mutation supplied by the Nimino core.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordWrite {
    /// Versioned Nimino record type.
    pub record_type: String,
    /// Stable record key within its type and community.
    pub key: String,
    /// Whether this canonical mutation is a tombstone.
    pub deleted: bool,
    /// JSON payload; tombstones carry JSON null.
    pub value: Value,
}

/// Optimistic atomic mutation of canonical state.
#[derive(Clone, Debug, Serialize)]
pub struct CanonicalCommit {
    /// Stable idempotency key.
    pub intent_id: String,
    /// Mandatory community scope.
    pub community_id: String,
    /// Exact canonical checkpoint required before applying.
    pub expected_checkpoint: u64,
    /// Canonical records committed together.
    pub writes: Vec<RecordWrite>,
}

/// Atomic replacement of one cache record type.
#[derive(Clone, Debug, Serialize)]
pub struct CacheReplacement {
    /// Stable idempotency key.
    pub intent_id: String,
    /// Mandatory community scope.
    pub community_id: String,
    /// Exact canonical checkpoint from which the cache was derived.
    pub source_checkpoint: u64,
    /// Cache record type being replaced, including when `rows` is empty.
    pub record_type: String,
    /// Complete replacement rows for the record type.
    pub rows: Vec<RecordWrite>,
}

/// Atomic append of operational evidence.
#[derive(Clone, Debug, Serialize)]
pub struct LogAppend {
    /// Stable idempotency key.
    pub intent_id: String,
    /// Mandatory community scope.
    pub community_id: String,
    /// Append-only log entries.
    pub entries: Vec<RecordWrite>,
}

/// Result of an idempotent write intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommitResult {
    /// Canonical checkpoint or final class-local sequence.
    pub checkpoint: u64,
    /// `false` when the exact intent was already durable.
    pub applied: bool,
}

/// Stored record returned by typed queries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoredRecord {
    /// Monotonic canonical or log sequence, or cache source checkpoint.
    pub sequence: u64,
    /// Versioned Nimino record type.
    pub record_type: String,
    /// Stable key within its type and community.
    pub key: String,
    /// Whether this canonical record is a tombstone.
    pub deleted: bool,
    /// JSON record payload.
    pub value: Value,
}

/// Durable store failure with conflicts kept distinct from engine failures.
#[derive(Debug, Error)]
pub enum StoreError {
    /// The adapter rejected malformed or unbounded input.
    #[error("invalid store input: {0}")]
    InvalidInput(&'static str),
    /// The canonical checkpoint did not match the intent precondition.
    #[error("canonical checkpoint conflict: expected {expected}, actual {actual}")]
    CheckpointConflict {
        /// Checkpoint supplied by the core.
        expected: u64,
        /// Current durable checkpoint.
        actual: u64,
    },
    /// An idempotency key was reused for different content.
    #[error("intent id was reused with different content")]
    IntentConflict,
    /// Atomic control metadata compare-and-set observed another revision.
    #[error("control metadata conflict: expected revision {expected}, actual {actual}")]
    ControlMetadataConflict {
        /// Revision supplied by the caller.
        expected: u64,
        /// Current durable revision.
        actual: u64,
    },
    /// A control append would create a non-contiguous prefix.
    #[error("control log gap: expected previous/index {expected}, actual {actual}")]
    ControlLogGap {
        /// Required previous or entry index.
        expected: u64,
        /// Supplied previous or entry index.
        actual: u64,
    },
    /// A control suffix replacement attempted to rewrite committed entries.
    #[error("cannot replace committed control prefix through index {committed}")]
    CommittedControlPrefix {
        /// Highest committed index that must remain unchanged.
        committed: u64,
    },
    /// A control append referenced state already removed by a snapshot.
    #[error("control log is compacted through snapshot index {snapshot}")]
    CompactedControlLog {
        /// Latest installed snapshot index.
        snapshot: u64,
    },
    /// An older control snapshot cannot replace a newer installed snapshot.
    #[error("control snapshot regressed from {current} to {incoming}")]
    ControlSnapshotRegression {
        /// Current snapshot index.
        current: u64,
        /// Supplied snapshot index.
        incoming: u64,
    },
    /// The same snapshot index was reused for different content.
    #[error("control snapshot at index {index} conflicts with installed content")]
    ControlSnapshotConflict {
        /// Conflicting snapshot index.
        index: u64,
    },
    /// Durable control state violates the local prefix/recovery shape.
    #[error("corrupt control store: {0}")]
    CorruptControlState(&'static str),
    /// The canonical change feed disagrees with its durable checkpoint.
    #[error("corrupt canonical changes: {0}")]
    CorruptCanonicalChanges(&'static str),
    /// A bounded sync digest scan was cancelled by its caller.
    #[error("sync digest scan cancelled")]
    SyncCancelled,
    /// Another projection epoch already owns this community/projection stage.
    #[error("projection stage conflicts with the active epoch")]
    ProjectionStageConflict,
    /// No matching projection stage exists.
    #[error("projection stage is missing")]
    ProjectionStageMissing,
    /// Projection stage metadata revision compare-and-set failed.
    #[error("projection stage revision conflict: expected {expected}, actual {actual}")]
    ProjectionStageRevisionConflict {
        /// Revision supplied by the caller.
        expected: u64,
        /// Durable current revision.
        actual: u64,
    },
    /// Projection stage cursor compare-and-set failed.
    #[error("projection stage cursor conflict")]
    ProjectionStageCursorConflict,
    /// A completed stage cannot accept more rows.
    #[error("projection stage is already complete")]
    ProjectionStageComplete,
    /// A derived projection consumer lost its durable checkpoint CAS.
    #[error("projection checkpoint conflict: expected {expected}, actual {actual}")]
    ProjectionCheckpointConflict {
        /// Checkpoint supplied by the consumer.
        expected: u64,
        /// Current durable consumer checkpoint.
        actual: u64,
    },
    /// Append-only storage already contains the supplied typed key.
    #[error("append-only log key already exists")]
    DuplicateLogKey,
    /// The database schema cannot be opened by this build.
    #[error("unsupported store schema {found}; this build supports {supported}")]
    UnsupportedSchema {
        /// Version found in the database.
        found: u64,
        /// Version supported by this adapter.
        supported: u64,
    },
    /// Backup and restore never overwrite an existing target.
    #[error("backup or restore target already exists")]
    TargetExists,
    /// The local database lock was poisoned.
    #[error("local store lock was poisoned")]
    LockPoisoned,
    /// The embedded engine rejected an operation.
    #[error("embedded store error: {0}")]
    Engine(String),
    /// A filesystem operation failed.
    #[error("store filesystem error: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialization or decoding failed.
    #[error("store record encoding error: {0}")]
    Encoding(#[from] serde_json::Error),
}

/// Swappable port for the per-node canonical, cache, and log store.
pub trait NodeStorePort: Send + Sync {
    /// Returns the durable canonical checkpoint for one community.
    fn canonical_checkpoint(&self, community_id: &str) -> Result<u64, StoreError>;

    /// Atomically commits canonical state using exact-checkpoint CAS.
    fn commit_canonical(&self, intent: CanonicalCommit) -> Result<CommitResult, StoreError>;

    /// Atomically replaces one cache record type at an exact source checkpoint.
    fn replace_cache(&self, intent: CacheReplacement) -> Result<CommitResult, StoreError>;

    /// Atomically appends operational evidence.
    fn append_log(&self, intent: LogAppend) -> Result<CommitResult, StoreError>;

    /// Reads one exact typed record.
    fn get(
        &self,
        class: RecordClass,
        community_id: &str,
        record_type: &str,
        key: &str,
    ) -> Result<Option<StoredRecord>, StoreError>;

    /// Reads a deterministic bounded page after an optional exclusive key cursor.
    fn page(
        &self,
        class: RecordClass,
        community_id: &str,
        record_type: &str,
        after_key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredRecord>, StoreError>;

    /// Reads canonical changes strictly after `after_sequence`.
    fn changes(
        &self,
        community_id: &str,
        after_sequence: u64,
        limit: usize,
    ) -> Result<Vec<StoredRecord>, StoreError>;

    /// Reads the current canonical state across all record types in key order.
    fn canonical_page(
        &self,
        community_id: &str,
        after: Option<(&str, &str)>,
        limit: usize,
    ) -> Result<Vec<StoredRecord>, StoreError>;

    /// Returns the last canonical change applied by a local derived projection.
    fn projection_checkpoint(
        &self,
        community_id: &str,
        projection: &str,
    ) -> Result<u64, StoreError>;

    /// Atomically advances a local derived projection checkpoint.
    fn advance_projection_checkpoint(
        &self,
        community_id: &str,
        projection: &str,
        expected_checkpoint: u64,
        next_checkpoint: u64,
    ) -> Result<(), StoreError>;

    /// Creates a schema-verified backup with atomic no-clobber installation.
    fn backup_to(&self, destination: &Path) -> Result<(), StoreError>;
}
