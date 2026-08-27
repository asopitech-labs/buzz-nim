//! Local content-addressed byte adapter for Nimino object replication.
//!
//! Nim owns manifests, fetch mode, pins, and GC policy. This crate only stages,
//! verifies, atomically installs, reads, and deletes digest-addressed bytes.

#![deny(missing_docs)]

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

/// Maximum accepted replication chunk; equal to the Chirps message ceiling.
pub const MAX_CHUNK_BYTES: usize = 1_048_576;
/// Largest object admitted by the v1 manifest and local adapter.
pub const MAX_OBJECT_BYTES: u64 = 68_719_476_736;

/// One resumable partial object's durable progress.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PartialState {
    /// Bytes durably appended so far.
    pub offset: u64,
    /// Expected final size.
    pub expected_size: u64,
    /// Expected lowercase SHA-256.
    pub expected_digest: String,
}

/// Result of an atomic content-addressed installation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallResult {
    /// Lowercase SHA-256 object identity.
    pub digest: String,
    /// Verified object size.
    pub size: u64,
    /// False when the same verified object was already installed.
    pub installed: bool,
}

/// Typed local object adapter failures.
#[derive(Debug, thiserror::Error)]
pub enum ObjectStoreError {
    /// A digest, transfer id, size, range, or budget was malformed.
    #[error("invalid object store input: {0}")]
    InvalidInput(&'static str),
    /// A chunk does not continue at the durable partial offset.
    #[error("partial offset mismatch: expected {expected}, actual {actual}")]
    OffsetMismatch {
        /// Durable current length.
        expected: u64,
        /// Caller-provided offset.
        actual: u64,
    },
    /// The partial does not yet contain the declared object size.
    #[error("partial object incomplete: expected {expected}, actual {actual}")]
    Incomplete {
        /// Declared object size.
        expected: u64,
        /// Durable current size.
        actual: u64,
    },
    /// Bytes do not match their content address.
    #[error("object digest mismatch: expected {expected}, actual {actual}")]
    DigestMismatch {
        /// Declared digest.
        expected: String,
        /// Computed digest.
        actual: String,
    },
    /// An installed object is missing.
    #[error("object not found")]
    NotFound,
    /// A bounded read refused to allocate the full object.
    #[error("object size {size} exceeds read limit {limit}")]
    ReadLimit {
        /// Installed object size.
        size: u64,
        /// Caller limit.
        limit: u64,
    },
    /// Filesystem I/O failed.
    #[error("object store I/O error: {0}")]
    Io(#[from] io::Error),
}

/// Filesystem-backed content-addressed object bytes.
#[derive(Clone, Debug)]
pub struct LocalObjectStore {
    root: PathBuf,
}

impl LocalObjectStore {
    /// Open or create the object and partial directories.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ObjectStoreError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(root.join("objects"))?;
        fs::create_dir_all(root.join("partials"))?;
        Ok(Self { root })
    }

    /// Create or resume one exact transfer.
    pub fn begin_partial(
        &self,
        transfer_id: &str,
        expected_digest: &str,
        expected_size: u64,
    ) -> Result<PartialState, ObjectStoreError> {
        validate_transfer_id(transfer_id)?;
        validate_digest(expected_digest)?;
        if expected_size == 0 || expected_size > MAX_OBJECT_BYTES {
            return Err(ObjectStoreError::InvalidInput(
                "object size must be between 1 byte and 64 GiB",
            ));
        }
        let path = self.partial_path(transfer_id, expected_digest, expected_size);
        let existed = path.exists();
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(path)?;
        if !existed {
            sync_directory(&self.root.join("partials"))?;
        }
        let offset = file.metadata()?.len();
        if offset > expected_size {
            return Err(ObjectStoreError::InvalidInput(
                "partial is larger than declared object",
            ));
        }
        Ok(PartialState {
            offset,
            expected_size,
            expected_digest: expected_digest.to_owned(),
        })
    }

    /// Append and fsync one bounded chunk at the exact durable offset.
    pub fn append_partial(
        &self,
        transfer_id: &str,
        expected_digest: &str,
        expected_size: u64,
        offset: u64,
        chunk: &[u8],
    ) -> Result<PartialState, ObjectStoreError> {
        if chunk.is_empty() || chunk.len() > MAX_CHUNK_BYTES {
            return Err(ObjectStoreError::InvalidInput(
                "chunk must be between 1 byte and 1 MiB",
            ));
        }
        let current = self.begin_partial(transfer_id, expected_digest, expected_size)?;
        if current.offset != offset {
            return Err(ObjectStoreError::OffsetMismatch {
                expected: current.offset,
                actual: offset,
            });
        }
        let chunk_len = u64::try_from(chunk.len())
            .map_err(|_| ObjectStoreError::InvalidInput("chunk length overflow"))?;
        if chunk_len > expected_size - offset {
            return Err(ObjectStoreError::InvalidInput(
                "chunk exceeds declared object size",
            ));
        }
        let path = self.partial_path(transfer_id, expected_digest, expected_size);
        let mut file = OpenOptions::new().append(true).open(path)?;
        file.write_all(chunk)?;
        file.sync_data()?;
        Ok(PartialState {
            offset: offset + chunk_len,
            ..current
        })
    }

    /// Verify and atomically install a complete partial without overwriting.
    pub fn finish_partial(
        &self,
        transfer_id: &str,
        expected_digest: &str,
        expected_size: u64,
    ) -> Result<InstallResult, ObjectStoreError> {
        let state = self.begin_partial(transfer_id, expected_digest, expected_size)?;
        if state.offset != expected_size {
            return Err(ObjectStoreError::Incomplete {
                expected: expected_size,
                actual: state.offset,
            });
        }
        let partial = self.partial_path(transfer_id, expected_digest, expected_size);
        let actual = digest_file(&partial)?;
        if actual != expected_digest {
            return Err(ObjectStoreError::DigestMismatch {
                expected: expected_digest.to_owned(),
                actual,
            });
        }
        File::open(&partial)?.sync_all()?;

        let object = self.object_path(expected_digest)?;
        let parent = object
            .parent()
            .ok_or(ObjectStoreError::InvalidInput("object path has no parent"))?;
        fs::create_dir_all(parent)?;
        let installed = match fs::hard_link(&partial, &object) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                verify_path(&object, expected_digest, expected_size)?;
                false
            }
            Err(error) => return Err(error.into()),
        };
        sync_directory(parent)?;
        fs::remove_file(&partial)?;
        sync_directory(&self.root.join("partials"))?;
        Ok(InstallResult {
            digest: expected_digest.to_owned(),
            size: expected_size,
            installed,
        })
    }

    /// Remove an unfinished transfer. Already-absent is idempotent success.
    pub fn abort_partial(
        &self,
        transfer_id: &str,
        expected_digest: &str,
        expected_size: u64,
    ) -> Result<(), ObjectStoreError> {
        validate_transfer_id(transfer_id)?;
        validate_digest(expected_digest)?;
        if expected_size == 0 || expected_size > MAX_OBJECT_BYTES {
            return Err(ObjectStoreError::InvalidInput("invalid object size"));
        }
        let path = self.partial_path(transfer_id, expected_digest, expected_size);
        match fs::remove_file(path) {
            Ok(()) => sync_directory(&self.root.join("partials")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    /// Verify an installed object's size and content address.
    pub fn verify(&self, digest: &str, expected_size: u64) -> Result<(), ObjectStoreError> {
        verify_path(&self.object_path(digest)?, digest, expected_size)
    }

    /// Read one complete object only when it fits the caller's budget.
    pub fn read(&self, digest: &str, limit: u64) -> Result<Vec<u8>, ObjectStoreError> {
        let path = self.object_path(digest)?;
        let size = fs::metadata(&path).map_err(map_not_found)?.len();
        if size > limit {
            return Err(ObjectStoreError::ReadLimit { size, limit });
        }
        fs::read(path).map_err(map_not_found)
    }

    /// Delete one exact installed object. Already-absent is idempotent success.
    pub fn delete(&self, digest: &str) -> Result<(), ObjectStoreError> {
        let path = self.object_path(digest)?;
        match fs::remove_file(path) {
            Ok(()) => sync_directory(&self.root.join("objects").join(&digest[..2])),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn object_path(&self, digest: &str) -> Result<PathBuf, ObjectStoreError> {
        validate_digest(digest)?;
        Ok(self.root.join("objects").join(&digest[..2]).join(digest))
    }

    fn partial_path(&self, transfer_id: &str, digest: &str, size: u64) -> PathBuf {
        self.root
            .join("partials")
            .join(format!("{transfer_id}-{digest}-{size}.part"))
    }
}

fn validate_digest(digest: &str) -> Result<(), ObjectStoreError> {
    if digest.len() != 64
        || !digest
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    {
        return Err(ObjectStoreError::InvalidInput(
            "digest must be lowercase SHA-256",
        ));
    }
    Ok(())
}

fn validate_transfer_id(transfer_id: &str) -> Result<(), ObjectStoreError> {
    if transfer_id.is_empty()
        || transfer_id.len() > 128
        || !transfer_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ObjectStoreError::InvalidInput("invalid transfer id"));
    }
    Ok(())
}

fn verify_path(path: &Path, digest: &str, expected_size: u64) -> Result<(), ObjectStoreError> {
    let size = fs::metadata(path).map_err(map_not_found)?.len();
    if size != expected_size {
        return Err(ObjectStoreError::Incomplete {
            expected: expected_size,
            actual: size,
        });
    }
    let actual = digest_file(path)?;
    if actual != digest {
        return Err(ObjectStoreError::DigestMismatch {
            expected: digest.to_owned(),
            actual,
        });
    }
    Ok(())
}

fn digest_file(path: &Path) -> Result<String, ObjectStoreError> {
    let mut file = File::open(path).map_err(map_not_found)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; MAX_CHUNK_BYTES];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

fn map_not_found(error: io::Error) -> ObjectStoreError {
    if error.kind() == io::ErrorKind::NotFound {
        ObjectStoreError::NotFound
    } else {
        ObjectStoreError::Io(error)
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), ObjectStoreError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), ObjectStoreError> {
    Ok(())
}
