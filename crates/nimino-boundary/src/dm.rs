//! Typed direct-message policy facts and decisions crossing the process boundary.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Direct-message mutation requested by an authenticated command adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmCommand {
    /// Open or restore the DM with an exact participant set.
    Open,
    /// Open a separate DM with an expanded participant set.
    Add,
    /// Hide a DM from the actor's own sidebar.
    Hide,
}

/// Effect selected by Nimino for a DM mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmAction {
    /// No effect is allowed.
    Reject,
    /// Create a DM for the canonical participant set.
    Create,
    /// Return the already-visible DM for that set.
    Reuse,
    /// Clear the actor's hidden marker and return the existing DM.
    Unhide,
    /// Set the actor's hidden marker.
    Hide,
    /// The requested visibility state is already durable.
    Noop,
}

/// DM read surface requested by a relay adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmAccessOperation {
    /// Read DM metadata or events.
    Read,
    /// Write an event to a DM.
    Write,
    /// Read a viewer-owned hidden-DM snapshot.
    Visibility,
}

/// Verified facts required for a DM mutation decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DmMutationRequest {
    /// Requested mutation.
    pub command: DmCommand,
    /// Community resolved from the request host.
    pub request_community: Uuid,
    /// Source channel provenance, when the command targets an existing DM.
    pub source_community: Option<Uuid>,
    /// Existing destination provenance for the canonical participant set.
    pub destination_community: Option<Uuid>,
    /// Whether the source channel exists.
    pub source_exists: bool,
    /// Whether the source channel is a DM.
    pub source_is_dm: bool,
    /// Whether the actor is an active source participant.
    pub actor_is_source_participant: bool,
    /// Whether the source DM is already hidden for the actor.
    pub source_actor_hidden: bool,
    /// Whether the canonical destination participant set includes the actor.
    pub actor_included: bool,
    /// Number of unique participants in the destination set.
    pub participant_count: i32,
    /// Number of unique participants added to the source set.
    pub new_participant_count: i32,
    /// Whether a DM already exists for the destination set.
    pub destination_exists: bool,
    /// Whether the existing destination is a DM.
    pub destination_is_dm: bool,
    /// Whether the destination DM is hidden for the actor.
    pub destination_actor_hidden: bool,
}

/// Verified facts required for a DM read or write decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DmAccessRequest {
    /// Requested access surface.
    pub operation: DmAccessOperation,
    /// Community resolved from the request host.
    pub request_community: Uuid,
    /// Community carried by the loaded resource.
    pub resource_community: Option<Uuid>,
    /// Whether the resource exists.
    pub resource_exists: bool,
    /// Whether the resource is a DM channel; ignored for visibility snapshots.
    pub channel_is_dm: bool,
    /// Whether the actor is an active DM participant.
    pub actor_is_participant: bool,
    /// Whether the visibility snapshot's `p` owner equals the reader.
    pub actor_is_viewer: bool,
}

/// Typed DM-policy decision requested from the Nimino core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DmPolicyRequest {
    /// Decide open, expanded-open, or hide.
    Mutation {
        /// Verified participant, provenance, and visibility facts.
        request: DmMutationRequest,
    },
    /// Decide participant or viewer access.
    Access {
        /// Verified tenant, resource, participant, and viewer facts.
        request: DmAccessRequest,
    },
}

/// Stable DM-policy failures returned by Nimino.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DmPolicyError {
    /// The decision is valid.
    None,
    /// No scoped source, destination, or visibility resource exists.
    ResourceMissing,
    /// The resource belongs to another community.
    TenantMismatch,
    /// The loaded channel is not a DM.
    NotDm,
    /// The actor is not an active participant.
    NotParticipant,
    /// The canonical participant set omitted the actor.
    ActorMissing,
    /// The unique participant count is outside 2 through 9.
    ParticipantCount,
    /// Add supplied no unique new participant.
    NoNewParticipant,
    /// Supplied existence and provenance facts contradict each other.
    FactConflict,
    /// The reader is not the visibility snapshot's viewer.
    ViewerMismatch,
}

/// Typed result of a Nimino DM-policy decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum DmPolicyResult {
    /// DM mutation result.
    Mutation {
        /// Selected persistence effect.
        action: DmAction,
        /// Validation outcome.
        error: DmPolicyError,
    },
    /// DM access result.
    Access {
        /// Whether the requested read or write may continue.
        allowed: bool,
        /// Validation outcome.
        error: DmPolicyError,
    },
}
