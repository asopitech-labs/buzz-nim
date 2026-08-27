//! Typed community-policy facts and decisions crossing the process boundary.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Durable lifecycle state supplied by the community storage adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityState {
    /// No community reserves the requested host.
    Missing,
    /// The community admits normal traffic.
    Active,
    /// The host remains reserved but traffic is disabled.
    Archived,
    /// Destructive deletion is in progress.
    Deleting,
    /// The host is permanently tombstoned.
    Deleted,
}

/// Lifecycle command requested by an authenticated operator adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityCommand {
    /// Reserve a new host with an initial owner.
    Create,
    /// Stop admitting traffic while retaining the host.
    Archive,
    /// Restore an archived host to active admission.
    Unarchive,
}

/// Verified facts required for a community lifecycle decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunityLifecycleRequest {
    /// Requested lifecycle command.
    pub command: CommunityCommand,
    /// Current durable lifecycle state.
    pub state: CommunityState,
    /// Whether the authenticated signer is a deployment operator.
    pub actor_is_operator: bool,
    /// Whether the asserted actor is the community owner.
    pub actor_is_owner: bool,
    /// Whether create supplied a validated initial owner.
    pub owner_provided: bool,
    /// Whether that owner already reached the ownership limit.
    pub owner_at_limit: bool,
    /// Whether this is the deployment's protected community.
    pub protected_deployment: bool,
}

/// Server-derived tenant and resource facts for an isolation decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommunityScopeRequest {
    /// Community resolved from the connection host by the server.
    pub request_community: Uuid,
    /// Community read from the scoped resource, or absent when not found.
    pub resource_community: Option<Uuid>,
}

/// Typed community-policy decision requested from the Nimino core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CommunityPolicyRequest {
    /// Decide a create, archive, or unarchive transition.
    Lifecycle {
        /// Verified authority and lifecycle facts.
        request: CommunityLifecycleRequest,
    },
    /// Decide whether a resource belongs to the host-derived tenant.
    Scope {
        /// Verified request and resource communities.
        request: CommunityScopeRequest,
    },
}

/// Effect selected by Nimino for a community lifecycle request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityAction {
    /// No effect is allowed.
    Reject,
    /// Create the community and initial owner atomically.
    Create,
    /// Persist the first archive timestamp.
    Archive,
    /// Clear the archive timestamp.
    Unarchive,
    /// The requested state is already durable.
    Noop,
}

/// Stable community-policy validation failures returned by Nimino.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommunityPolicyError {
    /// The decision is valid.
    None,
    /// The authenticated actor is not a deployment operator.
    NotOperator,
    /// The asserted actor is not the current owner.
    NotOwner,
    /// Create omitted an initial owner.
    OwnerRequired,
    /// The intended owner reached the ownership limit.
    OwnerLimit,
    /// The host is already reserved in any lifecycle state.
    HostReserved,
    /// The deployment community cannot transition through this path.
    ProtectedCommunity,
    /// The durable state does not accept the command.
    InvalidState,
    /// No scoped resource exists.
    ResourceMissing,
    /// The resource belongs to another community.
    TenantMismatch,
}

/// Typed result of a Nimino community-policy decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CommunityPolicyResult {
    /// Lifecycle transition result.
    Lifecycle {
        /// Selected persistence effect.
        action: CommunityAction,
        /// Validation outcome.
        error: CommunityPolicyError,
    },
    /// Tenant isolation result.
    Scope {
        /// Whether the scoped operation may continue.
        allowed: bool,
        /// Validation outcome.
        error: CommunityPolicyError,
    },
}
