use std::collections::HashSet;

use redb::{ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::{RecordWrite, RedbNodeStore, StoreError, MAX_RECORD_BYTES, MAX_TRANSACTION_WRITES};

const PROJECTION_META: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("nimino_projection_meta_v1");
const PROJECTION_ROWS: TableDefinition<&[u8], &[u8]> =
    TableDefinition::new("nimino_projection_rows_v1");

/// Immutable identity of one resumable projection build.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectionStageSpec {
    /// Mandatory tenant scope.
    pub community_id: String,
    /// `search`, `thread`, or `feed`.
    pub projection: String,
    /// Unique rebuild epoch.
    pub epoch: String,
    /// Node currently allowed to produce rows.
    pub owner_node_id: String,
    /// Exact canonical checkpoint being projected.
    pub source_checkpoint: u64,
    /// Lowercase SHA-256 of that canonical prefix.
    pub source_digest: String,
    /// Cache record type installed after completion.
    pub target_record_type: String,
}

/// Durable projection staging progress.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectionStageMetadata {
    /// Immutable build identity.
    pub spec: ProjectionStageSpec,
    /// Exact metadata CAS revision.
    pub revision: u64,
    /// Last inclusive canonical key consumed by the builder.
    pub cursor: String,
    /// True only after the source scan reaches EOF.
    pub complete: bool,
}

/// One exact-cursor batch of derived staging mutations.
#[derive(Clone, Debug)]
pub struct ProjectionStageBatch {
    /// Community whose stage is mutated.
    pub community_id: String,
    /// Projection whose stage is mutated.
    pub projection: String,
    /// Required active epoch.
    pub epoch: String,
    /// Exact metadata revision.
    pub expected_revision: u64,
    /// Exact current cursor.
    pub expected_cursor: String,
    /// Cursor after this batch.
    pub next_cursor: String,
    /// Whether this batch observed source EOF.
    pub complete: bool,
    /// Upserts or JSON-null deletions for staged rows.
    pub rows: Vec<RecordWrite>,
}

/// Metadata and all currently staged rows returned after restart.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectionStageRecovery {
    /// Durable build progress.
    pub metadata: ProjectionStageMetadata,
    /// Deterministically key-ordered staged rows.
    pub rows: Vec<RecordWrite>,
}

impl RedbNodeStore {
    /// Begin a projection epoch or idempotently recover the same identity.
    pub fn begin_projection_stage(
        &self,
        spec: ProjectionStageSpec,
    ) -> Result<ProjectionStageMetadata, StoreError> {
        validate_spec(&spec)?;
        let database = self.database()?;
        let transaction = database.begin_write().map_err(engine)?;
        let key = stage_prefix(&spec.community_id, &spec.projection);
        let metadata = {
            let mut table = transaction.open_table(PROJECTION_META).map_err(engine)?;
            if let Some(value) = table.get(key.as_slice()).map_err(engine)? {
                let metadata: ProjectionStageMetadata = serde_json::from_slice(value.value())?;
                return if metadata.spec == spec {
                    Ok(metadata)
                } else {
                    Err(StoreError::ProjectionStageConflict)
                };
            }
            let metadata = ProjectionStageMetadata {
                spec,
                revision: 0,
                cursor: String::new(),
                complete: false,
            };
            let encoded = serde_json::to_vec(&metadata)?;
            table
                .insert(key.as_slice(), encoded.as_slice())
                .map_err(engine)?;
            metadata
        };
        transaction.commit().map_err(engine)?;
        Ok(metadata)
    }

    /// Atomically stage one bounded batch under exact revision and cursor CAS.
    pub fn stage_projection_batch(
        &self,
        batch: ProjectionStageBatch,
    ) -> Result<ProjectionStageMetadata, StoreError> {
        validate_batch(&batch)?;
        let database = self.database()?;
        let transaction = database.begin_write().map_err(engine)?;
        let key = stage_prefix(&batch.community_id, &batch.projection);
        let mut metadata = read_metadata(&transaction, &key)?;
        if metadata.spec.epoch != batch.epoch {
            return Err(StoreError::ProjectionStageConflict);
        }
        if metadata.complete {
            return Err(StoreError::ProjectionStageComplete);
        }
        if metadata.revision != batch.expected_revision {
            return Err(StoreError::ProjectionStageRevisionConflict {
                expected: batch.expected_revision,
                actual: metadata.revision,
            });
        }
        if metadata.cursor != batch.expected_cursor {
            return Err(StoreError::ProjectionStageCursorConflict);
        }
        if batch.complete {
            if batch.next_cursor < batch.expected_cursor {
                return Err(StoreError::ProjectionStageCursorConflict);
            }
        } else if batch.next_cursor <= batch.expected_cursor || batch.rows.is_empty() {
            return Err(StoreError::ProjectionStageCursorConflict);
        }

        {
            let prefix = stage_prefix(&batch.community_id, &batch.projection);
            let mut rows = transaction.open_table(PROJECTION_ROWS).map_err(engine)?;
            for row in batch.rows {
                if row.record_type != metadata.spec.target_record_type {
                    return Err(StoreError::InvalidInput(
                        "projection row type does not match stage target",
                    ));
                }
                let row_key = stage_row_key(&prefix, &row.key);
                if row.deleted {
                    rows.remove(row_key.as_slice()).map_err(engine)?;
                } else {
                    let encoded = serde_json::to_vec(&row)?;
                    rows.insert(row_key.as_slice(), encoded.as_slice())
                        .map_err(engine)?;
                }
            }
        }
        metadata.revision = metadata
            .revision
            .checked_add(1)
            .ok_or(StoreError::InvalidInput("projection revision overflow"))?;
        metadata.cursor = batch.next_cursor;
        metadata.complete = batch.complete;
        let encoded = serde_json::to_vec(&metadata)?;
        transaction
            .open_table(PROJECTION_META)
            .map_err(engine)?
            .insert(key.as_slice(), encoded.as_slice())
            .map_err(engine)?;
        transaction.commit().map_err(engine)?;
        Ok(metadata)
    }

    /// Recover one active projection stage and its rows.
    pub fn recover_projection_stage(
        &self,
        community_id: &str,
        projection: &str,
    ) -> Result<ProjectionStageRecovery, StoreError> {
        validate_component(community_id)?;
        validate_component(projection)?;
        self.recover_projection_stage_from_key(&stage_prefix(community_id, projection))
    }

    /// Atomically discard one exact epoch after publish or cancellation.
    pub fn discard_projection_stage(
        &self,
        community_id: &str,
        projection: &str,
        epoch: &str,
    ) -> Result<(), StoreError> {
        validate_component(community_id)?;
        validate_component(projection)?;
        validate_component(epoch)?;
        let database = self.database()?;
        let transaction = database.begin_write().map_err(engine)?;
        let prefix = stage_prefix(community_id, projection);
        let metadata = read_metadata(&transaction, &prefix)?;
        if metadata.spec.epoch != epoch {
            return Err(StoreError::ProjectionStageConflict);
        }
        let end = prefix_end(&prefix);
        transaction
            .open_table(PROJECTION_ROWS)
            .map_err(engine)?
            .retain_in(prefix.as_slice()..end.as_slice(), |_, _| false)
            .map_err(engine)?;
        transaction
            .open_table(PROJECTION_META)
            .map_err(engine)?
            .remove(prefix.as_slice())
            .map_err(engine)?;
        transaction.commit().map_err(engine)
    }

    fn recover_projection_stage_from_key(
        &self,
        prefix: &[u8],
    ) -> Result<ProjectionStageRecovery, StoreError> {
        let database = self.database()?;
        let transaction = database.begin_read().map_err(engine)?;
        let metadata = {
            let table = transaction.open_table(PROJECTION_META).map_err(engine)?;
            let value = table
                .get(prefix)
                .map_err(engine)?
                .ok_or(StoreError::ProjectionStageMissing)?;
            serde_json::from_slice(value.value())?
        };
        let end = prefix_end(prefix);
        let table = transaction.open_table(PROJECTION_ROWS).map_err(engine)?;
        let mut rows = Vec::new();
        for entry in table
            .range::<&[u8]>(prefix..end.as_slice())
            .map_err(engine)?
        {
            let (_, value) = entry.map_err(engine)?;
            rows.push(serde_json::from_slice(value.value())?);
        }
        Ok(ProjectionStageRecovery { metadata, rows })
    }
}

pub(crate) fn initialize_tables(transaction: &redb::WriteTransaction) -> Result<(), StoreError> {
    transaction.open_table(PROJECTION_META).map_err(engine)?;
    transaction.open_table(PROJECTION_ROWS).map_err(engine)?;
    Ok(())
}

fn validate_spec(spec: &ProjectionStageSpec) -> Result<(), StoreError> {
    for value in [
        &spec.community_id,
        &spec.projection,
        &spec.epoch,
        &spec.owner_node_id,
        &spec.target_record_type,
    ] {
        validate_component(value)?;
    }
    if spec.source_digest.len() != 64
        || !spec
            .source_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(StoreError::InvalidInput(
            "projection source digest must be lowercase SHA-256",
        ));
    }
    Ok(())
}

fn validate_batch(batch: &ProjectionStageBatch) -> Result<(), StoreError> {
    for value in [&batch.community_id, &batch.projection, &batch.epoch] {
        validate_component(value)?;
    }
    if batch.rows.len() > MAX_TRANSACTION_WRITES {
        return Err(StoreError::InvalidInput(
            "projection stage batch exceeds write limit",
        ));
    }
    let mut keys = HashSet::with_capacity(batch.rows.len());
    for row in &batch.rows {
        validate_component(&row.record_type)?;
        validate_component(&row.key)?;
        if row.deleted != row.value.is_null()
            || serde_json::to_vec(&row.value)?.len() > MAX_RECORD_BYTES
            || !keys.insert(&row.key)
        {
            return Err(StoreError::InvalidInput("invalid projection stage row"));
        }
    }
    Ok(())
}

fn validate_component(value: &str) -> Result<(), StoreError> {
    if value.is_empty() || value.as_bytes().contains(&0) {
        return Err(StoreError::InvalidInput(
            "projection identifiers must be non-empty and NUL-free",
        ));
    }
    Ok(())
}

fn read_metadata(
    transaction: &redb::WriteTransaction,
    key: &[u8],
) -> Result<ProjectionStageMetadata, StoreError> {
    let table = transaction.open_table(PROJECTION_META).map_err(engine)?;
    let value = table
        .get(key)
        .map_err(engine)?
        .ok_or(StoreError::ProjectionStageMissing)?;
    Ok(serde_json::from_slice(value.value())?)
}

fn stage_prefix(community_id: &str, projection: &str) -> Vec<u8> {
    let mut key = community_id.as_bytes().to_vec();
    key.push(0);
    key.extend(projection.as_bytes());
    key.push(0);
    key
}

fn stage_row_key(prefix: &[u8], key: &str) -> Vec<u8> {
    let mut encoded = prefix.to_vec();
    encoded.extend(key.as_bytes());
    encoded
}

fn prefix_end(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    if let Some(last) = end.last_mut() {
        *last += 1;
    }
    end
}

fn engine(error: impl std::fmt::Display) -> StoreError {
    StoreError::Engine(error.to_string())
}
