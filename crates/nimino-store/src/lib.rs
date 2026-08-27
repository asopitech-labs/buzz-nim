//! Per-node durable storage for data intents already validated by the Nimino core.
//!
//! Product classification, authorization, and conflict policy stay in Nim. This
//! crate is the replaceable storage port plus a `redb` adapter for atomic local
//! persistence, crash recovery, and verified backup files.

#![deny(missing_docs)]

mod control_log;
mod projection_stage;
mod redb_store;
mod sync_digest;
mod types;

pub use control_log::{
    ControlLogEntry, ControlLogStorePort, ControlMetadata, ControlSnapshot, RecoveredControlState,
    VersionedControlMetadata, MAX_CONTROL_SNAPSHOT_BYTES,
};
pub use projection_stage::{
    ProjectionStageBatch, ProjectionStageMetadata, ProjectionStageRecovery, ProjectionStageSpec,
};
pub use redb_store::RedbNodeStore;
pub use sync_digest::{
    canonical_prefix_digest, canonical_record_digest, empty_prefix_digest, extend_prefix_digest,
    verify_range_digest, CanonicalPrefixDigest,
};
pub use types::{
    CacheReplacement, CanonicalCommit, CommitResult, LogAppend, NodeStorePort, RecordClass,
    RecordWrite, StoreError, StoredRecord, MAX_PAGE_SIZE, MAX_RECORD_BYTES, MAX_TRANSACTION_WRITES,
    SCHEMA_VERSION,
};
