//! Narrow dependency boundary around Alopex Chirps.
//!
//! Chirps supplies node negotiation, membership facts, and secure messaging.
//! Nimino owns quorum, replication, conflict, storage, and product policy.

#![deny(missing_docs)]

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::net::SocketAddr;
#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

mod runtime;
mod upstream;

pub use runtime::{
    MeshClient, MeshMessage, MeshRuntime, MeshRuntimeError, MeshRuntimeOptions, MeshSubscription,
    MAX_MESSAGE_BYTES,
};

static IDENTITY_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Opaque cluster-node identity without exposing the upstream crate.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NodeId([u8; 16]);

impl NodeId {
    /// Constructs a node identity from its stable 16-byte representation.
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(upstream::canonical_node_id(bytes))
    }

    /// Returns the stable 16-byte representation.
    pub fn as_bytes(self) -> [u8; 16] {
        self.0
    }
}

/// Stable node identity loaded from the configured persistence path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NodeIdentity {
    node_id: NodeId,
}

impl NodeIdentity {
    /// Returns the stable identifier retained across process restarts.
    pub fn node_id(self) -> NodeId {
        self.node_id
    }
}

/// Production-only Chirps node configuration.
///
/// Certificate, private-key, trust-anchor, and identity paths are mandatory;
/// there is no self-signed or plaintext production mode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NodeConfig {
    bind_addr: SocketAddr,
    seeds: Vec<SocketAddr>,
    identity_path: PathBuf,
    certificate_path: PathBuf,
    private_key_path: PathBuf,
    trust_anchor_paths: Vec<PathBuf>,
}

impl NodeConfig {
    /// Creates an explicit mTLS configuration without seed peers.
    pub fn new(
        bind_addr: SocketAddr,
        identity_path: PathBuf,
        certificate_path: PathBuf,
        private_key_path: PathBuf,
        trust_anchor_paths: Vec<PathBuf>,
    ) -> Self {
        Self {
            bind_addr,
            seeds: Vec::new(),
            identity_path,
            certificate_path,
            private_key_path,
            trust_anchor_paths,
        }
    }

    /// Sets the transport seed addresses used only for peer discovery.
    pub fn with_seeds(mut self, seeds: Vec<SocketAddr>) -> Self {
        self.seeds = seeds;
        self
    }

    /// Returns the transport bind address.
    pub fn bind_addr(&self) -> SocketAddr {
        self.bind_addr
    }

    /// Returns the discovery seed addresses.
    pub fn seeds(&self) -> &[SocketAddr] {
        &self.seeds
    }

    /// Returns the stable identity file path.
    pub fn identity_path(&self) -> &Path {
        &self.identity_path
    }

    /// Returns the DER certificate path.
    pub fn certificate_path(&self) -> &Path {
        &self.certificate_path
    }

    /// Returns the PKCS#8 DER private-key path.
    pub fn private_key_path(&self) -> &Path {
        &self.private_key_path
    }

    /// Returns the explicit DER trust-anchor paths.
    pub fn trust_anchor_paths(&self) -> &[PathBuf] {
        &self.trust_anchor_paths
    }

    /// Validates production mTLS material and loads or creates the stable ID.
    pub fn prepare(&self) -> Result<NodeIdentity, NodeConfigError> {
        if self.trust_anchor_paths.is_empty() {
            return Err(NodeConfigError::TrustAnchorsRequired);
        }
        validate_material(&self.certificate_path, Material::Certificate)?;
        validate_material(&self.private_key_path, Material::PrivateKey)?;
        for path in &self.trust_anchor_paths {
            validate_material(path, Material::TrustAnchor)?;
        }
        upstream::validate_node_config(self).map_err(NodeConfigError::UpstreamConfig)?;
        load_or_create_identity(&self.identity_path)
    }
}

/// Typed node configuration and identity lifecycle failure.
#[derive(Debug)]
pub enum NodeConfigError {
    /// Production mTLS requires at least one explicit peer trust anchor.
    TrustAnchorsRequired,
    /// The configured node certificate is absent.
    CertificateMissing {
        /// Missing path.
        path: PathBuf,
    },
    /// The configured private key is absent.
    PrivateKeyMissing {
        /// Missing path.
        path: PathBuf,
    },
    /// A configured trust anchor is absent.
    TrustAnchorMissing {
        /// Missing path.
        path: PathBuf,
    },
    /// A configured certificate, key, or trust anchor is empty or not a file.
    InvalidMaterial {
        /// Invalid path.
        path: PathBuf,
    },
    /// The private key is accessible by group or other users on Unix.
    InsecurePrivateKeyPermissions {
        /// Private-key path.
        path: PathBuf,
        /// Observed Unix permission bits.
        mode: u32,
    },
    /// The persisted node identity has an unsupported byte length.
    InvalidIdentity {
        /// Identity path.
        path: PathBuf,
        /// Observed byte length.
        bytes: usize,
    },
    /// The identity file is accessible by group or other users on Unix.
    InsecureIdentityPermissions {
        /// Identity path.
        path: PathBuf,
        /// Observed Unix permission bits.
        mode: u32,
    },
    /// A filesystem operation failed.
    Io {
        /// Failed operation.
        operation: &'static str,
        /// Affected path.
        path: PathBuf,
        /// Original filesystem error.
        source: io::Error,
    },
    /// Chirps rejected the fully explicit configuration.
    UpstreamConfig(String),
}

impl fmt::Display for NodeConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TrustAnchorsRequired => {
                write!(formatter, "at least one trust anchor is required")
            }
            Self::CertificateMissing { path } => {
                write!(formatter, "certificate is missing: {}", path.display())
            }
            Self::PrivateKeyMissing { path } => {
                write!(formatter, "private key is missing: {}", path.display())
            }
            Self::TrustAnchorMissing { path } => {
                write!(formatter, "trust anchor is missing: {}", path.display())
            }
            Self::InvalidMaterial { path } => {
                write!(
                    formatter,
                    "TLS material is empty or not a file: {}",
                    path.display()
                )
            }
            Self::InsecurePrivateKeyPermissions { path, mode } => write!(
                formatter,
                "private key permissions must deny group/other access: {} ({mode:o})",
                path.display()
            ),
            Self::InvalidIdentity { path, bytes } => write!(
                formatter,
                "node identity must contain 16 or 24 bytes: {} ({bytes})",
                path.display()
            ),
            Self::InsecureIdentityPermissions { path, mode } => write!(
                formatter,
                "node identity permissions must deny group/other access: {} ({mode:o})",
                path.display()
            ),
            Self::Io {
                operation,
                path,
                source,
            } => write!(
                formatter,
                "{operation} failed for {}: {source}",
                path.display()
            ),
            Self::UpstreamConfig(reason) => {
                write!(formatter, "Chirps configuration rejected: {reason}")
            }
        }
    }
}

impl std::error::Error for NodeConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Material {
    Certificate,
    PrivateKey,
    TrustAnchor,
}

fn validate_material(path: &Path, material: Material) -> Result<(), NodeConfigError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(match material {
                Material::Certificate => NodeConfigError::CertificateMissing {
                    path: path.to_path_buf(),
                },
                Material::PrivateKey => NodeConfigError::PrivateKeyMissing {
                    path: path.to_path_buf(),
                },
                Material::TrustAnchor => NodeConfigError::TrustAnchorMissing {
                    path: path.to_path_buf(),
                },
            });
        }
        Err(source) => return Err(io_error("inspect TLS material", path, source)),
    };
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(NodeConfigError::InvalidMaterial {
            path: path.to_path_buf(),
        });
    }
    File::open(path).map_err(|source| io_error("open TLS material", path, source))?;
    #[cfg(unix)]
    if matches!(material, Material::PrivateKey) {
        let mode = metadata.mode() & 0o777;
        if mode & 0o077 != 0 {
            return Err(NodeConfigError::InsecurePrivateKeyPermissions {
                path: path.to_path_buf(),
                mode,
            });
        }
    }
    Ok(())
}

fn load_or_create_identity(path: &Path) -> Result<NodeIdentity, NodeConfigError> {
    match fs::read(path) {
        Ok(bytes) => return decode_identity(path, &bytes),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error("read node identity", path, source)),
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|source| io_error("create identity directory", parent, source))?;
    }

    let bytes = upstream::new_node_id();
    let sequence = IDENTITY_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("tmp-{}-{sequence}", std::process::id()));
    write_identity_temp(&temporary, &bytes)?;
    match fs::hard_link(&temporary, path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(source) => {
            let _ = fs::remove_file(&temporary);
            return Err(io_error("install node identity", path, source));
        }
    }
    let _ = fs::remove_file(&temporary);
    sync_parent(path)?;
    let persisted =
        fs::read(path).map_err(|source| io_error("read node identity", path, source))?;
    decode_identity(path, &persisted)
}

fn write_identity_temp(path: &Path, bytes: &[u8; 16]) -> Result<(), NodeConfigError> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|source| io_error("create temporary node identity", path, source))?;
    if let Err(source) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(io_error("persist node identity", path, source));
    }
    Ok(())
}

fn decode_identity(path: &Path, bytes: &[u8]) -> Result<NodeIdentity, NodeConfigError> {
    if bytes.len() != 16 && bytes.len() != 24 {
        return Err(NodeConfigError::InvalidIdentity {
            path: path.to_path_buf(),
            bytes: bytes.len(),
        });
    }
    #[cfg(unix)]
    {
        let mode = fs::metadata(path)
            .map_err(|source| io_error("inspect node identity", path, source))?
            .mode()
            & 0o777;
        if mode & 0o077 != 0 {
            return Err(NodeConfigError::InsecureIdentityPermissions {
                path: path.to_path_buf(),
                mode,
            });
        }
    }
    let mut node_id = [0; 16];
    node_id.copy_from_slice(&bytes[..16]);
    Ok(NodeIdentity {
        node_id: NodeId::from_bytes(node_id),
    })
}

fn sync_parent(path: &Path) -> Result<(), NodeConfigError> {
    #[cfg(not(unix))]
    {
        let _ = path;
        return Ok(());
    }
    #[cfg(unix)]
    {
        let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        else {
            return Ok(());
        };
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|source| io_error("sync identity directory", parent, source))
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> NodeConfigError {
    NodeConfigError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_id_round_trips_without_exposing_chirps() {
        let bytes = [0x2a; 16];
        assert_eq!(NodeId::from_bytes(bytes).as_bytes(), bytes);
    }
}
