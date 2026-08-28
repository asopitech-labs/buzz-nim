//! Low-level verification and repair adapters for the Nim-owned repair policy.

#![deny(missing_docs)]

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use nimino_object_store::{LocalObjectStore, ObjectStoreError, MAX_OBJECT_BYTES};
use nimino_store::{
    canonical_prefix_digest, canonical_record_digest, NodeStorePort, RecordClass, RedbNodeStore,
    MAX_PAGE_SIZE,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const PROJECTIONS: [&str; 3] = ["feed_index", "search_index", "thread_index"];
const BACKUP_CONTRACT: &str = "nimino.cutover-backup/v1";
const BACKUP_MANIFEST: &str = "manifest.json";
const BACKUP_OBJECTS: &str = "objects";
const BACKUP_STORE: &str = "store.redb";

/// Explicit content-addressed object expected by an operator repair command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
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

/// Self-verifying snapshot metadata written last into a cutover backup bundle.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BackupManifest {
    /// Manifest schema version.
    pub schema_version: u32,
    /// Fixed hard-cut backup contract identity.
    pub contract: String,
    /// Community captured by this bundle.
    pub community_id: String,
    /// Exact digest of the verified redb file.
    pub store_sha256: String,
    /// Sorted content-addressed objects included in the bundle.
    pub objects: Vec<ObjectSpec>,
    /// Semantic inventory that must match before backup and after restore.
    pub inventory: ReplicaInventory,
}

/// Create a no-clobber store and object backup, verifying it before publication.
pub fn backup_replica(
    store_path: &Path,
    community_id: &str,
    object_root: Option<&Path>,
    objects: &[ObjectSpec],
    backup_directory: &Path,
) -> Result<BackupManifest> {
    if backup_directory.exists() {
        bail!(
            "backup destination already exists: {}",
            backup_directory.display()
        );
    }
    let parent = backup_directory
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    fs::create_dir(backup_directory)?;
    let result = (|| {
        let before = verify_replica(store_path, community_id, object_root, objects)?;
        let backup_store = backup_directory.join(BACKUP_STORE);
        RedbNodeStore::open(store_path)?.backup_to(&backup_store)?;

        let mut sorted_objects = objects.to_vec();
        sorted_objects.sort_by(|left, right| left.digest.cmp(&right.digest));
        let backup_object_root = backup_directory.join(BACKUP_OBJECTS);
        if !sorted_objects.is_empty() {
            let source =
                LocalObjectStore::open(object_root.context("object root is required for backup")?)?;
            let target = LocalObjectStore::open(&backup_object_root)?;
            for (index, object) in sorted_objects.iter().enumerate() {
                target.copy_from(
                    &source,
                    &format!("backup-{index}"),
                    &object.digest,
                    object.size,
                )?;
            }
        }
        let after = verify_replica(
            &backup_store,
            community_id,
            (!sorted_objects.is_empty()).then_some(backup_object_root.as_path()),
            &sorted_objects,
        )?;
        if after != before {
            bail!("verified backup inventory differs from source inventory");
        }
        let manifest = BackupManifest {
            schema_version: 1,
            contract: BACKUP_CONTRACT.to_owned(),
            community_id: community_id.to_owned(),
            store_sha256: digest_file(&backup_store)?,
            objects: sorted_objects,
            inventory: before,
        };
        let path = backup_directory.join(BACKUP_MANIFEST);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        file.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        sync_parent(&backup_directory.join(BACKUP_MANIFEST))?;
        sync_parent(backup_directory)?;
        Ok(manifest)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(backup_directory);
    }
    result
}

/// Verify a backup bundle and restore it only into new store and object paths.
pub fn restore_replica(
    backup_directory: &Path,
    destination_store: &Path,
    destination_object_root: Option<&Path>,
    expected_community_id: &str,
) -> Result<ReplicaInventory> {
    if destination_store.exists() {
        bail!(
            "restore store destination already exists: {}",
            destination_store.display()
        );
    }
    let manifest: BackupManifest =
        serde_json::from_slice(&fs::read(backup_directory.join(BACKUP_MANIFEST))?)?;
    validate_manifest(&manifest, expected_community_id)?;
    require_outside_backup(backup_directory, destination_store)?;
    let backup_store = backup_directory.join(BACKUP_STORE);
    if digest_file(&backup_store)? != manifest.store_sha256 {
        bail!("backup store digest mismatch");
    }
    let backup_object_root = backup_directory.join(BACKUP_OBJECTS);
    let verification_store = destination_store.with_extension(format!(
        "nimino-restore-verification-{}",
        std::process::id()
    ));
    if verification_store.exists() {
        bail!(
            "restore verification path already exists: {}",
            verification_store.display()
        );
    }
    let mut verification_store_created = false;
    let backup_inventory = (|| {
        RedbNodeStore::restore_backup(&backup_store, &verification_store)?;
        verification_store_created = true;
        verify_replica(
            &verification_store,
            expected_community_id,
            (!manifest.objects.is_empty()).then_some(backup_object_root.as_path()),
            &manifest.objects,
        )
    })();
    if verification_store_created {
        let _ = fs::remove_file(&verification_store);
    }
    let backup_inventory = backup_inventory?;
    if backup_inventory != manifest.inventory {
        bail!("backup manifest inventory mismatch");
    }

    let object_destination = if manifest.objects.is_empty() {
        None
    } else {
        let path = destination_object_root.context("restore object root is required")?;
        if path.exists() {
            bail!(
                "restore object destination already exists: {}",
                path.display()
            );
        }
        require_outside_backup(backup_directory, path)?;
        fs::create_dir(path)?;
        Some(path)
    };
    let mut destination_store_created = false;
    let result = (|| {
        RedbNodeStore::restore_backup(&backup_store, destination_store)?;
        destination_store_created = true;
        if let Some(object_destination) = object_destination {
            let source = LocalObjectStore::open(&backup_object_root)?;
            let target = LocalObjectStore::open(object_destination)?;
            for (index, object) in manifest.objects.iter().enumerate() {
                target.copy_from(
                    &source,
                    &format!("restore-{index}"),
                    &object.digest,
                    object.size,
                )?;
            }
        }
        let restored = verify_replica(
            destination_store,
            expected_community_id,
            object_destination,
            &manifest.objects,
        )?;
        if restored != manifest.inventory {
            bail!("restored replica inventory differs from verified backup");
        }
        Ok(restored)
    })();
    if result.is_err() {
        if destination_store_created {
            let _ = fs::remove_file(destination_store);
        }
        if let Some(path) = object_destination {
            let _ = fs::remove_dir_all(path);
        }
    }
    result
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

fn validate_manifest(manifest: &BackupManifest, expected_community_id: &str) -> Result<()> {
    if manifest.schema_version != 1
        || manifest.contract != BACKUP_CONTRACT
        || manifest.community_id != expected_community_id
        || manifest.inventory.community_id != expected_community_id
        || manifest.store_sha256.len() != 64
        || !manifest
            .store_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("backup manifest identity is invalid");
    }
    let mut sorted = manifest.objects.clone();
    sorted.sort_by(|left, right| left.digest.cmp(&right.digest));
    if sorted != manifest.objects
        || sorted
            .windows(2)
            .any(|pair| pair[0].digest == pair[1].digest)
    {
        bail!("backup object manifest must be unique and sorted");
    }
    Ok(())
}

fn require_outside_backup(backup_directory: &Path, destination: &Path) -> Result<()> {
    let backup = fs::canonicalize(backup_directory)?;
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let resolved_parent = fs::canonicalize(parent).with_context(|| {
        format!(
            "restore destination parent must exist: {}",
            parent.display()
        )
    })?;
    let name = destination
        .file_name()
        .context("restore destination must name a file or directory")?;
    if resolved_parent.join(name).starts_with(backup) {
        bail!("restore destination must be outside the backup bundle");
    }
    Ok(())
}

fn digest_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
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
