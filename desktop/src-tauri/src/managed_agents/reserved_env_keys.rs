// Canonical reserved-env-key list, `include!`d into BOTH `build.rs`
// (compile-time rejection of baked `NIMINO_BUILD_AGENT_ENV` collisions) and
// `managed_agents/env_vars.rs` (save-time validation and spawn-time
// filtering). Build scripts cannot import from the crate, so sharing the
// source via `include!` is what guarantees the build-time check and the
// runtime filter use one identical list — zero drift surface. See
// `commands/reconnect_hook_config.rs` for the same pattern.
//
// Keep this file dependency-free: no crate-internal imports, no external
// crates. Both consumers compile it as-is.

/// Env var keys that Nimino sets itself and users must not override from
/// the persona/agent env_vars UI. Three categories:
///
/// 1. **Identity / secrets** — overriding would swap the agent's nsec or
///    leak credentials.
/// 2. **Code-execution surface** — overriding the binary/args lets the
///    user run arbitrary code as the agent process.
/// 3. **Security gates** — overriding the respond-to mode/allowlist or
///    relay URL would silently break the saved security settings (the UI
///    shows owner-only while the running agent answers anyone, for
///    example), or redirect the agent to an attacker-controlled relay.
///
/// This list is deliberately narrow — it only covers keys with security
/// implications. Behavior knobs (GOOSE_MODE, NIMINO_ACP_MODEL, NIMINO_ACP_SYSTEM_PROMPT, …) remain freely
/// overridable; those have dedicated UI fields but power users may want
/// to bypass them.
pub(crate) const RESERVED_ENV_KEYS: &[&str] = &[
    // Identity / secrets.
    "NIMINO_PRIVATE_KEY",
    "NOSTR_PRIVATE_KEY",
    "NIMINO_AUTH_TAG",
    "NIMINO_API_TOKEN",
    "NIMINO_ACP_PRIVATE_KEY",
    "NIMINO_ACP_API_TOKEN",
    // Relay URL: overriding would let a malicious config redirect the
    // agent to an attacker-controlled relay.
    "NIMINO_RELAY_URL",
    // Code-execution surface: overriding would let the user run arbitrary
    // binaries/args as the agent process.
    "NIMINO_ACP_AGENT_COMMAND",
    "NIMINO_ACP_AGENT_ARGS",
    "NIMINO_ACP_MCP_COMMAND",
    // Control-plane parallelism: the Desktop resolves the effective
    // worker-pool size (applying any per-harness cap) and writes it into
    // launch.policy_env. A user-supplied NIMINO_ACP_AGENTS would bypass the
    // harness cap and cause OpenClaw agents to spawn uncapped workers.
    "NIMINO_ACP_AGENTS",
    // Security gates: respond-to mode + allowlist + deployment allowlist +
    // legacy owner-only fallback. Overriding would make the running agent's
    // gate diverge from the saved/UI-visible settings.
    "NIMINO_ACP_RESPOND_TO",
    "NIMINO_ACP_RESPOND_TO_ALLOWLIST",
    "NIMINO_ACP_ALLOWED_RESPOND_TO",
    "NIMINO_ACP_AGENT_OWNER",
    // Stable agent identity used for git attribution and private-conversation
    // provenance must come from the managed-agent record, not user overrides.
    "NIMINO_ACP_DISPLAY_NAME",
    // Remote lifetime/presence policy: user env must not disable the
    // desktop/provider-owned bounds while the saved record still promises them.
    "NIMINO_ACP_EXIT_AFTER_INACTIVITY",
    // Desktop-owned pool lifetime policy: user env must not disable or reset
    // the idle worker-reclamation window while the desktop launcher sets it.
    "NIMINO_ACP_IDLE_POOL_SLEEP",
    "NIMINO_ACP_NO_PRESENCE",
    // Readiness handoff: desktop is the ONLY readiness source. A saved or
    // ambient env var must not be able to forge setup mode (NotReady) on a
    // Ready agent or suppress it (empty/stale payload) on a NotReady one.
    "NIMINO_ACP_SETUP_PAYLOAD",
    // Desktop ownership markers: these brand every spawned harness with the
    // launching Desktop instance. A user-supplied override would let a
    // definition masquerade as a different instance or fake the nonce used
    // for same-session sweep decisions.
    "NIMINO_MANAGED_AGENT",
    "NIMINO_MANAGED_AGENT_START_NONCE",
];

pub(crate) fn is_reserved_env_key(key: &str) -> bool {
    RESERVED_ENV_KEYS
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(key))
}
