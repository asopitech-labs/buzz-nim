use crate::NodeConfig;

pub(crate) fn canonical_node_id(bytes: [u8; 16]) -> [u8; 16] {
    *alopex_chirps::NodeId::from(bytes).as_bytes()
}

pub(crate) fn new_node_id() -> [u8; 16] {
    *alopex_chirps::NodeId::new().as_bytes()
}

pub(crate) fn validate_node_config(config: &NodeConfig) -> Result<(), String> {
    chirps_node_config(config)
        .validate()
        .map_err(|error| error.to_string())
}

pub(crate) fn chirps_node_config(config: &NodeConfig) -> alopex_chirps::NodeConfig {
    alopex_chirps::NodeConfig {
        bind_addr: config.bind_addr,
        seeds: config.seeds.clone(),
        cert_path: Some(config.certificate_path.clone()),
        key_path: Some(config.private_key_path.clone()),
        trusted_cert_paths: config.trust_anchor_paths.clone(),
        node_id_path: config.identity_path.clone(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alopex_chirps::Frame;
    use alopex_chirps::MeshHandle;
    use alopex_chirps::UserMessage;
    use rcgen::generate_simple_self_signed;
    use std::fs;
    use std::net::{SocketAddr, UdpSocket};
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::time::Duration;
    use tempfile::TempDir;

    fn free_addr() -> SocketAddr {
        UdpSocket::bind("127.0.0.1:0")
            .expect("reserve loopback port")
            .local_addr()
            .expect("read loopback port")
    }

    fn write_certificate(root: &Path, name: &str, extra_name: Option<&str>) -> (PathBuf, PathBuf) {
        let mut names = vec!["alopex.local".to_owned()];
        names.extend(extra_name.map(str::to_owned));
        let certificate = generate_simple_self_signed(names).expect("test certificate");
        let certificate_path = root.join(format!("{name}.crt"));
        let private_key_path = root.join(format!("{name}.key"));
        fs::write(
            &certificate_path,
            certificate.serialize_der().expect("certificate DER"),
        )
        .expect("write certificate");
        fs::write(&private_key_path, certificate.serialize_private_key_der())
            .expect("write private key");
        #[cfg(unix)]
        fs::set_permissions(&private_key_path, fs::Permissions::from_mode(0o600))
            .expect("secure private key");
        (certificate_path, private_key_path)
    }

    fn test_config(
        root: &Path,
        identity: &str,
        certificate_path: PathBuf,
        private_key_path: PathBuf,
        trust_anchor: PathBuf,
        bind_addr: SocketAddr,
        seeds: Vec<SocketAddr>,
    ) -> alopex_chirps::NodeConfig {
        let config = NodeConfig::new(
            bind_addr,
            root.join(identity),
            certificate_path,
            private_key_path,
            vec![trust_anchor],
        )
        .with_seeds(seeds);
        config.prepare().expect("prepare node config");
        let mut mapped = chirps_node_config(&config);
        mapped.gossip_interval = Duration::from_millis(20);
        mapped
    }

    async fn connects(sender: &MeshHandle, target: alopex_chirps::NodeId) -> bool {
        for _ in 0..80 {
            if sender
                .send_to(
                    target,
                    Frame::User(UserMessage {
                        payload: b"mTLS probe".to_vec(),
                    }),
                )
                .await
                .is_ok()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }

    #[test]
    fn production_config_maps_every_explicit_security_path() {
        let bind_addr = "127.0.0.1:7311".parse::<SocketAddr>().expect("address");
        let seed = "127.0.0.1:7312".parse::<SocketAddr>().expect("seed");
        let config = NodeConfig::new(
            bind_addr,
            PathBuf::from("identity"),
            PathBuf::from("node.crt"),
            PathBuf::from("node.key"),
            vec![PathBuf::from("cluster-ca.crt")],
        )
        .with_seeds(vec![seed]);

        let mapped = chirps_node_config(&config);
        assert_eq!(mapped.bind_addr, bind_addr);
        assert_eq!(mapped.seeds, vec![seed]);
        assert_eq!(mapped.cert_path, Some(PathBuf::from("node.crt")));
        assert_eq!(mapped.key_path, Some(PathBuf::from("node.key")));
        assert_eq!(
            mapped.trusted_cert_paths,
            vec![PathBuf::from("cluster-ca.crt")]
        );
        assert_eq!(mapped.node_id_path, PathBuf::from("identity"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn mtls_accepts_trusted_rejects_untrusted_and_reloads_rotation() {
        let root = TempDir::new().expect("temporary cluster directory");

        let (trusted_cert, trusted_key) = write_certificate(root.path(), "trusted", None);
        let trusted_receiver_addr = free_addr();
        let trusted_receiver = alopex_chirps::start(test_config(
            root.path(),
            "trusted-receiver.identity",
            trusted_cert.clone(),
            trusted_key.clone(),
            trusted_cert.clone(),
            trusted_receiver_addr,
            vec![],
        ))
        .await
        .expect("start trusted receiver");
        let trusted_sender = alopex_chirps::start(test_config(
            root.path(),
            "trusted-sender.identity",
            trusted_cert.clone(),
            trusted_key.clone(),
            trusted_cert.clone(),
            free_addr(),
            vec![trusted_receiver_addr],
        ))
        .await
        .expect("start trusted sender");
        assert!(
            connects(&trusted_sender, trusted_receiver.node_id()).await,
            "mutually trusted nodes did not connect"
        );

        let (untrusted_receiver_cert, untrusted_receiver_key) =
            write_certificate(root.path(), "untrusted-receiver", None);
        let (untrusted_sender_cert, untrusted_sender_key) =
            write_certificate(root.path(), "untrusted-sender", None);
        let untrusted_receiver_addr = free_addr();
        let untrusted_receiver = alopex_chirps::start(test_config(
            root.path(),
            "untrusted-receiver.identity",
            untrusted_receiver_cert.clone(),
            untrusted_receiver_key,
            untrusted_receiver_cert,
            untrusted_receiver_addr,
            vec![],
        ))
        .await
        .expect("start untrusted receiver");
        let untrusted_sender = alopex_chirps::start(test_config(
            root.path(),
            "untrusted-sender.identity",
            untrusted_sender_cert.clone(),
            untrusted_sender_key,
            untrusted_sender_cert,
            free_addr(),
            vec![untrusted_receiver_addr],
        ))
        .await
        .expect("start untrusted sender");
        assert!(
            !connects(&untrusted_sender, untrusted_receiver.node_id()).await,
            "nodes without a shared trust anchor connected"
        );

        let (rotation_cert, rotation_key) = write_certificate(root.path(), "rotation", None);
        alopex_chirps::start(test_config(
            root.path(),
            "rotation-cache-warm.identity",
            rotation_cert.clone(),
            rotation_key.clone(),
            rotation_cert.clone(),
            free_addr(),
            vec![],
        ))
        .await
        .expect("warm certificate cache");
        let rotated = generate_simple_self_signed([
            "alopex.local".to_owned(),
            "rotation-generation-2".to_owned(),
        ])
        .expect("rotated certificate");
        fs::write(
            &rotation_cert,
            rotated.serialize_der().expect("rotated certificate DER"),
        )
        .expect("replace certificate");
        fs::write(&rotation_key, rotated.serialize_private_key_der()).expect("replace private key");
        #[cfg(unix)]
        fs::set_permissions(&rotation_key, fs::Permissions::from_mode(0o600))
            .expect("secure rotated private key");

        let rotated_receiver_addr = free_addr();
        let rotated_receiver = alopex_chirps::start(test_config(
            root.path(),
            "rotated-receiver.identity",
            rotation_cert.clone(),
            rotation_key.clone(),
            rotation_cert.clone(),
            rotated_receiver_addr,
            vec![],
        ))
        .await
        .expect("start rotated receiver");
        let rotated_sender = alopex_chirps::start(test_config(
            root.path(),
            "rotated-sender.identity",
            rotation_cert.clone(),
            rotation_key,
            rotation_cert,
            free_addr(),
            vec![rotated_receiver_addr],
        ))
        .await
        .expect("start rotated sender");
        assert!(
            connects(&rotated_sender, rotated_receiver.node_id()).await,
            "rotated certificate material was not reloaded"
        );
    }
}
