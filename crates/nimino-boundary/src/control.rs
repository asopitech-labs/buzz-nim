//! Typed replicated-control policy boundary owned by the Nimino core.

use serde::{Deserialize, Serialize};

/// Voter configuration phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlVoterPhase {
    /// One stable voter set is authoritative.
    StableOld,
    /// Both old and new voter sets require quorum.
    Joint,
    /// The replacement voter set is authoritative.
    StableNew,
}

/// Replicated entry kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlEntryKind {
    /// Opaque state-machine command.
    Command,
    /// Enter joint consensus with `target_voters`.
    BeginJoint,
    /// Finalize the joint voter transition.
    Finalize,
}

/// Effect selected by Nim control policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlEffect {
    /// Reject without mutation.
    Reject,
    /// Persist one term vote.
    Vote,
    /// Persist quorum-backed leader authority.
    ElectLeader,
    /// Append one uncommitted entry.
    Append,
    /// Return an identical committed command without appending it again.
    Replay,
    /// Commit one entry.
    Commit,
    /// Apply one committed entry.
    Apply,
    /// Install one applied snapshot.
    Snapshot,
}

/// Storage action selected by Nim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlStoreActionKind {
    /// Compare-and-set metadata.
    Metadata,
    /// Replace an uncommitted log suffix.
    Log,
    /// Install and compact a snapshot.
    Snapshot,
}

/// Stable control-policy failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlStateError {
    /// No error.
    None,
    /// Voter configuration is invalid.
    InvalidVoters,
    /// Election term is stale.
    StaleTerm,
    /// Candidate is not an active voter.
    CandidateNotVoter,
    /// Candidate log is behind the voter.
    CandidateLogStale,
    /// Required quorum is absent.
    QuorumRequired,
    /// The caller is not the elected leader.
    LeaderRequired,
    /// Leader authority is stale.
    AuthorityStale,
    /// An uncommitted entry already exists.
    PendingEntry,
    /// Entry kind is invalid for the voter phase.
    EntryKindInvalid,
    /// Command identity is missing.
    CommandRequired,
    /// A committed command reused an id with different content.
    CommandConflict,
    /// Control log has a gap.
    LogGap,
    /// Commit order is invalid.
    CommitOrder,
    /// Apply order is invalid.
    ApplyOrder,
    /// No fully applied prefix can be snapshotted.
    SnapshotUnavailable,
    /// Supplied facts conflict.
    FactConflict,
    /// Store execution failed.
    StoreFailure,
    /// Durable recovery state is corrupt.
    CorruptRecovery,
}

/// One replicated control entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlEntry {
    /// Contiguous one-based index.
    pub index: u64,
    /// Election term.
    pub term: u64,
    /// Voter epoch.
    pub voter_epoch: u64,
    /// Entry kind.
    pub kind: ControlEntryKind,
    /// Stable idempotency key.
    pub command_id: String,
    /// Opaque command payload.
    pub payload: String,
    /// Replacement voters for a begin-joint entry.
    pub target_voters: Vec<String>,
}

/// Durable control snapshot facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlSnapshotState {
    /// Last included index.
    pub last_included_index: u64,
    /// Term at the included index.
    pub last_included_term: u64,
    /// Installed voter epoch.
    pub voter_epoch: u64,
    /// Installed voter phase.
    pub phase: ControlVoterPhase,
    /// Old/stable voters.
    pub old_voters: Vec<String>,
    /// Joint/new voters.
    pub new_voters: Vec<String>,
    /// Opaque state-machine snapshot.
    pub state_payload: String,
}

/// Complete in-memory control state interpreted by Nim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlState {
    /// Whether construction and recovery invariants hold.
    pub valid: bool,
    /// Local metadata CAS revision.
    pub metadata_revision: u64,
    /// Current election term.
    pub term: u64,
    /// Persisted vote in the current term.
    pub voted_for: Option<String>,
    /// Current voter epoch.
    pub voter_epoch: u64,
    /// Voter phase.
    pub phase: ControlVoterPhase,
    /// Old/stable voters.
    pub old_voters: Vec<String>,
    /// Joint/new voters.
    pub new_voters: Vec<String>,
    /// Elected leader, empty when none.
    pub leader_id: String,
    /// Elected leader term.
    pub leader_term: u64,
    /// Authenticated quorum proof identities.
    pub leader_proof: Vec<String>,
    /// Last durable log index.
    pub last_index: u64,
    /// Highest committed index.
    pub commit_index: u64,
    /// Highest applied index.
    pub applied_index: u64,
    /// Latest installed snapshot.
    pub snapshot: Option<ControlSnapshotState>,
    /// Durable suffix after the snapshot.
    pub log: Vec<ControlEntry>,
}

/// One candidate vote request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlVoteRequest {
    /// Requested election term.
    pub term: u64,
    /// Candidate identity.
    pub candidate_id: String,
    /// Candidate last log index.
    pub last_index: u64,
    /// Candidate term at `last_index`.
    pub last_term: u64,
}

/// Quorum election proof.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlElectionRequest {
    /// Election term.
    pub term: u64,
    /// Candidate identity.
    pub candidate_id: String,
    /// Authenticated supporters.
    pub supporters: Vec<String>,
}

/// Leader append request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlAppendRequest {
    /// Leader identity.
    pub leader_id: String,
    /// Leader term.
    pub term: u64,
    /// Entry kind.
    pub kind: ControlEntryKind,
    /// Stable command identity.
    pub command_id: String,
    /// Opaque payload.
    pub payload: String,
    /// Replacement voters for begin-joint.
    pub target_voters: Vec<String>,
}

/// One authenticated follower-log replication request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlReplicationRequest {
    /// Current leader identity.
    pub leader_id: String,
    /// Current leader term.
    pub term: u64,
    /// Authenticated election supporters.
    pub supporters: Vec<String>,
    /// Durable prefix retained before installing `entry`.
    pub previous_index: u64,
    /// Exact leader entry to append after the retained prefix.
    pub entry: ControlEntry,
}

/// Quorum commit request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlCommitRequest {
    /// Entry index.
    pub index: u64,
    /// Leader identity.
    pub leader_id: String,
    /// Leader term.
    pub term: u64,
    /// Authenticated replication acknowledgements.
    pub supporters: Vec<String>,
}

/// Live supporter facts checked against the active voter phase by Nim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlQuorumRequest {
    /// Authenticated live supporter identities.
    pub supporters: Vec<String>,
}

/// Read-only quorum decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlQuorumDecision {
    /// Whether the supporters satisfy the current voter phase.
    pub granted: bool,
    /// Stable validation failure.
    pub error: ControlStateError,
}

/// Exact storage action selected by a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlStoreAction {
    /// Storage class.
    pub kind: ControlStoreActionKind,
    /// Metadata revision required by a CAS.
    pub expected_metadata_revision: u64,
    /// Prefix retained by a log suffix replacement.
    pub previous_index: u64,
}

/// Persistence-first control transition plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlPlan {
    /// Selected effect.
    pub effect: ControlEffect,
    /// Stable failure.
    pub error: ControlStateError,
    /// State retained on storage failure.
    pub before_state: ControlState,
    /// State accepted after storage success.
    pub next_state: ControlState,
    /// Ordered adapter actions.
    pub actions: Vec<ControlStoreAction>,
    /// Entry emitted by apply.
    pub applied_entry: Option<ControlEntry>,
}

/// Settled control transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlDecision {
    /// Settled effect.
    pub effect: ControlEffect,
    /// Stable failure.
    pub error: ControlStateError,
    /// Authoritative post-settlement state.
    pub state: ControlState,
    /// Applied command when present.
    pub applied_entry: Option<ControlEntry>,
}

/// Durable bytes decoded by the Rust store and interpreted by Nim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlRecoveryInput {
    /// Metadata CAS revision.
    pub metadata_revision: u64,
    /// Durable term.
    pub term: u64,
    /// Durable vote.
    pub voted_for: Option<String>,
    /// Durable commit index.
    pub commit_index: u64,
    /// Durable applied index.
    pub applied_index: u64,
    /// Bootstrap voter set when no snapshot exists.
    pub initial_voters: Vec<String>,
    /// Installed snapshot.
    pub snapshot: Option<ControlSnapshotState>,
    /// Contiguous durable suffix.
    pub entries: Vec<ControlEntry>,
}

/// Recovery result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlRecovery {
    /// Stable recovery failure.
    pub error: ControlStateError,
    /// Recovered state.
    pub state: ControlState,
}

/// Typed replicated-control policy request.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ControlPolicyRequest {
    /// Decide and persist one vote.
    Vote {
        /// Current control state.
        state: ControlState,
        /// Candidate facts.
        request: ControlVoteRequest,
    },
    /// Install quorum-backed leader authority.
    Election {
        /// Current control state.
        state: ControlState,
        /// Quorum election proof.
        request: ControlElectionRequest,
    },
    /// Plan one leader append.
    Append {
        /// Current control state.
        state: ControlState,
        /// Leader append facts.
        request: ControlAppendRequest,
    },
    /// Replace an uncommitted follower suffix with one leader entry.
    Replicate {
        /// Current control state.
        state: ControlState,
        /// Authenticated leader replication facts.
        request: ControlReplicationRequest,
    },
    /// Plan one quorum commit.
    Commit {
        /// Current control state.
        state: ControlState,
        /// Quorum commit facts.
        request: ControlCommitRequest,
    },
    /// Check live supporters without mutating control state.
    Quorum {
        /// Current control state.
        state: ControlState,
        /// Authenticated live supporters.
        request: ControlQuorumRequest,
    },
    /// Plan the next committed apply.
    Apply {
        /// Current control state.
        state: ControlState,
    },
    /// Plan a fully applied snapshot.
    Snapshot {
        /// Current control state.
        state: ControlState,
        /// Opaque applied state.
        state_payload: String,
    },
    /// Settle a storage-backed plan.
    Settle {
        /// Previously selected plan.
        plan: ControlPlan,
        /// Whether all requested storage actions succeeded.
        store_succeeded: bool,
    },
    /// Recover from local durable facts.
    Recover {
        /// Durable local recovery facts.
        input: ControlRecoveryInput,
    },
}

/// Typed replicated-control policy response.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum ControlPolicyResult {
    /// Persistence-first transition plan.
    Plan {
        /// Persistence-first plan.
        plan: ControlPlan,
    },
    /// Settled transition decision.
    Settle {
        /// Settled decision.
        result: ControlDecision,
    },
    /// Durable recovery result.
    Recover {
        /// Recovery result.
        result: ControlRecovery,
    },
    /// Read-only quorum decision.
    Quorum {
        /// Current voter-phase decision.
        result: ControlQuorumDecision,
    },
}
