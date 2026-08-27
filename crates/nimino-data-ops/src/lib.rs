//! Low-level verification and repair adapters for the Nim-owned repair policy.

#![deny(missing_docs)]

use std::{
    fs::{self, File},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use nimino_object_store::{LocalObjectStore, ObjectStoreError, MAX_OBJECT_BYTES};
use nimino_store::{
    canonical_prefix_digest, canonical_record_digest, NodeStorePort, RecordClass, RedbNodeStore,
    MAX_PAGE_SIZE,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

const PROJECTIONS: [&str; 3] = ["feed_index", "search_index", "thread_index"];

/// Explicit content-addressed object expected by an operator repair command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectSpec {
    /// Lowercase SHA-256 object identity.
    pub digest: String,
    /// Exact expected byte length.
    pub size: u64,
}

/// Source, target, and no-clobber quarantine roots for object repair.
#[derive(Clone, Copy, Debug)]
pub struct ObjectRepairRoots<'a> {
    /// Verified source object root.
    pub source: Option<&'a Path>,
    /// Target object root.
    pub target: Option<&'a Path>,
    /// Destination for corrupt target objects.
    pub quarantine: Option<&'a Path>,
}

/// Deterministic verification result for one local replica.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplicaInventory {
    /// Community scoped by the operator command.
    pub community_id: String,
    /// Durable canonical checkpoint.
    pub checkpoint: u64,
    /// Digest of the complete canonical change prefix.
    pub canonical_digest: String,
    /// Digest of all search, thread, and feed cache rows.
    pub projection_digest: String,
    /// Digest of the explicitly supplied, verified object manifest.
    pub object_digest: String,
    /// Number of canonical effect rows awaiting manual reconciliation.
    pub unknown_effects: u64,
}

/// Result of an idempotent store and object repair.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairOutcome {
    /// False when target store and objects already matched the source.
    pub applied: bool,
    /// Verified target inventory after repair.
    pub inventory: ReplicaInventory,
}

/// Inspect canonical, projection, object, and effect state without choosing authority.
pub fn verify_replica(
    store_path: &Path,
    community_id: &str,
    object_root: Option<&Path>,
    objects: &[ObjectSpec],
) -> Result<ReplicaInventory> {
    if community_id.is_empty() || community_id.as_bytes().contains(&0) {
        bail!("community id must be non-empty and NUL-free");
    }
    if !store_path.is_file() {
        bail!("store does not exist: {}", store_path.display());
    }
    let store = RedbNodeStore::open(store_path)
        .with_context(|| format!("open store {}", store_path.display()))?;
    let canonical = canonical_prefix_digest(&store, community_id, MAX_PAGE_SIZE, || false)?;
    let projection_digest = record_types_digest(&store, community_id, &PROJECTIONS)?;
    let unknown_effects = count_unknown_effects(&store, community_id)?;
    let object_digest = verify_objects(object_root, objects)?;
    Ok(ReplicaInventory {
        community_id: community_id.to_owned(),
        checkpoint: canonical.checkpoint,
        canonical_digest: canonical.hex(),
        projection_digest,
        object_digest,
        unknown_effects,
    })
}

/// Replace a quarantined target from an explicitly selected healthy source.
///
/// Objects are verified and repaired before the redb file is swapped. The old
/// target is moved to `quarantine_store`; neither path is overwritten.
pub fn repair_replica(
    source_store: &Path,
    target_store: &Path,
    quarantine_store: &Path,
    community_id: &str,
    object_roots: ObjectRepairRoots<'_>,
    objects: &[ObjectSpec],
) -> Result<RepairOutcome> {
    if source_store == target_store || target_store == quarantine_store {
        bail!("source, target, and quarantine store paths must be distinct");
    }
    let source = verify_replica(source_store, community_id, object_roots.source, objects)?;
    let mut applied = repair_objects(
        object_roots.source,
        object_roots.target,
        object_roots.quarantine,
        objects,
    )?;

    if target_store.exists() {
        if let Ok(target) = verify_replica(target_store, community_id, object_roots.target, objects)
        {
            if target == source {
                return Ok(RepairOutcome {
                    applied,
                    inventory: target,
                });
            }
        }
        if quarantine_store.exists() {
            bail!(
                "store quarantine destination already exists: {}",
                quarantine_store.display()
            );
        }
    }

    let candidate = candidate_path(target_store);
    cleanup_stale_backup_files(target_store)?;
    let candidate_matches = candidate.exists()
        && verify_replica(
            &candidate,
            community_id,
            object_roots.target.or(object_roots.source),
            objects,
        )
        .is_ok_and(|inventory| inventory == source);
    if !candidate_matches {
        if candidate.exists() {
            fs::remove_file(&candidate).context("remove stale repair candidate")?;
        }
        let source_adapter = RedbNodeStore::open(source_store)?;
        source_adapter.backup_to(&candidate)?;
        drop(source_adapter);
        let candidate_inventory = verify_replica(
            &candidate,
            community_id,
            object_roots.target.or(object_roots.source),
            objects,
        )?;
        if candidate_inventory != source {
            let _ = fs::remove_file(&candidate);
            bail!("verified repair candidate differs from source inventory");
        }
    }

    if target_store.exists() {
        if let Some(parent) = quarantine_store.parent() {
            fs::create_dir_all(parent)?;
        }
        if let Err(error) = fs::rename(target_store, quarantine_store) {
            let _ = fs::remove_file(&candidate);
            return Err(error).with_context(|| {
                format!(
                    "quarantine target {} at {}",
                    target_store.display(),
                    quarantine_store.display()
                )
            });
        }
        if let Err(error) = sync_parent(quarantine_store) {
            let _ = fs::rename(quarantine_store, target_store);
            let _ = fs::remove_file(&candidate);
            return Err(error).context("sync store quarantine directory");
        }
        if let Err(error) = fs::rename(&candidate, target_store) {
            let _ = fs::rename(quarantine_store, target_store);
            let _ = fs::remove_file(&candidate);
            return Err(error).context("install verified repair candidate");
        }
    } else {
        fs::rename(&candidate, target_store).context("install verified repair candidate")?;
    }
    sync_parent(target_store)?;
    applied = true;
    let inventory = verify_replica(target_store, community_id, object_roots.target, objects)?;
    if inventory != source {
        bail!("installed repair does not match source inventory");
    }
    Ok(RepairOutcome { applied, inventory })
}

fn repair_objects(
    source_root: Option<&Path>,
    target_root: Option<&Path>,
    quarantine_root: Option<&Path>,
    objects: &[ObjectSpec],
) -> Result<bool> {
    if objects.is_empty() {
        return Ok(false);
    }
    let source = LocalObjectStore::open(source_root.context("source object root is required")?)?;
    let target = LocalObjectStore::open(target_root.context("target object root is required")?)?;
    let quarantine = quarantine_root.context("object quarantine root is required")?;
    let mut applied = false;
    for (index, object) in objects.iter().enumerate() {
        source.verify(&object.digest, object.size)?;
        match target.verify(&object.digest, object.size) {
            Ok(()) => continue,
            Err(ObjectStoreError::NotFound) => {}
            Err(ObjectStoreError::DigestMismatch { .. })
            | Err(ObjectStoreError::Incomplete { .. }) => {
                target.quarantine_to(&object.digest, quarantine.join(&object.digest))?;
                applied = true;
            }
            Err(error) => return Err(error.into()),
        }
        let transfer_id = format!("repair-{index}");
        applied |= target
            .copy_from(&source, &transfer_id, &object.digest, object.size)?
            .installed;
    }
    Ok(applied)
}

fn verify_objects(root: Option<&Path>, objects: &[ObjectSpec]) -> Result<String> {
    let mut objects = objects.to_vec();
    objects.sort_by(|left, right| left.digest.cmp(&right.digest));
    if objects
        .windows(2)
        .any(|pair| pair[0].digest == pair[1].digest)
    {
        bail!("duplicate object digest");
    }
    if objects
        .iter()
        .any(|object| object.size == 0 || object.size > MAX_OBJECT_BYTES)
    {
        bail!("object size must be between 1 byte and 64 GiB");
    }
    let store = if objects.is_empty() {
        None
    } else {
        let root = root.context("object root is required")?;
        if !root.is_dir() {
            bail!("object root does not exist: {}", root.display());
        }
        Some(LocalObjectStore::open(root)?)
    };
    let mut digest = Sha256::new();
    digest.update(b"nimino.data-ops/v1/objects");
    for object in objects {
        store
            .as_ref()
            .context("object store is missing for non-empty manifest")?
            .verify(&object.digest, object.size)?;
        digest.update(object.digest.as_bytes());
        digest.update(object.size.to_be_bytes());
    }
    Ok(hex::encode(digest.finalize()))
}

fn record_types_digest(
    store: &dyn NodeStorePort,
    community_id: &str,
    record_types: &[&str],
) -> Result<String> {
    let mut digest = Sha256::new();
    digest.update(b"nimino.data-ops/v1/record-types");
    for record_type in record_types {
        digest.update(record_type.as_bytes());
        let mut cursor: Option<String> = None;
        loop {
            let rows = store.page(
                RecordClass::Cache,
                community_id,
                record_type,
                cursor.as_deref(),
                MAX_PAGE_SIZE,
            )?;
            for row in &rows {
                digest.update(canonical_record_digest(row)?);
            }
            if rows.len() < MAX_PAGE_SIZE {
                break;
            }
            cursor = rows.last().map(|row| row.key.clone());
        }
    }
    Ok(hex::encode(digest.finalize()))
}

fn count_unknown_effects(store: &dyn NodeStorePort, community_id: &str) -> Result<u64> {
    let mut count = 0_u64;
    let mut cursor: Option<String> = None;
    loop {
        let rows = store.page(
            RecordClass::Canonical,
            community_id,
            "workflow_effect",
            cursor.as_deref(),
            MAX_PAGE_SIZE,
        )?;
        for row in &rows {
            if !row.deleted
                && row.value.get("status").and_then(serde_json::Value::as_str) == Some("unknown")
            {
                count = count
                    .checked_add(1)
                    .context("unknown effect count overflow")?;
            }
        }
        if rows.len() < MAX_PAGE_SIZE {
            break;
        }
        cursor = rows.last().map(|row| row.key.clone());
    }
    Ok(count)
}

fn candidate_path(target: &Path) -> PathBuf {
    target.with_extension("nimino-repair-candidate")
}

fn cleanup_stale_backup_files(target: &Path) -> Result<()> {
    // ponytail: operator repairs are serialized per target. Add a durable
    // advisory lock if an orchestrator ever runs concurrent repairs.
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let stem = target
        .file_stem()
        .and_then(|value| value.to_str())
        .context("target store filename must be UTF-8")?;
    let prefix = format!("{stem}.nimino-tmp-");
    if !parent.exists() {
        fs::create_dir_all(parent)?;
        return Ok(());
    }
    for entry in fs::read_dir(parent)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
        {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<()> {
    Ok(())
}
