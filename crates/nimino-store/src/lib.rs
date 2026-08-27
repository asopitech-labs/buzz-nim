//! Per-node durable storage for data intents already validated by the Nimino core.
//!
//! Product classification, authorization, and conflict policy stay in Nim. This
//! crate is the replaceable storage port plus a `redb` adapter for atomic local
//! persistence, crash recovery, and verified backup files.

#![deny(missing_docs)]

mod control_log;
mod redb_store;
mod types;

pub use control_log::{
    ControlLogEntry, ControlLogStorePort, ControlMetadata, ControlSnapshot, RecoveredControlState,
    VersionedControlMetadata, MAX_CONTROL_SNAPSHOT_BYTES,
};
pub use redb_store::RedbNodeStore;
pub use types::{
    CacheReplacement, CanonicalCommit, CommitResult, LogAppend, NodeStorePort, RecordClass,
    RecordWrite, StoreError, StoredRecord, MAX_PAGE_SIZE, MAX_RECORD_BYTES, MAX_TRANSACTION_WRITES,
    SCHEMA_VERSION,
};
