//! Relay-operator community provisioning HTTP handler support.
//!
//! ## Authorization: operator, not owner
//!
//! Every other admin surface in this relay is community-scoped — the sender's
//! role is looked up in `relay_members (community_id, pubkey)` for the
//! host-resolved tenant. Community *creation* cannot work that way: its effect
//! is the creation of tenancy itself, so the authorizing identity must sit
//! above tenants. The gate here is the deployment-level
//! `RELAY_OPERATOR_PUBKEYS` allowlist (see `Config::relay_operator_pubkeys`).
//! An empty allowlist (the default) disables provisioning entirely.
//!
//! The public surface is `POST /operator/communities`, authenticated by NIP-98
//! and gated by the deployment-level `RELAY_OPERATOR_PUBKEYS` allowlist. The
//! endpoint is intentionally outside the Nostr event ingest data plane: no
//! relay-membership bypass, no special event kind, no storage or fan-out.
//!
//! ## Request shape
//!
//! ```json
//! { "host": "acme.communities.nimino.xyz", "initial_owner_pubkey": "<hex>" }
//! ```
//!
//! Creation is atomic and fails when the host already exists. Nimino has no
//! convergence or owner-rotation compatibility mode on this endpoint.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use nimino_boundary::{
    CommunityAction, CommunityCommand, CommunityLifecycleRequest, CommunityPolicyError,
    CommunityPolicyRequest, CommunityPolicyResult, CommunityState,
};
use nimino_core::tenant::{normalize_host, TenantContext};
use url::{Host, Url};

use crate::state::AppState;

/// Maximum accepted authority length. Matches `communities.host VARCHAR(255)`.
const MAX_HOST_LEN: usize = 255;

/// JSON body for `POST /operator/communities`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisionCommunityRequest {
    /// Normalized authority for the community to create.
    pub host: String,
    /// Initial owner created atomically with the community.
    pub initial_owner_pubkey: String,
}

/// JSON response from `POST /operator/communities`.
#[derive(Debug, Serialize)]
pub struct ProvisionCommunityResponse {
    /// UUID of the created community.
    pub community_id: String,
    /// Canonical host stored on the community row.
    pub host: String,
    /// Always `created`; an existing host is rejected.
    pub status: &'static str,
    /// Echoes the validated initial owner pubkey.
    pub owner_pubkey: String,
}

pub(crate) fn validate_pubkey_hex(value: &str) -> Option<String> {
    let normalized = value.to_ascii_lowercase();
    (normalized.len() == 64 && normalized.chars().all(|c| c.is_ascii_hexdigit()))
        .then_some(normalized)
}

/// Validate a normalized host authority value for a community.
///
/// The host must already be in normalized shape (`normalize_host` is a
/// no-op on it): lowercase, no default port, no trailing dot. Requiring the
/// caller to send the normalized form keeps the stored `communities.host`
/// value byte-identical to what request-time host resolution will look up.
fn validate_host(host: &str) -> Result<(), String> {
    if host.is_empty() {
        return Err("host is empty".to_string());
    }
    if host.len() > MAX_HOST_LEN {
        return Err(format!(
            "host too long: {} bytes (max {MAX_HOST_LEN})",
            host.len()
        ));
    }
    if normalize_host(host) != host {
        return Err(format!(
            "host is not normalized: expected {:?}",
            normalize_host(host)
        ));
    }
    validate_authority(host)
}

fn validate_authority(authority: &str) -> Result<(), String> {
    if authority
        .chars()
        .any(|c| c.is_control() || c.is_whitespace())
    {
        return Err("host contains invalid characters".to_string());
    }
    if authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
        || authority.contains('@')
    {
        return Err(
            "host must be a bare authority (no scheme, path, query, or userinfo)".to_string(),
        );
    }

    // Parse as an HTTP authority by wrapping it in a URL. This rejects empty
    // hosts, malformed bracketed IPv6 literals, and invalid ports while keeping
    // the accepted shape aligned with request `Host` authority syntax.
    let parsed = Url::parse(&format!("http://{authority}/"))
        .map_err(|_| "host is not a valid authority".to_string())?;
    let host = parsed
        .host()
        .ok_or_else(|| "host is not a valid authority".to_string())?;

    let serialized_host = match host {
        Host::Domain(domain) => {
            validate_domain_labels(domain)?;
            domain.to_string()
        }
        Host::Ipv4(addr) => addr.to_string(),
        Host::Ipv6(addr) => format!("[{addr}]"),
    };
    let canonical_authority = match parsed.port() {
        Some(port) => format!("{serialized_host}:{port}"),
        None => serialized_host,
    };

    if canonical_authority != authority {
        return Err(format!(
            "host is not a canonical authority: expected {canonical_authority:?}"
        ));
    }

    Ok(())
}

fn validate_domain_labels(domain: &str) -> Result<(), String> {
    if domain.len() > 253 {
        return Err("domain name too long".to_string());
    }
    for label in domain.split('.') {
        if label.is_empty() {
            return Err("domain contains an empty label".to_string());
        }
        if label.len() > 63 {
            return Err("domain label too long".to_string());
        }
        let valid_label = label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-');
        if !valid_label {
            return Err("domain label contains invalid characters".to_string());
        }
    }
    Ok(())
}

/// Normalize and validate a host supplied to read-only operator endpoints.
///
/// Unlike create, availability checks may accept non-canonical but normalizable
/// authority values (uppercase host, trailing dot, default port) so kgoose can
/// ask the relay for the canonical spelling before creating. Schemes, paths,
/// userinfo, whitespace/control characters, and oversized values are still
/// rejected.
pub(crate) fn normalize_candidate_host(host: &str) -> Result<String, String> {
    if host.is_empty() {
        return Err("host is empty".to_string());
    }
    if host.len() > MAX_HOST_LEN {
        return Err(format!(
            "host too long: {} bytes (max {MAX_HOST_LEN})",
            host.len()
        ));
    }
    if host.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("host contains invalid characters".to_string());
    }
    if host.contains('/') || host.contains('?') || host.contains('#') || host.contains('@') {
        return Err(
            "host must be a bare authority (no scheme, path, query, or userinfo)".to_string(),
        );
    }

    let normalized = normalize_host(host);
    validate_host(&normalized)?;
    Ok(normalized)
}

/// Publish the event-backed membership view after owner persistence.
///
/// Provisioning is already committed when this runs. Publication therefore
/// remains best-effort, matching every other membership mutation path: turning
/// a stored success into an HTTP failure would make create-only retries report
/// a misleading conflict while leaving clients without a repair path.
async fn publish_membership_snapshot_if_required(
    state: &Arc<AppState>,
    community: nimino_core::CommunityId,
    host: &str,
) {
    if !state.config.require_relay_membership {
        return;
    }

    let tenant = TenantContext::resolved(community, host);
    if let Err(error) =
        crate::handlers::side_effects::publish_nip43_membership_list(&tenant, state).await
    {
        warn!(
            community = %community,
            host,
            error = %error,
            "community provisioned but NIP-43 membership snapshot publication failed"
        );
    }
}

/// Validate and execute a relay-operator community provisioning request.
///
/// The caller is an HTTP operator endpoint, not the Nostr event ingest path.
/// That keeps the tenant data-plane fences unchanged: no relay-membership
/// bypass, no special event kind, no command routed ahead of moderation/write
/// blocks. The endpoint authenticates its NIP-98 signer first, then passes the
/// signer here for the deployment-level `RELAY_OPERATOR_PUBKEYS` allowlist.
///
/// The host and initial owner are created atomically. An existing host is a
/// conflict; ownership changes use the dedicated transfer endpoint.
pub async fn provision_community(
    state: &Arc<AppState>,
    operator_pubkey: &nostr::PublicKey,
    request: ProvisionCommunityRequest,
) -> Result<ProvisionCommunityResponse, String> {
    let operator_hex = operator_pubkey.to_hex();

    // Operator gate. Deliberately NOT a relay_members lookup: provisioning
    // authority spans tenants and lives in deployment config only. Empty
    // allowlist → everyone is rejected (fail closed).
    if !state
        .config
        .relay_operator_pubkeys
        .iter()
        .any(|pk| pk == &operator_hex)
    {
        return Err("actor not authorized: not a relay operator".to_string());
    }

    validate_host(&request.host)?;

    let initial_owner = validate_pubkey_hex(&request.initial_owner_pubkey)
        .ok_or_else(|| "invalid initial_owner_pubkey: expected 64-char hex pubkey".to_string())?;
    let existing = state
        .db
        .community_lifecycle_facts(&request.host)
        .await
        .map_err(|error| format!("failed to load community facts: {error}"))?;
    let owned = state
        .db
        .list_communities_owned_by(&initial_owner)
        .await
        .map_err(|error| format!("failed to load ownership facts: {error}"))?;
    let decision = super::ingest::call_community_policy(
        state,
        CommunityPolicyRequest::Lifecycle {
            request: CommunityLifecycleRequest {
                command: CommunityCommand::Create,
                state: if existing.is_some() {
                    CommunityState::Active
                } else {
                    CommunityState::Missing
                },
                actor_is_operator: true,
                actor_is_owner: false,
                owner_provided: true,
                owner_at_limit: i64::try_from(owned.len()).unwrap_or(i64::MAX)
                    >= nimino_db::relay_members::max_communities_per_owner(),
                protected_deployment: nimino_core::tenant::relay_url_authority(
                    &state.config.relay_url,
                ) == request.host,
            },
        },
    )
    .await
    .map_err(super::ingest::ingest_error_message)?;
    match decision {
        CommunityPolicyResult::Lifecycle {
            action: CommunityAction::Create,
            error: CommunityPolicyError::None,
        } => {}
        CommunityPolicyResult::Lifecycle { error, .. } => {
            return Err(format!(
                "community policy rejected provisioning ({error:?})"
            ));
        }
        CommunityPolicyResult::Scope { .. } => {
            return Err("Nim community policy returned an unexpected result".to_owned());
        }
    }
    let record = match state
        .db
        .create_community_with_owner(&request.host, &initial_owner)
        .await
        .map_err(|e| format!("failed to create community: {e}"))?
    {
        nimino_db::CreateCommunityWithOwnerResult::Created(record) => record,
        nimino_db::CreateCommunityWithOwnerResult::HostExists => {
            return Err("community already exists".to_string());
        }
        nimino_db::CreateCommunityWithOwnerResult::LimitReached => {
            return Err(
                "limit_reached: owner already owns the maximum number of communities".to_string(),
            );
        }
    };

    info!(
        operator = %operator_hex,
        community = %record.id,
        host = %record.host,
        owner = %initial_owner,
        "community created via operator endpoint"
    );
    publish_membership_snapshot_if_required(state, record.id, &record.host).await;

    Ok(ProvisionCommunityResponse {
        community_id: record.id.to_string(),
        host: record.host,
        status: "created",
        owner_pubkey: initial_owner,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisioning_request_rejects_the_removed_convergence_mode() {
        assert!(
            serde_json::from_value::<ProvisionCommunityRequest>(serde_json::json!({
                "host": "acme.example",
                "initial_owner_pubkey": "00".repeat(32),
                "create_only": false
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<ProvisionCommunityRequest>(serde_json::json!({
                "host": "acme.example"
            }))
            .is_err()
        );
    }

    #[test]
    fn host_valid_bare_domain() {
        assert!(validate_host("acme.communities.nimino.xyz").is_ok());
    }

    #[test]
    fn host_valid_with_port() {
        assert!(validate_host("localhost:3000").is_ok());
    }

    #[test]
    fn host_rejects_empty() {
        assert!(validate_host("").is_err());
    }

    #[test]
    fn host_rejects_uppercase() {
        assert!(validate_host("Acme.example").is_err());
    }

    #[test]
    fn host_rejects_default_port() {
        assert!(validate_host("acme.example:443").is_err());
        assert!(validate_host("acme.example:80").is_err());
    }

    #[test]
    fn host_rejects_trailing_dot() {
        assert!(validate_host("acme.example.").is_err());
    }

    #[test]
    fn host_rejects_scheme_path_userinfo() {
        assert!(validate_host("wss://acme.example").is_err());
        assert!(validate_host("acme.example/path").is_err());
        assert!(validate_host("user@acme.example").is_err());
        assert!(validate_host("acme.example?x=1").is_err());
        assert!(validate_host("acme.example#frag").is_err());
    }

    #[test]
    fn host_rejects_invalid_authorities() {
        assert!(validate_host(":").is_err());
        assert!(validate_host("example..com").is_err());
        assert!(validate_host("foo_bar.example").is_err());
        assert!(validate_host("-bad.example").is_err());
        assert!(validate_host("bad-.example").is_err());
        assert!(validate_host("example.com:99999").is_err());
        assert!(validate_host("[::1").is_err());
        assert!(validate_host("[not-ipv6]").is_err());
        assert!(validate_host(&format!("{}.example", "a".repeat(64))).is_err());
    }

    #[test]
    fn host_rejects_whitespace_and_control() {
        assert!(validate_host("acme .example").is_err());
        assert!(validate_host("acme\n.example").is_err());
    }

    #[test]
    fn host_rejects_oversized() {
        let long = format!("{}.example", "a".repeat(260));
        assert!(validate_host(&long).is_err());
    }

    #[test]
    fn host_accepts_ipv6_bracket_literal() {
        assert!(validate_host("[::1]:3000").is_ok());
    }

    #[test]
    fn candidate_host_normalizes_safe_variants() {
        assert_eq!(
            normalize_candidate_host("Acme.Example:443").unwrap(),
            "acme.example"
        );
        assert_eq!(
            normalize_candidate_host("acme.example.").unwrap(),
            "acme.example"
        );
    }

    #[test]
    fn candidate_host_rejects_non_authorities() {
        assert!(normalize_candidate_host("https://acme.example").is_err());
        assert!(normalize_candidate_host("acme.example/path").is_err());
        assert!(normalize_candidate_host("acme .example").is_err());
    }
}
