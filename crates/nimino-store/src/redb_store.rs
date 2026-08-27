use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io,
    ops::Bound,
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard},
};

use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    CacheReplacement, CanonicalCommit, CommitResult, LogAppend, NodeStorePort, RecordClass,
    RecordWrite, StoreError, StoredRecord, MAX_PAGE_SIZE, MAX_RECORD_BYTES, MAX_TRANSACTION_WRITES,
    SCHEMA_VERSION,
};

const META: TableDefinition<&str, u64> = TableDefinition::new("nimino_meta_v1");
const CANONICAL: TableDefinition<&[u8], &[u8]> = TableDefinition::new("nimino_canonical_v1");
const CHANGES: TableDefinition<&[u8], &[u8]> = TableDefinition::new("nimino_changes_v1");
const CACHE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("nimino_cache_v1");
const LOG: TableDefinition<&[u8], &[u8]> = TableDefinition::new("nimino_log_v1");
const RECEIPTS: TableDefinition<&[u8], &[u8]> = TableDefinition::new("nimino_receipts_v1");
const SCHEMA_KEY: &str = "schema_version";

#[derive(Deserialize, Serialize)]
struct Receipt {
    digest: [u8; 32],
    checkpoint: u64,
}

/// `redb` implementation of the per-node store port.
pub struct RedbNodeStore {
    database: Mutex<Database>,
    path: PathBuf,
}

impl RedbNodeStore {
    /// Opens or bootstraps a schema-v1 store, recovering an unclean database if needed.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        let path = path.as_ref().to_path_buf();
        let database = Database::create(&path).map_err(engine)?;
        initialize(&database)?;
        Ok(Self {
            database: Mutex::new(database),
            path,
        })
    }

    /// Restores a verified backup into a new path without overwriting existing data.
    pub fn restore_backup(
        backup: impl AsRef<Path>,
        destination: impl AsRef<Path>,
    ) -> Result<(), StoreError> {
        copy_verified(backup.as_ref(), destination.as_ref())
    }

    fn database(&self) -> Result<MutexGuard<'_, Database>, StoreError> {
        self.database.lock().map_err(|_| StoreError::LockPoisoned)
    }
}

impl NodeStorePort for RedbNodeStore {
    fn canonical_checkpoint(&self, community_id: &str) -> Result<u64, StoreError> {
        validate_component(community_id, "community id is required")?;
        let database = self.database()?;
        read_meta(&database, &checkpoint_key(community_id))
    }

    fn commit_canonical(&self, intent: CanonicalCommit) -> Result<CommitResult, StoreError> {
        validate_writes(
            &intent.intent_id,
            &intent.community_id,
            &intent.writes,
            true,
            None,
        )?;
        let intent_digest = digest(&intent)?;
        let database = self.database()?;
        let mut transaction = database.begin_write().map_err(engine)?;
        transaction.set_quick_repair(true);
        let receipt_key = receipt_key(b'c', &intent.community_id, &intent.intent_id);
        if let Some(receipt) = read_receipt(&transaction, &receipt_key)? {
            return same_intent(receipt, intent_digest);
        }

        let checkpoint_key = checkpoint_key(&intent.community_id);
        let actual = read_meta_tx(&transaction, &checkpoint_key)?;
        if actual != intent.expected_checkpoint {
            return Err(StoreError::CheckpointConflict {
                expected: intent.expected_checkpoint,
                actual,
            });
        }

        let mut sequence = actual;
        {
            let mut canonical = transaction.open_table(CANONICAL).map_err(engine)?;
            let mut changes = transaction.open_table(CHANGES).map_err(engine)?;
            for write in intent.writes {
                sequence = sequence
                    .checked_add(1)
                    .ok_or(StoreError::InvalidInput("canonical checkpoint overflow"))?;
                let record = stored(sequence, write);
                let value = serde_json::to_vec(&record)?;
                canonical
                    .insert(
                        record_key(&intent.community_id, &record.record_type, &record.key)
                            .as_slice(),
                        value.as_slice(),
                    )
                    .map_err(engine)?;
                changes
                    .insert(
                        change_key(&intent.community_id, sequence).as_slice(),
                        value.as_slice(),
                    )
                    .map_err(engine)?;
            }
        }
        write_meta_tx(&transaction, &checkpoint_key, sequence)?;
        write_receipt(&transaction, &receipt_key, intent_digest, sequence)?;
        transaction.commit().map_err(engine)?;
        Ok(CommitResult {
            checkpoint: sequence,
            applied: true,
        })
    }

    fn replace_cache(&self, intent: CacheReplacement) -> Result<CommitResult, StoreError> {
        validate_component(&intent.record_type, "cache record type is required")?;
        validate_writes(
            &intent.intent_id,
            &intent.community_id,
            &intent.rows,
            false,
            Some(&intent.record_type),
        )?;
        let intent_digest = digest(&intent)?;
        let database = self.database()?;
        let mut transaction = database.begin_write().map_err(engine)?;
        transaction.set_quick_repair(true);
        let receipt_key = receipt_key(b'x', &intent.community_id, &intent.intent_id);
        if let Some(receipt) = read_receipt(&transaction, &receipt_key)? {
            return same_intent(receipt, intent_digest);
        }

        let actual = read_meta_tx(&transaction, &checkpoint_key(&intent.community_id))?;
        if actual != intent.source_checkpoint {
            return Err(StoreError::CheckpointConflict {
                expected: intent.source_checkpoint,
                actual,
            });
        }

        {
            let prefix = record_prefix(&intent.community_id, &intent.record_type);
            let end = prefix_end(&prefix);
            let mut cache = transaction.open_table(CACHE).map_err(engine)?;
            cache
                .retain_in(prefix.as_slice()..end.as_slice(), |_, _| false)
                .map_err(engine)?;
            for write in intent.rows {
                let record = stored(intent.source_checkpoint, write);
                let value = serde_json::to_vec(&record)?;
                cache
                    .insert(
                        record_key(&intent.community_id, &record.record_type, &record.key)
                            .as_slice(),
                        value.as_slice(),
                    )
                    .map_err(engine)?;
            }
        }
        write_receipt(
            &transaction,
            &receipt_key,
            intent_digest,
            intent.source_checkpoint,
        )?;
        transaction.commit().map_err(engine)?;
        Ok(CommitResult {
            checkpoint: intent.source_checkpoint,
            applied: true,
        })
    }

    fn append_log(&self, intent: LogAppend) -> Result<CommitResult, StoreError> {
        validate_writes(
            &intent.intent_id,
            &intent.community_id,
            &intent.entries,
            false,
            None,
        )?;
        if intent.entries.is_empty() {
            return Err(StoreError::InvalidInput("log entries are required"));
        }
        let intent_digest = digest(&intent)?;
        let database = self.database()?;
        let mut transaction = database.begin_write().map_err(engine)?;
        transaction.set_quick_repair(true);
        let receipt_key = receipt_key(b'l', &intent.community_id, &intent.intent_id);
        if let Some(receipt) = read_receipt(&transaction, &receipt_key)? {
            return same_intent(receipt, intent_digest);
        }

        let sequence_key = log_sequence_key(&intent.community_id);
        let mut sequence = read_meta_tx(&transaction, &sequence_key)?;
        {
            let mut log = transaction.open_table(LOG).map_err(engine)?;
            for entry in intent.entries {
                let key = record_key(&intent.community_id, &entry.record_type, &entry.key);
                if log.get(key.as_slice()).map_err(engine)?.is_some() {
                    return Err(StoreError::DuplicateLogKey);
                }
                sequence = sequence
                    .checked_add(1)
                    .ok_or(StoreError::InvalidInput("log sequence overflow"))?;
                let value = serde_json::to_vec(&stored(sequence, entry))?;
                log.insert(key.as_slice(), value.as_slice())
                    .map_err(engine)?;
            }
        }
        write_meta_tx(&transaction, &sequence_key, sequence)?;
        write_receipt(&transaction, &receipt_key, intent_digest, sequence)?;
        transaction.commit().map_err(engine)?;
        Ok(CommitResult {
            checkpoint: sequence,
            applied: true,
        })
    }

    fn get(
        &self,
        class: RecordClass,
        community_id: &str,
        record_type: &str,
        key: &str,
    ) -> Result<Option<StoredRecord>, StoreError> {
        validate_query(community_id, record_type, Some(key), 1)?;
        let database = self.database()?;
        let transaction = database.begin_read().map_err(engine)?;
        let table = transaction.open_table(table_for(class)).map_err(engine)?;
        let key = record_key(community_id, record_type, key);
        table
            .get(key.as_slice())
            .map_err(engine)?
            .map(|value| serde_json::from_slice(value.value()).map_err(StoreError::from))
            .transpose()
    }

    fn page(
        &self,
        class: RecordClass,
        community_id: &str,
        record_type: &str,
        after_key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<StoredRecord>, StoreError> {
        validate_query(community_id, record_type, after_key, limit)?;
        let database = self.database()?;
        let transaction = database.begin_read().map_err(engine)?;
        let table = transaction.open_table(table_for(class)).map_err(engine)?;
        let prefix = record_prefix(community_id, record_type);
        let end = prefix_end(&prefix);
        let start = after_key
            .map(|key| Bound::Excluded(record_key(community_id, record_type, key)))
            .unwrap_or_else(|| Bound::Included(prefix));
        let mut records = Vec::with_capacity(limit);
        for entry in table
            .range::<&[u8]>((bound_ref(&start), Bound::Excluded(end.as_slice())))
            .map_err(engine)?
            .take(limit)
        {
            let (_, value) = entry.map_err(engine)?;
            records.push(serde_json::from_slice(value.value())?);
        }
        Ok(records)
    }

    fn changes(
        &self,
        community_id: &str,
        after_sequence: u64,
        limit: usize,
    ) -> Result<Vec<StoredRecord>, StoreError> {
        validate_component(community_id, "community id is required")?;
        validate_limit(limit)?;
        if after_sequence == u64::MAX {
            return Ok(Vec::new());
        }
        let database = self.database()?;
        let transaction = database.begin_read().map_err(engine)?;
        let table = transaction.open_table(CHANGES).map_err(engine)?;
        let prefix = component_prefix(community_id);
        let start = change_key(community_id, after_sequence + 1);
        let end = prefix_end(&prefix);
        let mut records = Vec::with_capacity(limit);
        for entry in table
            .range(start.as_slice()..end.as_slice())
            .map_err(engine)?
            .take(limit)
        {
            let (_, value) = entry.map_err(engine)?;
            records.push(serde_json::from_slice(value.value())?);
        }
        Ok(records)
    }

    fn backup_to(&self, destination: &Path) -> Result<(), StoreError> {
        let _database = self.database()?;
        copy_verified(&self.path, destination)
    }
}

fn initialize(database: &Database) -> Result<(), StoreError> {
    let mut transaction = database.begin_write().map_err(engine)?;
    transaction.set_quick_repair(true);
    {
        let mut meta = transaction.open_table(META).map_err(engine)?;
        let version = meta
            .get(SCHEMA_KEY)
            .map_err(engine)?
            .map(|version| version.value());
        match version {
            Some(found) if found != SCHEMA_VERSION => {
                return Err(StoreError::UnsupportedSchema {
                    found,
                    supported: SCHEMA_VERSION,
                });
            }
            Some(_) => {}
            None => {
                meta.insert(SCHEMA_KEY, SCHEMA_VERSION).map_err(engine)?;
            }
        }
    }
    transaction.open_table(CANONICAL).map_err(engine)?;
    transaction.open_table(CHANGES).map_err(engine)?;
    transaction.open_table(CACHE).map_err(engine)?;
    transaction.open_table(LOG).map_err(engine)?;
    transaction.open_table(RECEIPTS).map_err(engine)?;
    transaction.commit().map_err(engine)
}

fn validate_writes(
    intent_id: &str,
    community_id: &str,
    writes: &[RecordWrite],
    canonical: bool,
    required_type: Option<&str>,
) -> Result<(), StoreError> {
    validate_component(intent_id, "intent id is required")?;
    validate_component(community_id, "community id is required")?;
    if canonical && writes.is_empty() {
        return Err(StoreError::InvalidInput("canonical writes are required"));
    }
    if writes.len() > MAX_TRANSACTION_WRITES {
        return Err(StoreError::InvalidInput("transaction write limit exceeded"));
    }
    let mut keys = HashSet::with_capacity(writes.len());
    for write in writes {
        validate_component(&write.record_type, "record type is required")?;
        validate_component(&write.key, "record key is required")?;
        if required_type.is_some_and(|record_type| record_type != write.record_type) {
            return Err(StoreError::InvalidInput(
                "cache row type does not match replacement scope",
            ));
        }
        if write.deleted != (canonical && write.value.is_null()) {
            return Err(StoreError::InvalidInput(
                "only canonical JSON-null tombstones may be deleted",
            ));
        }
        if serde_json::to_vec(&write.value)?.len() > MAX_RECORD_BYTES {
            return Err(StoreError::InvalidInput("record size limit exceeded"));
        }
        if !keys.insert((&write.record_type, &write.key)) {
            return Err(StoreError::InvalidInput(
                "duplicate typed key in transaction",
            ));
        }
    }
    Ok(())
}

fn validate_query(
    community_id: &str,
    record_type: &str,
    key: Option<&str>,
    limit: usize,
) -> Result<(), StoreError> {
    validate_component(community_id, "community id is required")?;
    validate_component(record_type, "record type is required")?;
    if let Some(key) = key {
        validate_component(key, "record key is required")?;
    }
    validate_limit(limit)
}

fn validate_limit(limit: usize) -> Result<(), StoreError> {
    if limit == 0 || limit > MAX_PAGE_SIZE {
        return Err(StoreError::InvalidInput(
            "page limit must be between 1 and 1000",
        ));
    }
    Ok(())
}

fn validate_component(value: &str, empty_error: &'static str) -> Result<(), StoreError> {
    if value.is_empty() {
        return Err(StoreError::InvalidInput(empty_error));
    }
    if value.as_bytes().contains(&0) {
        return Err(StoreError::InvalidInput(
            "store identifiers cannot contain NUL",
        ));
    }
    Ok(())
}

fn stored(sequence: u64, write: RecordWrite) -> StoredRecord {
    StoredRecord {
        sequence,
        record_type: write.record_type,
        key: write.key,
        deleted: write.deleted,
        value: write.value,
    }
}

fn table_for(class: RecordClass) -> TableDefinition<'static, &'static [u8], &'static [u8]> {
    match class {
        RecordClass::Canonical => CANONICAL,
        RecordClass::Cache => CACHE,
        RecordClass::Log => LOG,
    }
}

fn digest(value: &impl Serialize) -> Result<[u8; 32], StoreError> {
    Ok(Sha256::digest(serde_json::to_vec(value)?).into())
}

fn same_intent(receipt: Receipt, digest: [u8; 32]) -> Result<CommitResult, StoreError> {
    if receipt.digest != digest {
        return Err(StoreError::IntentConflict);
    }
    Ok(CommitResult {
        checkpoint: receipt.checkpoint,
        applied: false,
    })
}

fn read_receipt(
    transaction: &redb::WriteTransaction,
    key: &[u8],
) -> Result<Option<Receipt>, StoreError> {
    let table = transaction.open_table(RECEIPTS).map_err(engine)?;
    let receipt = table
        .get(key)
        .map_err(engine)?
        .map(|value| serde_json::from_slice(value.value()).map_err(StoreError::from))
        .transpose()?;
    Ok(receipt)
}

fn write_receipt(
    transaction: &redb::WriteTransaction,
    key: &[u8],
    digest: [u8; 32],
    checkpoint: u64,
) -> Result<(), StoreError> {
    let value = serde_json::to_vec(&Receipt { digest, checkpoint })?;
    transaction
        .open_table(RECEIPTS)
        .map_err(engine)?
        .insert(key, value.as_slice())
        .map_err(engine)?;
    Ok(())
}

fn read_meta(database: &Database, key: &str) -> Result<u64, StoreError> {
    let transaction = database.begin_read().map_err(engine)?;
    let table = transaction.open_table(META).map_err(engine)?;
    let value = table
        .get(key)
        .map_err(engine)?
        .map_or(0, |value| value.value());
    Ok(value)
}

fn read_meta_tx(transaction: &redb::WriteTransaction, key: &str) -> Result<u64, StoreError> {
    let table = transaction.open_table(META).map_err(engine)?;
    let value = table
        .get(key)
        .map_err(engine)?
        .map_or(0, |value| value.value());
    Ok(value)
}

fn write_meta_tx(
    transaction: &redb::WriteTransaction,
    key: &str,
    value: u64,
) -> Result<(), StoreError> {
    transaction
        .open_table(META)
        .map_err(engine)?
        .insert(key, value)
        .map_err(engine)?;
    Ok(())
}

fn checkpoint_key(community_id: &str) -> String {
    format!("canonical\0{community_id}")
}

fn log_sequence_key(community_id: &str) -> String {
    format!("log\0{community_id}")
}

fn receipt_key(class: u8, community_id: &str, intent_id: &str) -> Vec<u8> {
    let mut key = vec![class, 0];
    key.extend(record_key(community_id, "", intent_id));
    key
}

fn component_prefix(component: &str) -> Vec<u8> {
    let mut key = component.as_bytes().to_vec();
    key.push(0);
    key
}

fn record_prefix(community_id: &str, record_type: &str) -> Vec<u8> {
    let mut key = component_prefix(community_id);
    key.extend(record_type.as_bytes());
    key.push(0);
    key
}

fn record_key(community_id: &str, record_type: &str, key: &str) -> Vec<u8> {
    let mut encoded = record_prefix(community_id, record_type);
    encoded.extend(key.as_bytes());
    encoded
}

fn change_key(community_id: &str, sequence: u64) -> Vec<u8> {
    let mut key = component_prefix(community_id);
    key.extend(sequence.to_be_bytes());
    key
}

fn prefix_end(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    let last = end.last_mut().expect("validated prefix is non-empty");
    *last += 1;
    end
}

fn bound_ref(bound: &Bound<Vec<u8>>) -> Bound<&[u8]> {
    match bound {
        Bound::Included(value) => Bound::Included(value.as_slice()),
        Bound::Excluded(value) => Bound::Excluded(value.as_slice()),
        Bound::Unbounded => Bound::Unbounded,
    }
}

fn copy_verified(source: &Path, destination: &Path) -> Result<(), StoreError> {
    if destination.exists() {
        return Err(StoreError::TargetExists);
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let (temporary_path, mut output) = create_temporary(destination)?;
    let result = (|| {
        let mut input = File::open(source)?;
        io::copy(&mut input, &mut output)?;
        output.sync_all()?;
        drop(output);
        let mut database = Database::open(&temporary_path).map_err(engine)?;
        verify_schema(&database)?;
        database.check_integrity().map_err(engine)?;
        drop(database);
        match fs::hard_link(&temporary_path, destination) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                return Err(StoreError::TargetExists);
            }
            Err(error) => return Err(error.into()),
        }
        fs::remove_file(&temporary_path)?;
        sync_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), StoreError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), StoreError> {
    Ok(())
}

fn create_temporary(destination: &Path) -> Result<(PathBuf, File), StoreError> {
    for attempt in 0..100 {
        let path =
            destination.with_extension(format!("nimino-tmp-{}-{attempt}", std::process::id()));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(StoreError::InvalidInput(
        "unable to allocate temporary backup path",
    ))
}

fn verify_schema(database: &Database) -> Result<(), StoreError> {
    let transaction = database.begin_read().map_err(engine)?;
    let table = transaction.open_table(META).map_err(engine)?;
    let found = table
        .get(SCHEMA_KEY)
        .map_err(engine)?
        .map_or(0, |value| value.value());
    if found != SCHEMA_VERSION {
        return Err(StoreError::UnsupportedSchema {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(())
}

fn engine(error: impl std::fmt::Display) -> StoreError {
    StoreError::Engine(error.to_string())
}
