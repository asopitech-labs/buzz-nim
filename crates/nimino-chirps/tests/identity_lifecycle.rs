use std::fs;
use std::net::SocketAddr;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, PermissionsExt};

use nimino_chirps::{NodeConfig, NodeConfigError};
use tempfile::tempdir;

fn material(path: &std::path::Path) {
    fs::write(path, b"non-empty DER fixture").expect("write material");
}

fn config(root: &std::path::Path, generation: &str) -> NodeConfig {
    let cert = root.join(format!("{generation}.crt"));
    let key = root.join(format!("{generation}.key"));
    let trust = root.join("cluster-ca.crt");
    material(&cert);
    material(&key);
    material(&trust);
    #[cfg(unix)]
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).expect("secure private key");

    NodeConfig::new(
        "127.0.0.1:0".parse::<SocketAddr>().expect("bind address"),
        root.join("state/node.identity"),
        cert,
        key,
        vec![trust],
    )
}

#[test]
fn production_config_requires_explicit_trust() {
    let root = tempdir().expect("tempdir");
    let cert = root.path().join("node.crt");
    let key = root.path().join("node.key");
    material(&cert);
    material(&key);
    #[cfg(unix)]
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).expect("secure private key");

    let error = NodeConfig::new(
        "127.0.0.1:0".parse().expect("bind address"),
        root.path().join("node.identity"),
        cert,
        key,
        vec![],
    )
    .prepare()
    .expect_err("trust anchors are mandatory");

    assert!(matches!(error, NodeConfigError::TrustAnchorsRequired));
}

#[test]
fn identity_is_stable_across_prepare_and_certificate_rotation() {
    let root = tempdir().expect("tempdir");
    let first = config(root.path(), "generation-1")
        .prepare()
        .expect("first preparation");
    let identity_path = root.path().join("state/node.identity");
    let mut upgraded = fs::read(&identity_path).expect("initial identity bytes");
    upgraded.extend_from_slice(&1_u64.to_be_bytes());
    fs::write(&identity_path, upgraded).expect("simulate Chirps identity upgrade");
    let restarted = config(root.path(), "generation-1")
        .prepare()
        .expect("restart preparation");
    let rotated = config(root.path(), "generation-2")
        .prepare()
        .expect("rotated preparation");

    assert_eq!(first.node_id(), restarted.node_id());
    assert_eq!(first.node_id(), rotated.node_id());
    assert_eq!(fs::read(&identity_path).expect("identity bytes").len(), 24,);
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(identity_path)
            .expect("identity metadata")
            .mode()
            & 0o777,
        0o600,
    );
}

#[cfg(unix)]
#[test]
fn insecure_rotated_private_key_is_a_typed_failure() {
    let root = tempdir().expect("tempdir");
    let config = config(root.path(), "generation-2");
    fs::set_permissions(config.private_key_path(), fs::Permissions::from_mode(0o644))
        .expect("make key insecure");

    let error = config.prepare().expect_err("insecure key must fail closed");
    assert!(matches!(
        error,
        NodeConfigError::InsecurePrivateKeyPermissions { mode: 0o644, .. }
    ));
}

#[test]
fn missing_trust_anchor_is_a_typed_failure() {
    let root = tempdir().expect("tempdir");
    let cert = root.path().join("node.crt");
    let key = root.path().join("node.key");
    material(&cert);
    material(&key);
    #[cfg(unix)]
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).expect("secure private key");
    let missing = root.path().join("untrusted-ca.crt");

    let error = NodeConfig::new(
        "127.0.0.1:0".parse().expect("bind address"),
        root.path().join("node.identity"),
        cert,
        key,
        vec![missing.clone()],
    )
    .prepare()
    .expect_err("missing trust must fail closed");

    assert!(matches!(
        error,
        NodeConfigError::TrustAnchorMissing { path } if path == missing
    ));
}

#[test]
fn corrupt_identity_is_rejected_without_replacement() {
    let root = tempdir().expect("tempdir");
    let config = config(root.path(), "generation-1");
    let identity = config.identity_path();
    fs::create_dir_all(identity.parent().expect("identity parent")).expect("identity directory");
    fs::write(identity, b"truncated").expect("corrupt identity");
    #[cfg(unix)]
    fs::set_permissions(identity, fs::Permissions::from_mode(0o600))
        .expect("secure corrupt identity");

    let error = config
        .prepare()
        .expect_err("corrupt identity must fail closed");
    assert!(matches!(
        error,
        NodeConfigError::InvalidIdentity { bytes: 9, .. }
    ));
    assert_eq!(
        fs::read(identity).expect("preserved corrupt bytes"),
        b"truncated"
    );
}
