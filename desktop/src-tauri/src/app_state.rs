use std::{
    collections::HashMap,
    io::Write,
    sync::{
        atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicU8},
        Arc, Mutex,
    },
};

use nostr::{Keys, ToBech32};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex as AsyncMutex;

use crate::huddle::HuddleState;
pub(crate) use crate::identity_storage::{IdentityStorage, RecoveryState, ResolvedIdentity};
use crate::managed_agents::config_bridge::SessionConfigCache;
use crate::managed_agents::{ManagedAgentPairRuntime, ManagedAgentRuntimeKey};

pub struct AppState {
    pub keys: Mutex<Keys>,
    /// Durable backend holding `keys`. Updated after the key write and before
    /// recovery flags are cleared so `get_identity` reports a consistent state.
    pub(crate) identity_storage: AtomicU8,
    pub http_client: reqwest::Client,
    /// A no-redirect client for authenticated relay media fetches (download,
    /// clipboard copy, snapshot, editor). Every caller pre-validates the URL
    /// origin, but the app-wide `http_client` follows redirects by default, so
    /// a relay `/media/` URL returning a 3xx to an off-origin or private host
    /// would forward the minted media Authorization header across origins —
    /// a redirect-hop SSRF. This client treats any 3xx as a non-success
    /// response (surfaced as an error) so the auth token never leaves the
    /// validated relay origin.
    pub media_fetch_client: reqwest::Client,
    pub relay_url_override: Mutex<Option<String>>,
    pub workspace_apply_lock: Arc<AsyncMutex<()>>,
    pub workspace_apply_generation: AtomicU64,
    /// Defers managed-agent restore until `apply_workspace` installs relay and identity.
    pub managed_agent_restore_pending: AtomicBool,
    /// Disabled by agent-managed profiles so agent profile updates survive start/restore.
    pub managed_agent_profile_reconcile_enabled: AtomicBool,
    /// Shared shutdown signal checked by launch-time agent restoration.
    pub shutdown_started: AtomicBool,
    /// Serializes every managed-runtime transition that changes the protected
    /// PID set: spawn/register, adoption, stop, shutdown, and sweep snapshots.
    /// Never perform network I/O while holding this lock.
    pub managed_agent_runtime_transition: Mutex<()>,
    pub managed_agents_store_lock: Mutex<()>,
    pub channel_templates_store_lock: Mutex<()>,
    pub managed_agent_processes: Mutex<HashMap<ManagedAgentRuntimeKey, ManagedAgentPairRuntime>>,
    pub provider_deploy_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pub huddle_state: Mutex<HuddleState>,
    pub huddle_audio: crate::huddle::tts_settings::HuddleAudioSettingsState,
    /// Tauri app handle — stored after setup so huddle commands can emit
    /// `huddle-state-changed` events without needing the handle threaded
    /// through every call site.
    ///
    /// Set once during `setup()` in `lib.rs`; never cleared.
    pub app_handle: Mutex<Option<AppHandle>>,
    /// Port of the localhost media streaming proxy (set during setup).
    pub media_proxy_port: AtomicU16,
    /// Set when identity resolution detected a "keyring-locked" state: the
    /// keyring is unreachable this boot but a migration marker shows the key
    /// lives there. An ephemeral key is generated so the app can open; all
    /// signing commands check this flag via [`AppState::signing_keys`] and
    /// return `Err` so no events are published under the inaccessible identity.
    /// Mutually exclusive with `identity_lost` (guaranteed by `RecoveryState`
    /// at the resolve boundary).
    ///
    /// Ordering: writers store with `Ordering::Release` after `state.keys` is
    /// updated, so a reader observing `false` with `Ordering::Acquire` is
    /// guaranteed to see the updated keys. Writers: `setup()` (initial
    /// resolution via `resolve_persisted_identity`) and `import_identity`
    /// (clears the flag when the user successfully imports a new key).
    pub keyring_locked: AtomicBool,
    /// Set when identity resolution detected a "lost" state: the migration
    /// marker was present but the keyring was empty. An ephemeral key was
    /// generated to let the app boot; the frontend checks this flag via
    /// `get_identity` and routes to the nsec
    /// re-import step instead of the normal onboarding profile flow.
    ///
    /// Ordering: writers store with `Ordering::Release` after `state.keys` is
    /// updated, so a reader observing `false` with `Ordering::Acquire` is
    /// guaranteed to see the updated keys. Writers: `setup()` (initial
    /// resolution) and `import_identity`/`persist_current_identity`
    /// (user-initiated key import).
    pub identity_lost: AtomicBool,
    /// Serializes runtime identity mutations (`import_identity` and
    /// `persist_current_identity`) so a stale ephemeral key can never overwrite
    /// a newer imported key during concurrent calls. Deliberately separate from
    /// `keys` so readers (signing, get_identity, etc.) are not blocked during
    /// keyring I/O.
    pub identity_mutation: Mutex<()>,
    /// Set when the boot-time Phase 2 reset attempted a wipe but verification
    /// failed. The sentinel is preserved so the next relaunch retries. All
    /// identity-dependent setup is skipped; the frontend shows a reset-failed
    /// recovery screen via `get_identity`.
    ///
    /// Ordering: written once in `setup()` with `Ordering::Release`; read in
    /// `get_identity` with `Ordering::Acquire`.
    pub reset_failed: AtomicBool,
    /// Cached ACP session config from running agents, keyed by canonical
    /// `(agent pubkey, relay URL)` runtime identity.
    /// Populated when the harness emits `session_config_captured` observer events.
    pub session_config_cache: Mutex<HashMap<ManagedAgentRuntimeKey, SessionConfigCache>>,
    /// IOKit power assertion state — prevents idle sleep while agents run.
    pub prevent_sleep: Arc<Mutex<crate::prevent_sleep::PreventSleepState>>,
    /// In-process mesh-llm node started by Nimino Desktop.
    #[cfg(feature = "mesh-llm")]
    pub mesh_llm_runtime: AsyncMutex<Option<crate::mesh_llm::DesktopMeshRuntime>>,
    #[cfg(feature = "mesh-llm")]
    pub mesh_recovery: crate::mesh_llm::MeshRecoveryState,
    /// Runtime-owned shared-compute coordinator. It publishes member-signed
    /// discovery status and reconciles MeshLLM's admission roster; MeshLLM
    /// itself owns direct QUIC/iroh connection establishment.
    #[cfg(feature = "mesh-llm")]
    pub mesh_coordinator: AsyncMutex<Option<crate::mesh_llm::MeshCoordinator>>,
    /// `(creator_pubkey_hex, channel_id)` pairs for channels the *named*
    /// identity created via `create_channel` and has not yet observed its own
    /// kind:39002 membership entry for. The relay provisions that entry
    /// asynchronously (#1761), so without this overlay a freshly created
    /// channel's owner reads back as `is_member=false` until the snapshot
    /// propagates, disabling their own composer. Entries are bound to the
    /// creating identity so an in-process identity swap (`import_identity`,
    /// workspace apply) can never inherit another identity's stale
    /// membership. Populated only by this process's own `create_channel`
    /// calls — a relay can never write into it — so it carries no
    /// trust-boundary risk. `get_channels` clears an entry once the real
    /// kind:39002 is observed for the current identity, keeping the set
    /// bounded and letting a later leave correctly flip the channel back to
    /// `is_member=false`.
    pub pending_owned_channels: Mutex<std::collections::HashSet<(String, String)>>,
    pub archive_db: crate::archive::ArchiveDb,
}

/// Parse the `NIMINO_PRIVATE_KEY` env var into identity keys. `Some` means the
/// env var was present and valid and MUST win over any persisted/keyring key
/// (the dev/CI/harness override). `None` means absent or malformed — callers
/// fall through to persisted resolution. A malformed value is logged and
/// treated as absent rather than left on an ephemeral identity.
fn identity_from_env() -> Option<Keys> {
    match std::env::var("NIMINO_PRIVATE_KEY") {
        Ok(nsec) => match Keys::parse(nsec.trim()) {
            Ok(keys) => Some(keys),
            Err(error) => {
                eprintln!("nimino-desktop: invalid NIMINO_PRIVATE_KEY: {error}");
                None
            }
        },
        Err(std::env::VarError::NotUnicode(_)) => {
            eprintln!("nimino-desktop: NIMINO_PRIVATE_KEY contains invalid UTF-8");
            None
        }
        Err(std::env::VarError::NotPresent) => None,
    }
}

/// Build the no-redirect HTTP client used for authenticated relay media
/// fetches (download / copy).
///
/// This client is a security boundary, not a convenience: it carries a minted
/// media `Authorization` header, so it MUST NOT follow redirects. A relay 3xx
/// to an off-origin or private host would otherwise forward that header across
/// origins (a redirect-hop SSRF). `redirect::Policy::none()` returns the 3xx
/// verbatim so the caller can reject it.
///
/// Returned as a `Result` so the fail-closed invariant is testable — callers
/// must never substitute a redirect-following client on build failure. Shares
/// the localhost `resolve`/pool config with the app-wide `http_client`.
pub fn build_media_fetch_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
        .pool_idle_timeout(std::time::Duration::from_secs(10))
        .pool_max_idle_per_host(1)
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

pub fn build_app_state() -> AppState {
    // Env var takes precedence (dev/CI). If absent, resolve_persisted_identity()
    // in setup() will replace the ephemeral placeholder with a persisted key.
    let (keys, identity_storage) = match identity_from_env() {
        Some(keys) => {
            eprintln!(
                "nimino-desktop: configured identity pubkey {}",
                keys.public_key().to_hex()
            );
            (keys, IdentityStorage::Environment)
        }
        None => (Keys::generate(), IdentityStorage::Ephemeral),
    };

    AppState {
        keys: Mutex::new(keys),
        identity_storage: AtomicU8::new(identity_storage as u8),
        http_client: reqwest::Client::builder()
            .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)))
            .pool_idle_timeout(std::time::Duration::from_secs(10))
            .pool_max_idle_per_host(1)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
        media_fetch_client: build_media_fetch_client().expect(
            "media_fetch_client must build with redirect::Policy::none(); a \
             redirect-following fallback would forward the minted media auth \
             header across origins (redirect-hop SSRF)",
        ),
        relay_url_override: Mutex::new(None),
        workspace_apply_lock: Arc::new(AsyncMutex::new(())),
        workspace_apply_generation: AtomicU64::new(0),
        managed_agent_restore_pending: AtomicBool::new(false),
        managed_agent_profile_reconcile_enabled: AtomicBool::new(true),
        shutdown_started: AtomicBool::new(false),
        managed_agent_runtime_transition: Mutex::new(()),
        identity_mutation: Mutex::new(()),
        managed_agents_store_lock: Mutex::new(()),
        channel_templates_store_lock: Mutex::new(()),
        managed_agent_processes: Mutex::new(HashMap::new()),
        provider_deploy_locks: Mutex::new(HashMap::new()),
        session_config_cache: Mutex::new(HashMap::new()),
        huddle_state: Mutex::new(HuddleState::default()),
        huddle_audio: Default::default(),
        app_handle: Mutex::new(None),
        media_proxy_port: AtomicU16::new(0),
        prevent_sleep: Default::default(),
        keyring_locked: AtomicBool::new(false),
        identity_lost: AtomicBool::new(false),
        reset_failed: AtomicBool::new(false),
        #[cfg(feature = "mesh-llm")]
        mesh_llm_runtime: AsyncMutex::new(None),
        #[cfg(feature = "mesh-llm")]
        mesh_recovery: crate::mesh_llm::MeshRecoveryState::default(),
        #[cfg(feature = "mesh-llm")]
        mesh_coordinator: AsyncMutex::new(None),
        pending_owned_channels: Mutex::new(std::collections::HashSet::new()),
        archive_db: crate::archive::ArchiveDb::default(),
    }
}

impl AppState {
    /// Lock the huddle state mutex, converting a poisoned-lock error to a String.
    ///
    /// Convenience wrapper — replaces 15+ instances of
    /// `state.huddle_state.lock().map_err(|e| e.to_string())?` throughout the
    /// huddle module.
    pub fn huddle(&self) -> Result<std::sync::MutexGuard<'_, crate::huddle::HuddleState>, String> {
        self.huddle_state.lock().map_err(|e| e.to_string())
    }

    pub fn get_session_cache(&self, key: &ManagedAgentRuntimeKey) -> Option<SessionConfigCache> {
        self.session_config_cache.lock().ok()?.get(key).cloned()
    }

    pub fn put_session_cache(&self, key: ManagedAgentRuntimeKey, cache: SessionConfigCache) {
        if let Ok(mut map) = self.session_config_cache.lock() {
            map.insert(key, cache);
        }
    }

    pub fn clear_agent_session_cache(&self, key: &ManagedAgentRuntimeKey) {
        if let Ok(mut map) = self.session_config_cache.lock() {
            map.remove(key);
        }
    }

    pub fn clear_agent_session_caches(&self, pubkey: &str) {
        if let Ok(mut map) = self.session_config_cache.lock() {
            map.retain(|key, _| key.pubkey != pubkey);
        }
    }

    /// Return the active identity keys if they are in a signable state.
    ///
    /// Returns `Err` when the identity is in a lost state (`identity_lost`
    /// — ephemeral key, user must re-import their nsec) or when the keyring
    /// is locked (`keyring_locked` — key is held in a keyring that is
    /// unavailable this boot). All signing and publish commands must call
    /// this instead of locking `state.keys` directly, so that recovery mode
    /// blocks publishing under an invalid or inaccessible identity.
    pub fn signing_keys(&self) -> Result<Keys, String> {
        if self
            .identity_lost
            .load(std::sync::atomic::Ordering::Acquire)
            || self
                .keyring_locked
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("identity is in recovery mode; event signing is disabled \
                 until the identity is restored and Nimino is relaunched"
                .to_string());
        }
        self.keys
            .lock()
            .map_err(|e| e.to_string())
            .map(|k| k.clone())
    }

    /// Emit the current huddle state to the frontend via Tauri event.
    ///
    /// Acquires both locks (app_handle + huddle_state), clones a snapshot,
    /// releases both, then emits. Best-effort — no-op if either lock is
    /// poisoned or the app_handle hasn't been set yet.
    pub fn emit_huddle_state_changed(&self) {
        let app = match self.app_handle.lock() {
            Ok(guard) => guard.clone(),
            Err(_) => return,
        };
        let Some(app) = app else { return };
        let snapshot = match self.huddle_state.lock() {
            Ok(hs) => hs.clone(),
            Err(_) => return,
        };
        crate::huddle::state::emit_huddle_state(&app, &snapshot);
    }
}

/// Resolve the user's identity key from the app data directory and wire
/// the resulting [`RecoveryState`] into `AppState`.
///
/// Priority: `NIMINO_PRIVATE_KEY` env var (already handled in `build_app_state`)
/// → Secret Service → generate into Secret Service.
///
/// On success, writes the resolved keys into `state.keys` (with the mutex)
/// before storing the recovery flags (Release), so any thread that reads
/// either flag as `false` with Acquire is guaranteed to see the updated keys.
///
/// Sets `state.identity_lost` on `RecoveryState::Lost` (keyring empty after
/// migration — key gone externally) and `state.keyring_locked` on
/// `RecoveryState::KeyringLocked` (keyring unreachable — key still in keyring
/// but inaccessible this boot). Both states boot with an ephemeral key; the
/// frontend shows different recovery screens for each.
pub fn resolve_persisted_identity(app: &AppHandle, state: &AppState) -> Result<(), String> {
    // Only skip persisted resolution if the env var was present AND parsed
    // successfully. A malformed env var should fall through to the persisted
    // key rather than leaving the app on an ephemeral identity.
    if identity_from_env().is_some() {
        return Ok(());
    }

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("create app data dir: {e}"))?;

    let resolved = load_or_create_identity(&data_dir)?;
    // Write keys and storage before setting the recovery flags (Release) so
    // any thread that reads a flag as false with Acquire sees consistent data.
    {
        let mut active_keys = state.keys.lock().map_err(|e| e.to_string())?;
        *active_keys = resolved.keys;
        state.set_identity_storage(resolved.storage);
    }
    state.identity_lost.store(
        resolved.recovery == RecoveryState::Lost,
        std::sync::atomic::Ordering::Release,
    );
    state.keyring_locked.store(
        resolved.recovery == RecoveryState::KeyringLocked,
        std::sync::atomic::Ordering::Release,
    );
    Ok(())
}

#[path = "app_state_keyring.rs"]
mod keyring_config;
pub(crate) use keyring_config::keyring_service;

#[path = "app_state_pending_channels.rs"]
mod pending_channels;

/// Keyring key name for the human identity nsec.
const IDENTITY_KEY_NAME: &str = "identity";

/// Filename recording that an identity has lived in Secret Service. An empty
/// reachable keyring with this marker is identity loss, never a fresh launch.
const MIGRATION_MARKER_NAME: &str = "identity.migrated";

/// The keyring operations the identity resolution flow needs. Abstracted so the
/// corrupt-keyring recovery decision ([`recover_from_keyring`]) can be
/// unit-tested against a fake without touching the live OS keyring.
trait IdentityKeyStore {
    fn probe(&self, name: &str) -> crate::secret_store::KeyringProbe;
    fn load(&self, name: &str) -> Result<Option<String>, String>;
    fn store(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
    /// Verify that `key` holds `expected` by reading directly from the OS
    /// backend — bypassing any in-process cache. Returns `Ok(true)` when the
    /// stored value matches, `Ok(false)` when it does not or is absent, and
    /// `Err` when the backend is unavailable.
    fn verify_stored(&self, key: &str, expected: &str) -> Result<bool, String>;
}

impl IdentityKeyStore for crate::secret_store::SecretStore {
    fn probe(&self, name: &str) -> crate::secret_store::KeyringProbe {
        crate::secret_store::SecretStore::probe(self, name)
    }
    fn load(&self, name: &str) -> Result<Option<String>, String> {
        crate::secret_store::SecretStore::load(self, name)
    }
    fn store(&self, name: &str, value: &str) -> Result<(), String> {
        crate::secret_store::SecretStore::store(self, name, value)
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        crate::secret_store::SecretStore::delete(self, name)
    }
    fn verify_stored(&self, key: &str, expected: &str) -> Result<bool, String> {
        crate::secret_store::SecretStore::verify_stored_raw(self, key, expected)
    }
}

/// Resolve the human identity from Secret Service. Plaintext predecessors are
/// deleted without being read; Secret Service unavailability always fails
/// closed into a non-signing recovery state.
fn load_or_create_identity(data_dir: &std::path::Path) -> Result<ResolvedIdentity, String> {
    if !cfg!(feature = "system-keyring") {
        remove_plaintext_identity(data_dir)?;
        return Err("Linux Secret Service support is required".to_string());
    }
    let store = crate::secret_store::SecretStore::shared(keyring_service());
    resolve_identity_with_store(store, data_dir)
}

/// Identity resolution over an IdentityKeyStore seam. Split from
/// load_or_create_identity so recovery is testable without the live keyring.
fn resolve_identity_with_store(
    store: &impl IdentityKeyStore,
    data_dir: &std::path::Path,
) -> Result<ResolvedIdentity, String> {
    use crate::secret_store::KeyringProbe;

    remove_plaintext_identity(data_dir)?;
    match store.probe(IDENTITY_KEY_NAME) {
        KeyringProbe::Present => {
            if let Some(nsec) = store.load(IDENTITY_KEY_NAME)? {
                return match Keys::parse(nsec.trim()) {
                    Ok(keys) => {
                        if !migration_marker_path(data_dir).exists() {
                            write_migration_marker(&migration_marker_path(data_dir))?;
                        }
                        eprintln!(
                            "nimino-desktop: persisted identity pubkey {}",
                            keys.public_key().to_hex()
                        );
                        Ok(ResolvedIdentity {
                            keys,
                            recovery: RecoveryState::None,
                            storage: IdentityStorage::SystemKeyring,
                        })
                    }
                    Err(error) => recover_from_keyring(store, data_dir, &error.to_string()),
                };
            }
            resolve_empty_keyring(store, data_dir)
        }
        KeyringProbe::ReachableButEmpty => resolve_empty_keyring(store, data_dir),
        KeyringProbe::Unreachable => {
            let keys = Keys::generate();
            eprintln!(
                "nimino-desktop: Secret Service unavailable; locked recovery uses ephemeral key {}",
                keys.public_key().to_hex()
            );
            Ok(ResolvedIdentity {
                keys,
                recovery: RecoveryState::KeyringLocked,
                storage: IdentityStorage::Ephemeral,
            })
        }
    }
}

fn resolve_empty_keyring(
    store: &impl IdentityKeyStore,
    data_dir: &std::path::Path,
) -> Result<ResolvedIdentity, String> {
    if migration_marker_path(data_dir).exists() {
        return Ok(ResolvedIdentity {
            keys: Keys::generate(),
            recovery: RecoveryState::Lost,
            storage: IdentityStorage::Ephemeral,
        });
    }
    let keys = generate_and_persist(store, data_dir)?;
    Ok(ResolvedIdentity {
        keys,
        recovery: RecoveryState::None,
        storage: IdentityStorage::SystemKeyring,
    })
}

fn recover_from_keyring(
    store: &impl IdentityKeyStore,
    data_dir: &std::path::Path,
    error: &str,
) -> Result<ResolvedIdentity, String> {
    eprintln!("nimino-desktop: corrupt Secret Service identity ({error})");
    store.delete(IDENTITY_KEY_NAME)?;
    if migration_marker_path(data_dir).exists() {
        return Ok(ResolvedIdentity {
            keys: Keys::generate(),
            recovery: RecoveryState::Lost,
            storage: IdentityStorage::Ephemeral,
        });
    }
    let keys = generate_and_persist(store, data_dir)?;
    Ok(ResolvedIdentity {
        keys,
        recovery: RecoveryState::None,
        storage: IdentityStorage::SystemKeyring,
    })
}

/// Persist into Secret Service with an uncached read-back verification.
fn persist_identity_to_keyring(
    store: &impl IdentityKeyStore,
    keys: &Keys,
    data_dir: &std::path::Path,
) -> Result<(), String> {
    let nsec = keys
        .secret_key()
        .to_bech32()
        .map_err(|error| format!("encode nsec: {error}"))?;

    write_migration_marker(&migration_marker_path(data_dir))?;
    store.store(IDENTITY_KEY_NAME, &nsec)?;
    match store.verify_stored(IDENTITY_KEY_NAME, &nsec) {
        Ok(true) => Ok(()),
        Ok(false) => Err("keyring read-back verify failed".to_string()),
        Err(error) => Err(format!("keyring read-back verify failed: {error}")),
    }
}

fn persist_imported_identity_impl(
    store: &impl IdentityKeyStore,
    keys: &Keys,
    data_dir: &std::path::Path,
) -> Result<IdentityStorage, String> {
    persist_identity_to_keyring(store, keys, data_dir)?;
    Ok(IdentityStorage::SystemKeyring)
}

pub(crate) fn persist_imported_identity(
    store: &crate::secret_store::SecretStore,
    keys: &Keys,
    data_dir: &std::path::Path,
) -> Result<IdentityStorage, String> {
    persist_imported_identity_impl(store, keys, data_dir)
}
/// Path of the migration-completed marker within `data_dir`.
fn migration_marker_path(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join(keyring_config::migration_marker_name(
        keyring_service(),
        MIGRATION_MARKER_NAME,
    ))
}

/// Atomically write (and fsync) the migration-completed marker. The content is
/// irrelevant — only the file's durable existence is the signal — so a single
/// byte keeps it minimal. Atomicity + fsync guarantee that once this returns
/// `Ok`, the marker survives a crash.
fn write_migration_marker(marker_path: &std::path::Path) -> Result<(), String> {
    use atomic_write_file::AtomicWriteFile;

    let mut file = AtomicWriteFile::open(marker_path)
        .map_err(|e| format!("open migration marker for atomic write: {e}"))?;
    file.write_all(b"1")
        .map_err(|e| format!("write migration marker: {e}"))?;
    file.commit()
        .map_err(|e| format!("commit migration marker: {e}"))
}

/// Generate and persist a fresh Secret Service identity.
fn generate_and_persist(
    store: &impl IdentityKeyStore,
    data_dir: &std::path::Path,
) -> Result<Keys, String> {
    let keys = Keys::generate();
    persist_identity_to_keyring(store, &keys, data_dir)?;
    eprintln!(
        "nimino-desktop: generated and saved identity pubkey {}",
        keys.public_key().to_hex()
    );
    Ok(keys)
}

/// Remove a predecessor plaintext secret without ever parsing or importing it.
/// Failure is fatal because continuing would leave key material on disk.
fn remove_plaintext_identity(data_dir: &std::path::Path) -> Result<(), String> {
    let path = data_dir.join("identity.key");
    if !path.exists() {
        return Ok(());
    }
    std::fs::remove_file(&path).map_err(|error| format!("remove plaintext identity.key: {error}"))
}
#[cfg(test)]
#[path = "app_state_tests.rs"]
mod tests;
