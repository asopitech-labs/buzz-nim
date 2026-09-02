//! Typed moderation facts and decisions crossing the process boundary.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::membership::MembershipRole;

/// Tenant-scoped target carried by a NIP-56 report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationReportTarget {
    /// Stored event target.
    Event,
    /// Community-local pubkey target.
    Pubkey,
    /// Tenant-scoped media blob target.
    Blob,
}

/// Accepted NIP-56 report classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationReportType {
    /// Illegal content.
    Illegal,
    /// Nudity.
    Nudity,
    /// Malware.
    Malware,
    /// Spam.
    Spam,
    /// Impersonation.
    Impersonation,
    /// Profanity.
    Profanity,
    /// Other policy concern.
    Other,
}

/// Restriction transition requested by a moderation command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationRestrictionCommand {
    /// Apply a permanent or expiring ban.
    Ban,
    /// Lift an active ban.
    Unban,
    /// Apply a write timeout.
    Timeout,
    /// Clear an active timeout.
    Untimeout,
}

/// Durable report status requested by a resolution command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationResolutionStatus {
    /// The report was acted on or escalated.
    Resolved,
    /// The report was dismissed.
    Dismissed,
}

/// Moderator-selected report resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationResolutionAction {
    /// Delete the reported event.
    Delete,
    /// Remove the target from a channel.
    Kick,
    /// Ban the target.
    Ban,
    /// Time out the target.
    Timeout,
    /// Dismiss the report.
    Dismiss,
    /// Escalate to the deployment safety lane.
    Escalate,
}

/// Runtime surface subject to an active moderation restriction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationEnforcementOperation {
    /// Connection or credential admission; bans deny, timeouts do not.
    Authenticate,
    /// Content write; bans and timeouts deny.
    Write,
}

/// Verified facts required to accept a report into the moderation queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModerationReportRequest {
    /// Community resolved from the request host.
    pub request_community: Uuid,
    /// Community of the resolved event/blob or the local pubkey target.
    pub target_community: Option<Uuid>,
    /// Whether the target resolved inside that community.
    pub target_exists: bool,
    /// Whether the authenticated reporter is the report target.
    pub reporter_is_target: bool,
    /// Whether the signed report id already exists in this community.
    pub duplicate: bool,
    /// Resolved target class.
    pub target_kind: ModerationReportTarget,
    /// Validated report classification.
    pub report_type: ModerationReportType,
}

/// Verified authority, restriction, and time facts for a ban or timeout command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModerationRestrictionRequest {
    /// Requested restriction transition.
    pub command: ModerationRestrictionCommand,
    /// Community resolved from the request host.
    pub request_community: Uuid,
    /// Provenance of the actor's relay-membership role.
    pub actor_role_community: Option<Uuid>,
    /// Provenance of the target's relay-membership role.
    pub target_role_community: Option<Uuid>,
    /// Provenance of the actor's raw restriction row.
    pub actor_restriction_community: Option<Uuid>,
    /// Provenance of the target's raw restriction row.
    pub target_restriction_community: Option<Uuid>,
    /// Actor's active relay-membership role.
    pub actor_role: MembershipRole,
    /// Target's active relay-membership role, or none.
    pub target_role: MembershipRole,
    /// Whether an actor restriction row exists.
    pub actor_restriction_exists: bool,
    /// Raw actor ban flag.
    pub actor_ban_set: bool,
    /// Raw actor ban expiry, in Unix seconds.
    pub actor_ban_expires_at: Option<i64>,
    /// Whether a target restriction row exists.
    pub target_restriction_exists: bool,
    /// Raw target ban flag.
    pub target_ban_set: bool,
    /// Raw target ban expiry, in Unix seconds.
    pub target_ban_expires_at: Option<i64>,
    /// Raw target timeout expiry, in Unix seconds.
    pub target_muted_until: Option<i64>,
    /// Whether actor and target are the same verified pubkey.
    pub actor_is_target: bool,
    /// Signed command timestamp, in Unix seconds.
    pub created_at_seconds: i64,
    /// Clock value acquired by the Rust adapter, in Unix seconds.
    pub now_seconds: i64,
    /// Requested ban or timeout expiry, in Unix seconds.
    pub requested_expires_at: Option<i64>,
}

/// Verified authority and report facts for a resolution command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModerationResolutionRequest {
    /// Community resolved from the request host.
    pub request_community: Uuid,
    /// Provenance of the actor's relay-membership role.
    pub actor_role_community: Option<Uuid>,
    /// Provenance of the actor's raw restriction row.
    pub actor_restriction_community: Option<Uuid>,
    /// Provenance of the loaded report.
    pub report_community: Option<Uuid>,
    /// Actor's active relay-membership role.
    pub actor_role: MembershipRole,
    /// Whether an actor restriction row exists.
    pub actor_restriction_exists: bool,
    /// Raw actor ban flag.
    pub actor_ban_set: bool,
    /// Raw actor ban expiry, in Unix seconds.
    pub actor_ban_expires_at: Option<i64>,
    /// Whether the scoped report exists.
    pub report_exists: bool,
    /// Whether the report is currently open.
    pub report_open: bool,
    /// Signed command timestamp, in Unix seconds.
    pub created_at_seconds: i64,
    /// Clock value acquired by the Rust adapter, in Unix seconds.
    pub now_seconds: i64,
    /// Requested durable report status.
    pub status: ModerationResolutionStatus,
    /// Requested resolution action.
    pub action: ModerationResolutionAction,
}

/// Raw restriction facts used for runtime admission and write enforcement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModerationEnforcementRequest {
    /// Runtime surface being authorized.
    pub operation: ModerationEnforcementOperation,
    /// Community resolved from the request host.
    pub request_community: Uuid,
    /// Provenance of the principal's raw restriction row.
    pub principal_restriction_community: Option<Uuid>,
    /// Provenance of the attested owner's raw restriction row.
    pub owner_restriction_community: Option<Uuid>,
    /// Whether a principal restriction row exists.
    pub principal_restriction_exists: bool,
    /// Raw principal ban flag.
    pub principal_ban_set: bool,
    /// Raw principal ban expiry, in Unix seconds.
    pub principal_ban_expires_at: Option<i64>,
    /// Raw principal timeout expiry, in Unix seconds.
    pub principal_muted_until: Option<i64>,
    /// Whether NIP-OA cryptographically attested an owner.
    pub owner_attested: bool,
    /// Whether an owner restriction row exists.
    pub owner_restriction_exists: bool,
    /// Raw owner ban flag.
    pub owner_ban_set: bool,
    /// Raw owner ban expiry, in Unix seconds.
    pub owner_ban_expires_at: Option<i64>,
    /// Clock value acquired by the Rust adapter, in Unix seconds.
    pub now_seconds: i64,
}

/// Typed moderation decision requested from the Nimino core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ModerationPolicyRequest {
    /// Decide whether to queue a report.
    Report {
        /// Verified target and idempotency facts.
        request: ModerationReportRequest,
    },
    /// Decide a ban, unban, timeout, or untimeout transition.
    Restriction {
        /// Verified role, restriction, provenance, and time facts.
        request: ModerationRestrictionRequest,
    },
    /// Decide an open-report resolution.
    Resolution {
        /// Verified role, restriction, report, provenance, and time facts.
        request: ModerationResolutionRequest,
    },
    /// Decide connection or write enforcement from raw restriction facts.
    Enforcement {
        /// Verified principal, optional owner, provenance, and clock facts.
        request: ModerationEnforcementRequest,
    },
}

/// Persistence effect selected by Nimino.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationEffect {
    /// No effect is allowed.
    Reject,
    /// Insert a report into the tenant queue.
    QueueReport,
    /// Apply a ban.
    ApplyBan,
    /// Lift a ban.
    LiftBan,
    /// Apply a timeout.
    ApplyTimeout,
    /// Clear a timeout.
    ClearTimeout,
    /// Atomically close an open report with an audit row.
    ResolveReport,
    /// Admit the requested runtime operation.
    Allow,
    /// Deny because the principal or attested owner has an active ban.
    DenyBan,
    /// Deny a content write because the principal has an active timeout.
    DenyTimeout,
}

/// Authority recorded for an accepted moderation decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationAuthority {
    /// No authority matched.
    None,
    /// Any authenticated, non-self reporter.
    Reporter,
    /// Community owner.
    CommunityOwner,
    /// Community administrator.
    CommunityAdmin,
}

/// Stable audit action selected by Nimino for an accepted command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationAuditAction {
    /// Report intake writes no moderation action row.
    None,
    /// Ban enforcement.
    Ban,
    /// Ban reversal.
    Unban,
    /// Timeout enforcement.
    Timeout,
    /// Timeout reversal.
    Untimeout,
    /// Dismiss-report resolution.
    DismissReport,
    /// Safety escalation.
    Escalate,
    /// Resolution decision to delete.
    ResolveDelete,
    /// Resolution decision to kick.
    ResolveKick,
    /// Resolution decision to ban.
    ResolveBan,
    /// Resolution decision to time out.
    ResolveTimeout,
}

/// Stable moderation-policy failures returned by Nimino.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModerationPolicyError {
    /// The decision is valid.
    None,
    /// The target or report does not exist in the request scope.
    ResourceMissing,
    /// A loaded fact belongs to another community.
    TenantMismatch,
    /// Supplied existence and provenance facts contradict each other.
    FactConflict,
    /// The acting moderator has an active ban.
    ActorBanned,
    /// The actor is neither a community owner nor administrator.
    NotAuthorized,
    /// An administrator attempted to restrict an owner or administrator.
    ProtectedTarget,
    /// The actor attempted to report or restrict itself.
    SelfTarget,
    /// The signed command timestamp is outside the freshness window.
    StaleCommand,
    /// Timeout omitted its required expiry.
    ExpirationRequired,
    /// The requested expiry is not in the future.
    ExpirationElapsed,
    /// The report or active restriction duplicates current state.
    Duplicate,
    /// No active ban exists to lift.
    NotBanned,
    /// No active timeout exists to clear.
    NotTimedOut,
    /// The report is already closed.
    ReportClosed,
    /// Resolution status and action do not form a valid pair.
    ResolutionPair,
}

/// Typed result of a Nimino moderation decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ModerationPolicyResult {
    /// Report intake result.
    Report {
        /// Selected persistence effect.
        effect: ModerationEffect,
        /// Matched authority.
        authority: ModerationAuthority,
        /// Selected audit action.
        audit_action: ModerationAuditAction,
        /// Validation outcome.
        error: ModerationPolicyError,
    },
    /// Restriction transition result.
    Restriction {
        /// Selected persistence effect.
        effect: ModerationEffect,
        /// Matched authority.
        authority: ModerationAuthority,
        /// Selected audit action.
        audit_action: ModerationAuditAction,
        /// Validation outcome.
        error: ModerationPolicyError,
    },
    /// Report resolution result.
    Resolution {
        /// Selected persistence effect.
        effect: ModerationEffect,
        /// Matched authority.
        authority: ModerationAuthority,
        /// Selected audit action.
        audit_action: ModerationAuditAction,
        /// Validation outcome.
        error: ModerationPolicyError,
    },
    /// Runtime enforcement result.
    Enforcement {
        /// Selected admission effect.
        effect: ModerationEffect,
        /// Matched authority; always none for state enforcement.
        authority: ModerationAuthority,
        /// Selected audit action; always none for state enforcement.
        audit_action: ModerationAuditAction,
        /// Validation outcome.
        error: ModerationPolicyError,
    },
}
