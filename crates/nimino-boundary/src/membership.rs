//! Typed membership-policy facts and decisions crossing the process boundary.

use serde::{Deserialize, Serialize};

/// Membership role used by channel and relay-roster decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipRole {
    /// No active membership row exists.
    None,
    /// Highest channel and relay-roster authority.
    Owner,
    /// Delegated channel and relay-roster administrator.
    Admin,
    /// Ordinary participant.
    Member,
    /// Read-only channel participant.
    Guest,
    /// Automated channel participant.
    Bot,
}

/// Channel visibility fact supplied by the storage adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelVisibility {
    /// Any authenticated actor may join or add an ordinary role.
    Open,
    /// Joining requires an existing member to add the target.
    Private,
}

/// Membership transition requested by a channel or relay adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipCommand {
    /// Join an open channel as oneself.
    Join,
    /// Add or reactivate a membership.
    Add,
    /// Change an active member's role.
    ChangeRole,
    /// Remove a membership.
    Remove,
    /// Leave a channel as oneself.
    Leave,
}

/// Agent-controlled policy for third-party channel additions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentAddPolicy {
    /// Any otherwise-authorized actor may add the agent.
    Anyone,
    /// Only the agent's verified owner may add it.
    OwnerOnly,
    /// The agent refuses third-party additions.
    Nobody,
}

/// Verified facts required for a channel membership transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ChannelMembershipRequest {
    /// Requested transition.
    pub command: MembershipCommand,
    /// Current channel visibility.
    pub visibility: ChannelVisibility,
    /// Actor's active role, or [`MembershipRole::None`].
    pub actor_role: MembershipRole,
    /// Target's active role, or [`MembershipRole::None`].
    pub target_role: MembershipRole,
    /// Explicit requested role, or [`MembershipRole::None`] when omitted.
    pub requested_role: MembershipRole,
    /// Whether actor and target are the same verified pubkey.
    pub actor_is_target: bool,
    /// Whether the actor owns the target agent.
    pub actor_owns_target_agent: bool,
    /// Whether the target is an agent.
    pub target_is_agent: bool,
    /// Target agent's channel-add policy.
    pub target_add_policy: AgentAddPolicy,
    /// Number of active channel owners observed under the membership lock.
    pub owner_count: i32,
}

/// Verified facts required for a relay-roster transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelayMembershipRequest {
    /// Requested transition; join and leave are invalid for relay administration.
    pub command: MembershipCommand,
    /// Actor's current relay-roster role.
    pub actor_role: MembershipRole,
    /// Target's current relay-roster role.
    pub target_role: MembershipRole,
    /// Explicit requested role, or [`MembershipRole::None`] for add-member defaulting.
    pub requested_role: MembershipRole,
    /// Whether actor and target are the same verified pubkey.
    pub actor_is_target: bool,
}

/// Invite operation requested by an authenticated HTTP adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteCommand {
    /// Mint a new durable invite.
    Mint,
    /// Claim a presented durable invite.
    Claim,
}

/// Durable invite state observed inside the claim transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteState {
    /// The invite exists, is live, and has capacity.
    Valid,
    /// The invite expiry has passed.
    Expired,
    /// The invite has consumed all uses.
    Exhausted,
    /// No scoped invite matches the verified token hash.
    Invalid,
}

/// Verified facts required for invite mint or claim policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvitePolicyRequest {
    /// Requested invite operation.
    pub command: InviteCommand,
    /// Actor's relay-roster role; ignored for claims.
    pub actor_role: MembershipRole,
    /// Durable invite state; ignored for minting.
    pub invite_state: InviteState,
    /// Requested lifetime in seconds; ignored for claims.
    pub ttl_seconds: i64,
    /// Requested use limit, or unlimited when absent; ignored for claims.
    pub max_uses: Option<i32>,
    /// Whether the claimant already belongs to the relay roster.
    pub already_member: bool,
    /// Whether the deployment requires policy acceptance.
    pub policy_required: bool,
    /// Whether the Rust crypto adapter verified the acceptance receipt.
    pub policy_accepted: bool,
}

/// Verified facts required for atomic relay ownership transfer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnershipTransferRequest {
    /// Whether the authenticated signer is a deployment operator.
    pub actor_is_operator: bool,
    /// Whether a current owner row exists.
    pub owner_present: bool,
    /// Whether the caller's expected owner matches the locked owner row.
    pub expected_owner_matches: bool,
    /// Whether the transferee is already the sole owner.
    pub new_owner_is_current_owner: bool,
    /// Whether the transferee reached the ownership limit.
    pub new_owner_at_limit: bool,
}

/// Typed membership-policy decision requested from the Nimino core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MembershipPolicyRequest {
    /// Decide a channel join, add, role change, remove, or leave.
    Channel {
        /// Verified channel and membership facts.
        request: ChannelMembershipRequest,
    },
    /// Decide a relay-roster add, role change, or remove.
    Relay {
        /// Verified relay-roster facts.
        request: RelayMembershipRequest,
    },
    /// Decide a durable invite mint or claim.
    Invite {
        /// Verified authority, limit, receipt, and durable invite facts.
        request: InvitePolicyRequest,
    },
    /// Decide an atomic ownership transfer.
    OwnershipTransfer {
        /// Verified operator, owner, compare-and-swap, and quota facts.
        request: OwnershipTransferRequest,
    },
}

/// Effect selected by Nimino for a membership policy request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipAction {
    /// No effect is allowed.
    Reject,
    /// Insert a channel or relay membership.
    Insert,
    /// Update an active membership role.
    UpdateRole,
    /// Remove an active membership.
    Remove,
    /// The requested state is already durable.
    Noop,
    /// Mint and persist a durable invite.
    Mint,
    /// Atomically claim the invite and join the relay roster.
    Join,
    /// Atomically promote the new owner and demote the old owner.
    Transfer,
}

/// Stable membership-policy validation failures returned by Nimino.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MembershipPolicyError {
    /// The decision is valid.
    None,
    /// The authenticated actor is not a deployment operator.
    NotOperator,
    /// The actor lacks the required role or ownership capability.
    NotAuthorized,
    /// A self-leave target has no active membership.
    NotMember,
    /// A private channel cannot be self-joined.
    InviteRequired,
    /// The command supplied or implied an invalid role.
    RoleInvalid,
    /// No active target membership exists.
    TargetMissing,
    /// Removing or demoting the target would orphan the channel.
    LastOwner,
    /// A command that requires or forbids self-targeting violated that rule.
    SelfMutation,
    /// A relay owner cannot be removed or changed by roster administration.
    OwnerProtected,
    /// The target agent's channel-add policy refused the actor.
    AgentAddDenied,
    /// No durable invite matches the scoped token hash.
    InviteInvalid,
    /// The durable invite has expired.
    InviteExpired,
    /// The durable invite has no remaining uses.
    InviteExhausted,
    /// Required join-policy acceptance was not verified.
    PolicyRequired,
    /// Invite lifetime or use limit is outside the v1 bounds.
    MintBounds,
    /// Ownership transfer found no current owner.
    OwnerMissing,
    /// Ownership changed since the caller observed it.
    OwnerConflict,
    /// The intended owner reached the ownership limit.
    OwnerLimit,
}

/// Typed result of a Nimino membership-policy decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MembershipPolicyResult {
    /// Channel membership transition result.
    Channel {
        /// Selected persistence effect.
        action: MembershipAction,
        /// Validation outcome.
        error: MembershipPolicyError,
        /// Role selected or preserved by the decision.
        effective_role: MembershipRole,
    },
    /// Relay-roster transition result.
    Relay {
        /// Selected persistence effect.
        action: MembershipAction,
        /// Validation outcome.
        error: MembershipPolicyError,
        /// Role selected or preserved by the decision.
        effective_role: MembershipRole,
    },
    /// Invite mint or claim result.
    Invite {
        /// Selected persistence effect.
        action: MembershipAction,
        /// Validation outcome.
        error: MembershipPolicyError,
        /// Claimed role, or none for a mint or rejection.
        effective_role: MembershipRole,
    },
    /// Ownership transfer result.
    OwnershipTransfer {
        /// Selected persistence effect.
        action: MembershipAction,
        /// Validation outcome.
        error: MembershipPolicyError,
        /// New owner role, or none for a rejection.
        effective_role: MembershipRole,
    },
}
