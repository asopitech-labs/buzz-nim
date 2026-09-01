use sha2::{Digest, Sha256};

use crate::{NodeStorePort, StoreError, StoredRecord, MAX_PAGE_SIZE};

const EMPTY_DOMAIN: &[u8] = b"nimino.sync/v2/empty";
const RECORD_DOMAIN: &[u8] = b"nimino.sync/v2/record";
const PREFIX_DOMAIN: &[u8] = b"nimino.sync/v2/prefix";
const STATE_EMPTY_DOMAIN: &[u8] = b"nimino.sync/v2/state-empty";
const STATE_RECORD_DOMAIN: &[u8] = b"nimino.sync/v2/state-record";
const STATE_DOMAIN: &[u8] = b"nimino.sync/v2/state";

/// Durable checkpoint plus the digest of its complete canonical change prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalPrefixDigest {
    /// Highest canonical sequence covered by `digest`.
    pub checkpoint: u64,
    /// SHA-256 rolling prefix digest.
    pub digest: [u8; 32],
}

impl CanonicalPrefixDigest {
    /// Lowercase hexadecimal form used by the Nimino sync protocol.
    pub fn hex(self) -> String {
        to_hex(self.digest)
    }
}

/// Sequence-independent digest of one community's current canonical state.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CanonicalStateDigest {
    /// Local checkpoint observed while computing the state.
    pub checkpoint: u64,
    /// Number of current logical records covered by `digest`.
    pub record_count: u64,
    /// SHA-256 over records ordered by `(record_type, key)`.
    pub digest: [u8; 32],
}

impl CanonicalStateDigest {
    /// Lowercase hexadecimal form used by the synchronization protocol.
    pub fn hex(self) -> String {
        to_hex(self.digest)
    }
}

/// Digest of the empty canonical change prefix.
pub fn empty_prefix_digest() -> [u8; 32] {
    Sha256::digest(EMPTY_DOMAIN).into()
}

/// Digest one stored record using its exact ordered canonical representation.
pub fn canonical_record_digest(record: &StoredRecord) -> Result<[u8; 32], StoreError> {
    let value = serde_json::to_vec(&record.value)?;
    let mut hasher = Sha256::new();
    hasher.update(RECORD_DOMAIN);
    hasher.update(record.sequence.to_be_bytes());
    update_component(&mut hasher, record.record_type.as_bytes());
    update_component(&mut hasher, record.key.as_bytes());
    hasher.update([u8::from(record.deleted)]);
    update_component(&mut hasher, &value);
    Ok(hasher.finalize().into())
}

/// Digests logical record content without its node-local sequence.
pub fn canonical_logical_record_digest(record: &StoredRecord) -> Result<[u8; 32], StoreError> {
    let value = serde_json::to_vec(&record.value)?;
    let mut hasher = Sha256::new();
    hasher.update(STATE_RECORD_DOMAIN);
    update_component(&mut hasher, record.record_type.as_bytes());
    update_component(&mut hasher, record.key.as_bytes());
    hasher.update([u8::from(record.deleted)]);
    update_component(&mut hasher, &value);
    Ok(hasher.finalize().into())
}

/// Computes a bounded, cancelable digest of current canonical state.
pub fn canonical_state_digest(
    store: &dyn NodeStorePort,
    community_id: &str,
    page_limit: usize,
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<CanonicalStateDigest, StoreError> {
    if page_limit == 0 || page_limit > MAX_PAGE_SIZE {
        return Err(StoreError::InvalidInput(
            "sync digest page limit must be between 1 and 1000",
        ));
    }
    let checkpoint = store.canonical_checkpoint(community_id)?;
    let mut after: Option<(String, String)> = None;
    let mut digest: [u8; 32] = Sha256::digest(STATE_EMPTY_DOMAIN).into();
    let mut record_count = 0_u64;
    loop {
        if is_cancelled() {
            return Err(StoreError::SyncCancelled);
        }
        let page = store.canonical_page(
            community_id,
            after
                .as_ref()
                .map(|(record_type, key)| (record_type.as_str(), key.as_str())),
            page_limit,
        )?;
        if page.is_empty() {
            break;
        }
        for record in &page {
            let mut hasher = Sha256::new();
            hasher.update(STATE_DOMAIN);
            hasher.update(digest);
            hasher.update(canonical_logical_record_digest(record)?);
            digest = hasher.finalize().into();
            record_count = record_count
                .checked_add(1)
                .ok_or(StoreError::InvalidInput("canonical record count overflow"))?;
        }
        after = page
            .last()
            .map(|record| (record.record_type.clone(), record.key.clone()));
        if page.len() < page_limit {
            break;
        }
    }
    let actual = store.canonical_checkpoint(community_id)?;
    if checkpoint != actual {
        return Err(StoreError::CheckpointConflict {
            expected: checkpoint,
            actual,
        });
    }
    Ok(CanonicalStateDigest {
        checkpoint,
        record_count,
        digest,
    })
}

/// Extend a rolling prefix digest with one bounded, ordered record range.
pub fn extend_prefix_digest(
    mut prefix: [u8; 32],
    records: &[StoredRecord],
) -> Result<[u8; 32], StoreError> {
    for record in records {
        let record_digest = canonical_record_digest(record)?;
        let mut hasher = Sha256::new();
        hasher.update(PREFIX_DOMAIN);
        hasher.update(prefix);
        hasher.update(record_digest);
        prefix = hasher.finalize().into();
    }
    Ok(prefix)
}

/// Verify a claimed range result without interpreting product conflicts.
pub fn verify_range_digest(
    base: [u8; 32],
    records: &[StoredRecord],
    claimed: [u8; 32],
) -> Result<bool, StoreError> {
    Ok(extend_prefix_digest(base, records)? == claimed)
}

/// Recompute one community's durable prefix digest in bounded, cancelable pages.
pub fn canonical_prefix_digest(
    store: &dyn NodeStorePort,
    community_id: &str,
    page_limit: usize,
    is_cancelled: impl FnMut() -> bool,
) -> Result<CanonicalPrefixDigest, StoreError> {
    let checkpoint = store.canonical_checkpoint(community_id)?;
    canonical_prefix_digest_at(store, community_id, checkpoint, page_limit, is_cancelled)
}

/// Recompute a community prefix through an exact durable checkpoint.
pub fn canonical_prefix_digest_at(
    store: &dyn NodeStorePort,
    community_id: &str,
    checkpoint: u64,
    page_limit: usize,
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<CanonicalPrefixDigest, StoreError> {
    if page_limit == 0 || page_limit > MAX_PAGE_SIZE {
        return Err(StoreError::InvalidInput(
            "sync digest page limit must be between 1 and 1000",
        ));
    }
    if checkpoint > store.canonical_checkpoint(community_id)? {
        return Err(StoreError::InvalidInput(
            "sync digest checkpoint exceeds the durable checkpoint",
        ));
    }
    let mut cursor = 0_u64;
    let mut digest = empty_prefix_digest();
    while cursor < checkpoint {
        if is_cancelled() {
            return Err(StoreError::SyncCancelled);
        }
        let remaining = usize::try_from(checkpoint - cursor).unwrap_or(usize::MAX);
        let expected_count = remaining.min(page_limit);
        let records = store.changes(community_id, cursor, expected_count)?;
        if records.len() != expected_count {
            return Err(StoreError::CorruptCanonicalChanges(
                "durable checkpoint has a missing change record",
            ));
        }
        for record in &records {
            let expected = cursor
                .checked_add(1)
                .ok_or(StoreError::CorruptCanonicalChanges(
                    "canonical sequence overflow",
                ))?;
            if record.sequence != expected || record.sequence > checkpoint {
                return Err(StoreError::CorruptCanonicalChanges(
                    "canonical change sequence is not contiguous",
                ));
            }
            cursor = record.sequence;
        }
        digest = extend_prefix_digest(digest, &records)?;
    }
    Ok(CanonicalPrefixDigest { checkpoint, digest })
}

fn update_component(hasher: &mut Sha256, value: &[u8]) {
    hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_be_bytes());
    hasher.update(value);
}

fn to_hex(bytes: [u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(64);
    for byte in bytes {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    value
}
