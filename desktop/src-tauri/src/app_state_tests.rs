use super::*;

use std::cell::RefCell;
use std::collections::HashMap;

use crate::secret_store::KeyringProbe;

fn assert_key_eq(left: &Keys, right: &Keys) {
    assert_eq!(left.public_key().to_hex(), right.public_key().to_hex());
}

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn with_env_key<T>(value: Option<&str>, body: impl FnOnce() -> T) -> T {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let prior = std::env::var("NIMINO_PRIVATE_KEY").ok();
    match value {
        Some(value) => std::env::set_var("NIMINO_PRIVATE_KEY", value),
        None => std::env::remove_var("NIMINO_PRIVATE_KEY"),
    }
    let output = body();
    match prior {
        Some(value) => std::env::set_var("NIMINO_PRIVATE_KEY", value),
        None => std::env::remove_var("NIMINO_PRIVATE_KEY"),
    }
    output
}

#[test]
fn identity_from_env_wins_when_valid() {
    let configured = Keys::generate();
    let nsec = configured.secret_key().to_bech32().unwrap();
    let resolved =
        with_env_key(Some(&nsec), identity_from_env).expect("valid env key must resolve");
    assert_key_eq(&configured, &resolved);
}

#[test]
fn identity_from_env_rejects_absent_or_malformed_values() {
    assert!(with_env_key(None, identity_from_env).is_none());
    assert!(with_env_key(Some("not-a-valid-nsec"), identity_from_env).is_none());
}

struct FakeIdentityStore {
    probe: KeyringProbe,
    slot: RefCell<HashMap<String, String>>,
    deleted: RefCell<Vec<String>>,
    store_fails: bool,
    verify_fails: bool,
}

impl FakeIdentityStore {
    fn present(value: &str) -> Self {
        Self {
            probe: KeyringProbe::Present,
            slot: RefCell::new(HashMap::from([(
                IDENTITY_KEY_NAME.to_string(),
                value.to_string(),
            )])),
            deleted: RefCell::new(Vec::new()),
            store_fails: false,
            verify_fails: false,
        }
    }

    fn empty() -> Self {
        Self {
            probe: KeyringProbe::ReachableButEmpty,
            slot: RefCell::new(HashMap::new()),
            deleted: RefCell::new(Vec::new()),
            store_fails: false,
            verify_fails: false,
        }
    }

    fn unreachable() -> Self {
        Self {
            probe: KeyringProbe::Unreachable,
            ..Self::empty()
        }
    }

    fn store_failing() -> Self {
        Self {
            store_fails: true,
            ..Self::empty()
        }
    }

    fn verify_failing() -> Self {
        Self {
            verify_fails: true,
            ..Self::empty()
        }
    }
}

impl IdentityKeyStore for FakeIdentityStore {
    fn probe(&self, _name: &str) -> KeyringProbe {
        self.probe
    }

    fn load(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.slot.borrow().get(name).cloned())
    }

    fn store(&self, name: &str, value: &str) -> Result<(), String> {
        if self.store_fails {
            return Err("simulated Secret Service failure".to_string());
        }
        self.slot
            .borrow_mut()
            .insert(name.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, name: &str) -> Result<(), String> {
        self.deleted.borrow_mut().push(name.to_string());
        self.slot.borrow_mut().remove(name);
        Ok(())
    }

    fn verify_stored(&self, name: &str, expected: &str) -> Result<bool, String> {
        Ok(!self.verify_fails
            && self
                .slot
                .borrow()
                .get(name)
                .is_some_and(|value| value == expected))
    }
}

#[test]
fn present_secret_service_is_authoritative_and_plaintext_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let plaintext = dir.path().join("identity.key");
    std::fs::write(&plaintext, "different-plaintext-secret").unwrap();
    let keys = Keys::generate();
    let nsec = keys.secret_key().to_bech32().unwrap();
    let store = FakeIdentityStore::present(&nsec);

    let resolved = resolve_identity_with_store(&store, dir.path()).unwrap();

    assert_key_eq(&keys, &resolved.keys);
    assert_eq!(resolved.storage, IdentityStorage::SystemKeyring);
    assert_eq!(resolved.recovery, RecoveryState::None);
    assert!(!plaintext.exists());
    assert!(migration_marker_path(dir.path()).exists());
}

#[test]
fn unreachable_secret_service_is_locked_even_on_first_run() {
    let dir = tempfile::tempdir().unwrap();
    let store = FakeIdentityStore::unreachable();

    let resolved = resolve_identity_with_store(&store, dir.path()).unwrap();

    assert_eq!(resolved.recovery, RecoveryState::KeyringLocked);
    assert_eq!(resolved.storage, IdentityStorage::Ephemeral);
    assert!(!dir.path().join("identity.key").exists());
    assert!(store.slot.borrow().is_empty());
}

#[test]
fn unreachable_secret_service_deletes_predecessor_plaintext_without_using_it() {
    let dir = tempfile::tempdir().unwrap();
    let plaintext = dir.path().join("identity.key");
    std::fs::write(
        &plaintext,
        Keys::generate().secret_key().to_bech32().unwrap(),
    )
    .unwrap();
    let store = FakeIdentityStore::unreachable();

    let resolved = resolve_identity_with_store(&store, dir.path()).unwrap();

    assert_eq!(resolved.recovery, RecoveryState::KeyringLocked);
    assert!(!plaintext.exists());
    assert!(store.slot.borrow().is_empty());
}

#[test]
fn first_run_persists_only_to_secret_service() {
    let dir = tempfile::tempdir().unwrap();
    let store = FakeIdentityStore::empty();

    let resolved = resolve_identity_with_store(&store, dir.path()).unwrap();

    assert_eq!(resolved.storage, IdentityStorage::SystemKeyring);
    assert_eq!(resolved.recovery, RecoveryState::None);
    assert_eq!(
        store.slot.borrow().get(IDENTITY_KEY_NAME).cloned(),
        Some(resolved.keys.secret_key().to_bech32().unwrap())
    );
    assert!(migration_marker_path(dir.path()).exists());
    assert!(!dir.path().join("identity.key").exists());
}

#[test]
fn empty_secret_service_after_prior_identity_returns_lost() {
    let dir = tempfile::tempdir().unwrap();
    write_migration_marker(&migration_marker_path(dir.path())).unwrap();
    let store = FakeIdentityStore::empty();

    let resolved = resolve_identity_with_store(&store, dir.path()).unwrap();

    assert_eq!(resolved.recovery, RecoveryState::Lost);
    assert_eq!(resolved.storage, IdentityStorage::Ephemeral);
    assert!(store.slot.borrow().is_empty());
}

#[test]
fn corrupt_prior_identity_returns_lost_without_plaintext_recovery() {
    let dir = tempfile::tempdir().unwrap();
    write_migration_marker(&migration_marker_path(dir.path())).unwrap();
    let store = FakeIdentityStore::present("not-a-valid-nsec");

    let resolved = resolve_identity_with_store(&store, dir.path()).unwrap();

    assert_eq!(resolved.recovery, RecoveryState::Lost);
    assert_eq!(store.deleted.borrow().as_slice(), [IDENTITY_KEY_NAME]);
    assert!(store.slot.borrow().is_empty());
}

#[test]
fn corrupt_first_run_entry_is_replaced_in_secret_service() {
    let dir = tempfile::tempdir().unwrap();
    let store = FakeIdentityStore::present("not-a-valid-nsec");

    let resolved = resolve_identity_with_store(&store, dir.path()).unwrap();

    assert_eq!(resolved.recovery, RecoveryState::None);
    assert_eq!(resolved.storage, IdentityStorage::SystemKeyring);
    assert_eq!(store.deleted.borrow().as_slice(), [IDENTITY_KEY_NAME]);
    assert!(migration_marker_path(dir.path()).exists());
}

#[test]
fn secret_service_write_failure_does_not_create_plaintext() {
    let dir = tempfile::tempdir().unwrap();
    let store = FakeIdentityStore::store_failing();

    assert!(resolve_identity_with_store(&store, dir.path()).is_err());

    assert!(!dir.path().join("identity.key").exists());
    assert!(store.slot.borrow().is_empty());
}

#[test]
fn imported_identity_requires_verified_secret_service_write() {
    let dir = tempfile::tempdir().unwrap();
    let store = FakeIdentityStore::verify_failing();
    let keys = Keys::generate();

    assert!(persist_imported_identity_impl(&store, &keys, dir.path()).is_err());

    assert!(!dir.path().join("identity.key").exists());
    assert!(migration_marker_path(dir.path()).exists());
}

#[test]
fn marker_failure_happens_before_secret_service_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let marker = migration_marker_path(dir.path());
    std::fs::create_dir(&marker).unwrap();
    let store = FakeIdentityStore::empty();

    assert!(persist_identity_to_keyring(&store, &Keys::generate(), dir.path()).is_err());

    assert!(store.slot.borrow().is_empty());
    assert!(!dir.path().join("identity.key").exists());
}

#[test]
fn signing_keys_returns_keys_only_in_normal_state() {
    let state = build_app_state();
    state
        .identity_lost
        .store(false, std::sync::atomic::Ordering::Relaxed);
    state
        .keyring_locked
        .store(false, std::sync::atomic::Ordering::Relaxed);
    assert!(state.signing_keys().is_ok());

    state
        .identity_lost
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(state.signing_keys().is_err());
    state
        .identity_lost
        .store(false, std::sync::atomic::Ordering::Relaxed);
    state
        .keyring_locked
        .store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(state.signing_keys().is_err());
}
