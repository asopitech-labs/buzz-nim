use std::ops::Bound;

use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::{
    redb_store::engine, RedbNodeStore, StoreError, MAX_RECORD_BYTES, MAX_TRANSACTION_WRITES,
};

const CONTROL_METADATA: TableDefinition<&str, &[u8]> =
    TableDefinition::new("nimino_control_metadata_v2");
const CONTROL_LOG: TableDefinition<u64, &[u8]> = TableDefinition::new("nimino_control_log_v2");
const CONTROL_SNAPSHOT: TableDefinition<&str, &[u8]> =
    TableDefinition::new("nimino_control_snapshot_v2");
const STATE_KEY: &str = "state";
const SNAPSHOT_KEY: &str = "latest";

/// Largest opaque state-machine snapshot accepted by the local adapter.
pub const MAX_CONTROL_SNAPSHOT_BYTES: usize = 64 * 1_048_576;

/// One opaque, ordered control-log entry produced by the Nim control plane.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlLogEntry {
    /// Contiguous one-based log index.
    pub index: u64,
    /// Election term attached by the control plane.
    pub term: u64,
    /// Voter epoch attached by the control plane.
    pub voter_epoch: u64,
    /// Versioned entry kind interpreted by the Nim state machine.
    pub kind: String,
    /// Stable idempotency identity interpreted by Nim.
    pub command_id: String,
    /// Opaque encoded state-machine command.
    pub payload: Vec<u8>,
    /// Replacement voter set for a membership transition.
    pub target_voters: Vec<String>,
}

/// Atomically persisted election and replay watermarks.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlMetadata {
    /// Current election term.
    pub term: u64,
    /// Node voted for in the current term, if any.
    pub voted_for: Option<String>,
    /// Highest quorum-committed log index.
    pub commit_index: u64,
    /// Highest state-machine-applied log index.
    pub applied_index: u64,
}

/// Control metadata plus its compare-and-set revision.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct VersionedControlMetadata {
    /// Monotonic local storage revision.
    pub revision: u64,
    /// Atomically stored metadata.
    pub state: ControlMetadata,
}

/// Opaque state-machine snapshot with the authority metadata required by v1.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ControlSnapshot {
    /// Last state-machine index included in `state`.
    pub last_included_index: u64,
    /// Term at `last_included_index`.
    pub last_included_term: u64,
    /// Voter epoch after applying the included prefix.
    pub voter_epoch: u64,
    /// Voter phase after applying the included prefix.
    pub voter_phase: String,
    /// Old/stable voters at the included index.
    pub old_voters: Vec<String>,
    /// Joint/new voters at the included index.
    pub new_voters: Vec<String>,
    /// Opaque snapshot bytes interpreted only by the Nim state machine.
    pub state: Vec<u8>,
}

/// Complete local state used to recover the control plane.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveredControlState {
    /// Latest atomic term/vote/commit/apply metadata.
    pub metadata: VersionedControlMetadata,
    /// Latest installed snapshot, if any.
    pub snapshot: Option<ControlSnapshot>,
    /// Contiguous suffix strictly after the snapshot.
    pub entries: Vec<ControlLogEntry>,
}

/// Storage-only port for the replicated Nimino control log.
pub trait ControlLogStorePort: Send + Sync {
    /// Loads and validates metadata, snapshot, and the contiguous log suffix.
    fn recover_control_state(&self) -> Result<RecoveredControlState, StoreError>;

    /// Atomically replaces metadata if its local revision matches.
    fn compare_and_set_control_metadata(
        &self,
        expected_revision: u64,
        state: ControlMetadata,
    ) -> Result<VersionedControlMetadata, StoreError>;

    /// Atomically truncates the uncommitted suffix after `previous_index` and appends entries.
    fn replace_control_suffix(
        &self,
        previous_index: u64,
        entries: Vec<ControlLogEntry>,
    ) -> Result<u64, StoreError>;

    /// Atomically installs a snapshot and compacts its covered prefix.
    fn install_control_snapshot(
        &self,
        expected_metadata_revision: u64,
        snapshot: ControlSnapshot,
    ) -> Result<bool, StoreError>;
}

impl ControlLogStorePort for RedbNodeStore {
    fn recover_control_state(&self) -> Result<RecoveredControlState, StoreError> {
        let database = self.database()?;
        let transaction = database.begin_read().map_err(engine)?;
        let metadata = read_metadata_read(&transaction)?;
        let snapshot = read_snapshot_read(&transaction)?;
        let log = transaction.open_table(CONTROL_LOG).map_err(engine)?;
        let mut entries = Vec::new();
        for row in log.iter().map_err(engine)? {
            let (index, value) = row.map_err(engine)?;
            let entry: ControlLogEntry = serde_json::from_slice(value.value())?;
            if entry.index != index.value() {
                return Err(StoreError::CorruptControlState(
                    "control entry key and encoded index differ",
                ));
            }
            entries.push(entry);
        }
        validate_recovery(&metadata, snapshot.as_ref(), &entries)?;
        Ok(RecoveredControlState {
            metadata,
            snapshot,
            entries,
        })
    }

    fn compare_and_set_control_metadata(
        &self,
        expected_revision: u64,
        state: ControlMetadata,
    ) -> Result<VersionedControlMetadata, StoreError> {
        validate_metadata_components(&state)?;
        let database = self.database()?;
        let mut transaction = database.begin_write().map_err(engine)?;
        transaction.set_quick_repair(true);
        let current = read_metadata_write(&transaction)?;
        if current.revision != expected_revision {
            return Err(StoreError::ControlMetadataConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        let snapshot = read_snapshot_write(&transaction)?;
        let last_index = last_index(&transaction, snapshot.as_ref())?;
        validate_metadata_indexes(&state, snapshot.as_ref(), last_index)?;
        let next = VersionedControlMetadata {
            revision: current
                .revision
                .checked_add(1)
                .ok_or(StoreError::InvalidInput(
                    "control metadata revision overflow",
                ))?,
            state,
        };
        write_metadata(&transaction, &next)?;
        transaction.commit().map_err(engine)?;
        Ok(next)
    }

    fn replace_control_suffix(
        &self,
        previous_index: u64,
        entries: Vec<ControlLogEntry>,
    ) -> Result<u64, StoreError> {
        validate_entries(previous_index, &entries)?;
        let database = self.database()?;
        let mut transaction = database.begin_write().map_err(engine)?;
        transaction.set_quick_repair(true);
        let metadata = read_metadata_write(&transaction)?;
        if previous_index < metadata.state.commit_index {
            return Err(StoreError::CommittedControlPrefix {
                committed: metadata.state.commit_index,
            });
        }
        let snapshot = read_snapshot_write(&transaction)?;
        let snapshot_index = snapshot
            .as_ref()
            .map_or(0, |snapshot| snapshot.last_included_index);
        if previous_index < snapshot_index {
            return Err(StoreError::CompactedControlLog {
                snapshot: snapshot_index,
            });
        }
        let current_last = last_index(&transaction, snapshot.as_ref())?;
        if previous_index > current_last {
            return Err(StoreError::ControlLogGap {
                expected: current_last,
                actual: previous_index,
            });
        }
        if previous_index > snapshot_index {
            let log = transaction.open_table(CONTROL_LOG).map_err(engine)?;
            if log.get(previous_index).map_err(engine)?.is_none() {
                return Err(StoreError::CorruptControlState(
                    "control suffix previous index is missing",
                ));
            }
        }

        {
            let mut log = transaction.open_table(CONTROL_LOG).map_err(engine)?;
            log.retain_in(
                (Bound::Excluded(previous_index), Bound::Unbounded),
                |_, _| false,
            )
            .map_err(engine)?;
            for entry in &entries {
                let encoded = serde_json::to_vec(entry)?;
                log.insert(entry.index, encoded.as_slice())
                    .map_err(engine)?;
            }
        }
        let last = entries.last().map_or(previous_index, |entry| entry.index);
        transaction.commit().map_err(engine)?;
        Ok(last)
    }

    fn install_control_snapshot(
        &self,
        expected_metadata_revision: u64,
        snapshot: ControlSnapshot,
    ) -> Result<bool, StoreError> {
        validate_snapshot(&snapshot)?;
        let database = self.database()?;
        let mut transaction = database.begin_write().map_err(engine)?;
        transaction.set_quick_repair(true);
        let persisted_metadata = read_metadata_write(&transaction)?;
        if persisted_metadata.revision != expected_metadata_revision {
            return Err(StoreError::ControlMetadataConflict {
                expected: expected_metadata_revision,
                actual: persisted_metadata.revision,
            });
        }
        let current = read_snapshot_write(&transaction)?;
        if let Some(current) = current.as_ref() {
            if snapshot.last_included_index < current.last_included_index {
                return Err(StoreError::ControlSnapshotRegression {
                    current: current.last_included_index,
                    incoming: snapshot.last_included_index,
                });
            }
            if snapshot.last_included_index == current.last_included_index {
                if snapshot == *current {
                    return Ok(false);
                }
                return Err(StoreError::ControlSnapshotConflict {
                    index: snapshot.last_included_index,
                });
            }
        }

        let suffix_matches = {
            let log = transaction.open_table(CONTROL_LOG).map_err(engine)?;
            let matches = log
                .get(snapshot.last_included_index)
                .map_err(engine)?
                .map(|value| serde_json::from_slice::<ControlLogEntry>(value.value()))
                .transpose()?
                .is_some_and(|entry| entry.term == snapshot.last_included_term);
            matches
        };
        {
            let mut log = transaction.open_table(CONTROL_LOG).map_err(engine)?;
            if suffix_matches {
                log.retain_in(..=snapshot.last_included_index, |_, _| false)
                    .map_err(engine)?;
            } else {
                log.retain(|_, _| false).map_err(engine)?;
            }
        }
        transaction
            .open_table(CONTROL_SNAPSHOT)
            .map_err(engine)?
            .insert(SNAPSHOT_KEY, serde_json::to_vec(&snapshot)?.as_slice())
            .map_err(engine)?;

        let mut metadata = persisted_metadata;
        metadata.revision = metadata
            .revision
            .checked_add(1)
            .ok_or(StoreError::InvalidInput(
                "control metadata revision overflow",
            ))?;
        metadata.state.commit_index = metadata
            .state
            .commit_index
            .max(snapshot.last_included_index);
        metadata.state.applied_index = metadata
            .state
            .applied_index
            .max(snapshot.last_included_index);
        write_metadata(&transaction, &metadata)?;
        transaction.commit().map_err(engine)?;
        Ok(true)
    }
}

pub(crate) fn initialize_tables(transaction: &redb::WriteTransaction) -> Result<(), StoreError> {
    transaction.open_table(CONTROL_METADATA).map_err(engine)?;
    transaction.open_table(CONTROL_LOG).map_err(engine)?;
    transaction.open_table(CONTROL_SNAPSHOT).map_err(engine)?;
    Ok(())
}

fn read_metadata_read(
    transaction: &redb::ReadTransaction,
) -> Result<VersionedControlMetadata, StoreError> {
    decode_metadata(transaction.open_table(CONTROL_METADATA).map_err(engine)?)
}

fn read_metadata_write(
    transaction: &redb::WriteTransaction,
) -> Result<VersionedControlMetadata, StoreError> {
    decode_metadata(transaction.open_table(CONTROL_METADATA).map_err(engine)?)
}

fn decode_metadata(
    table: impl ReadableTable<&'static str, &'static [u8]>,
) -> Result<VersionedControlMetadata, StoreError> {
    let value = table
        .get(STATE_KEY)
        .map_err(engine)?
        .map(|value| serde_json::from_slice(value.value()))
        .transpose()?
        .unwrap_or_default();
    Ok(value)
}

fn write_metadata(
    transaction: &redb::WriteTransaction,
    metadata: &VersionedControlMetadata,
) -> Result<(), StoreError> {
    let encoded = serde_json::to_vec(metadata)?;
    transaction
        .open_table(CONTROL_METADATA)
        .map_err(engine)?
        .insert(STATE_KEY, encoded.as_slice())
        .map_err(engine)?;
    Ok(())
}

fn read_snapshot_read(
    transaction: &redb::ReadTransaction,
) -> Result<Option<ControlSnapshot>, StoreError> {
    decode_snapshot(transaction.open_table(CONTROL_SNAPSHOT).map_err(engine)?)
}

fn read_snapshot_write(
    transaction: &redb::WriteTransaction,
) -> Result<Option<ControlSnapshot>, StoreError> {
    decode_snapshot(transaction.open_table(CONTROL_SNAPSHOT).map_err(engine)?)
}

fn decode_snapshot(
    table: impl ReadableTable<&'static str, &'static [u8]>,
) -> Result<Option<ControlSnapshot>, StoreError> {
    let snapshot = table
        .get(SNAPSHOT_KEY)
        .map_err(engine)?
        .map(|value| serde_json::from_slice(value.value()))
        .transpose()?;
    Ok(snapshot)
}

fn last_index(
    transaction: &redb::WriteTransaction,
    snapshot: Option<&ControlSnapshot>,
) -> Result<u64, StoreError> {
    let log = transaction.open_table(CONTROL_LOG).map_err(engine)?;
    let last = log.last().map_err(engine)?.map_or_else(
        || snapshot.map_or(0, |snapshot| snapshot.last_included_index),
        |row| row.0.value(),
    );
    Ok(last)
}

fn validate_entries(previous_index: u64, entries: &[ControlLogEntry]) -> Result<(), StoreError> {
    if entries.len() > MAX_TRANSACTION_WRITES {
        return Err(StoreError::InvalidInput("control append limit exceeded"));
    }
    let mut expected = previous_index
        .checked_add(1)
        .ok_or(StoreError::InvalidInput("control log index overflow"))?;
    for entry in entries {
        if entry.index != expected {
            return Err(StoreError::ControlLogGap {
                expected,
                actual: entry.index,
            });
        }
        validate_identifier(&entry.kind, "control entry kind is required")?;
        validate_identifier(&entry.command_id, "control command id is required")?;
        for voter in &entry.target_voters {
            validate_identifier(voter, "control target voter is required")?;
        }
        if entry.payload.len() > MAX_RECORD_BYTES {
            return Err(StoreError::InvalidInput(
                "control entry size limit exceeded",
            ));
        }
        expected = expected
            .checked_add(1)
            .ok_or(StoreError::InvalidInput("control log index overflow"))?;
    }
    Ok(())
}

fn validate_metadata_components(state: &ControlMetadata) -> Result<(), StoreError> {
    if let Some(voted_for) = state.voted_for.as_deref() {
        validate_identifier(voted_for, "voted-for node id is required")?;
    }
    Ok(())
}

fn validate_metadata_indexes(
    state: &ControlMetadata,
    snapshot: Option<&ControlSnapshot>,
    last_index: u64,
) -> Result<(), StoreError> {
    if state.applied_index > state.commit_index {
        return Err(StoreError::InvalidInput(
            "applied index cannot exceed commit index",
        ));
    }
    if state.commit_index > last_index {
        return Err(StoreError::InvalidInput(
            "commit index cannot exceed durable log",
        ));
    }
    if snapshot.is_some_and(|snapshot| snapshot.last_included_index > state.applied_index) {
        return Err(StoreError::InvalidInput(
            "applied index cannot precede installed snapshot",
        ));
    }
    Ok(())
}

fn validate_snapshot(snapshot: &ControlSnapshot) -> Result<(), StoreError> {
    if snapshot.last_included_index == 0 {
        return Err(StoreError::InvalidInput(
            "control snapshot index is required",
        ));
    }
    validate_identifier(
        &snapshot.voter_phase,
        "control snapshot voter phase is required",
    )?;
    if snapshot.old_voters.is_empty() {
        return Err(StoreError::InvalidInput(
            "control snapshot old voters are required",
        ));
    }
    for voter in snapshot.old_voters.iter().chain(&snapshot.new_voters) {
        validate_identifier(voter, "control snapshot voter is required")?;
    }
    if snapshot.state.len() > MAX_CONTROL_SNAPSHOT_BYTES {
        return Err(StoreError::InvalidInput(
            "control snapshot size limit exceeded",
        ));
    }
    Ok(())
}

fn validate_identifier(value: &str, empty_error: &'static str) -> Result<(), StoreError> {
    if value.is_empty() {
        return Err(StoreError::InvalidInput(empty_error));
    }
    if value.as_bytes().contains(&0) {
        return Err(StoreError::InvalidInput(
            "control identifiers cannot contain NUL",
        ));
    }
    Ok(())
}

fn validate_recovery(
    metadata: &VersionedControlMetadata,
    snapshot: Option<&ControlSnapshot>,
    entries: &[ControlLogEntry],
) -> Result<(), StoreError> {
    validate_metadata_components(&metadata.state).map_err(|_| {
        StoreError::CorruptControlState("control metadata contains an invalid node id")
    })?;
    if let Some(snapshot) = snapshot {
        validate_snapshot(snapshot).map_err(|_| {
            StoreError::CorruptControlState("control snapshot metadata or size is invalid")
        })?;
    }
    let snapshot_index = snapshot.map_or(0, |snapshot| snapshot.last_included_index);
    let mut expected = snapshot_index.saturating_add(1);
    for entry in entries {
        if entry.index != expected {
            return Err(StoreError::CorruptControlState(
                "control log suffix is not contiguous",
            ));
        }
        validate_identifier(&entry.kind, "control entry kind is required")
            .map_err(|_| StoreError::CorruptControlState("control log entry kind is invalid"))?;
        validate_identifier(&entry.command_id, "control command id is required")
            .map_err(|_| StoreError::CorruptControlState("control log command id is invalid"))?;
        for voter in &entry.target_voters {
            validate_identifier(voter, "control target voter is required").map_err(|_| {
                StoreError::CorruptControlState("control log target voter is invalid")
            })?;
        }
        if entry.payload.len() > MAX_RECORD_BYTES {
            return Err(StoreError::CorruptControlState(
                "control log entry exceeds the size limit",
            ));
        }
        expected = expected.saturating_add(1);
    }
    let last_index = entries.last().map_or(snapshot_index, |entry| entry.index);
    validate_metadata_indexes(&metadata.state, snapshot, last_index)
        .map_err(|_| StoreError::CorruptControlState("control metadata indexes are inconsistent"))
}
