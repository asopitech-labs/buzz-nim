//! Quorum control-log replication over authenticated Chirps messages.
//!
//! Nim decides votes, quorum, append, commit, apply, and recovery. This crate
//! only schedules messages and executes the exact durable actions Nim returns.

#![deny(missing_docs)]

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};

use nimino_boundary::{
    BoundaryClient, BoundaryError, BoundaryRequest, BoundaryResult, CallContext,
    ControlAppendRequest, ControlCommitRequest, ControlDecision, ControlEffect, ControlEntry,
    ControlEntryKind, ControlPlan, ControlPolicyRequest, ControlPolicyResult,
    ControlQuorumDecision, ControlQuorumRequest, ControlRecoveryInput, ControlReplicationRequest,
    ControlSnapshotState, ControlState, ControlStateError, ControlStoreActionKind,
    ControlVoteRequest, ControlVoterPhase,
};
use nimino_chirps::{MeshClient, MeshRuntimeError, NodeId};
use nimino_store::{
    ControlLogEntry, ControlLogStorePort, ControlMetadata, ControlSnapshot, RecoveredControlState,
    StoreError,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

mod admission;
mod ephemeral;
mod lease;

pub use admission::*;
pub use ephemeral::*;
pub use lease::*;

const WIRE_PREFIX: &[u8] = b"NIMINO-CONTROL/1\n";
const MAX_CAPACITY: usize = 4_096;

/// Timing and queue bounds for one control runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControlRuntimeOptions {
    heartbeat_interval: Duration,
    election_timeout: Duration,
    policy_timeout: Duration,
    command_capacity: usize,
}

impl ControlRuntimeOptions {
    /// Creates explicit runtime bounds validated by [`ControlRuntime::start`].
    pub fn new(
        heartbeat_interval: Duration,
        election_timeout: Duration,
        policy_timeout: Duration,
        command_capacity: usize,
    ) -> Self {
        Self {
            heartbeat_interval,
            election_timeout,
            policy_timeout,
            command_capacity,
        }
    }

    fn validate(self) -> Result<(), ControlRuntimeError> {
        if self.heartbeat_interval.is_zero()
            || self.election_timeout < self.heartbeat_interval.saturating_mul(3)
            || self.policy_timeout.is_zero()
            || self.command_capacity == 0
            || self.command_capacity > MAX_CAPACITY
        {
            return Err(ControlRuntimeError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl Default for ControlRuntimeOptions {
    fn default() -> Self {
        Self::new(
            Duration::from_millis(100),
            Duration::from_secs(2),
            Duration::from_secs(2),
            64,
        )
    }
}

/// Observable control-plane authority and liveness.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlStatus {
    /// Whether the background task is alive.
    pub running: bool,
    /// Stable local Chirps identity.
    pub local_node_id: String,
    /// Current Nim-owned election term.
    pub term: u64,
    /// Current Nim-owned voter epoch.
    pub voter_epoch: u64,
    /// Elected leader, if one is durably accepted.
    pub leader_id: Option<String>,
    /// Whether fresh authenticated supporters satisfy Nim's quorum rule.
    pub quorum_available: bool,
    /// Highest durable committed control index.
    pub commit_index: u64,
    /// Highest locally applied control index.
    pub applied_index: u64,
    /// Most recent background failure.
    pub last_error: Option<String>,
}

/// One locally applied entry and whether it arrived through recovery/catch-up.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppliedControlEntry {
    /// Committed entry applied by the Nim state machine.
    pub entry: ControlEntry,
    /// `true` when applying it must not activate process-bound authority.
    pub recovered: bool,
}

/// Typed control-plane runtime failure.
#[derive(Debug, Error)]
pub enum ControlRuntimeError {
    /// Timings or queue bounds are empty or unsafe.
    #[error("invalid control runtime configuration")]
    InvalidConfiguration,
    /// A configured voter is not one canonical 16-byte lowercase hex identity.
    #[error("invalid control voter identity: {0}")]
    InvalidVoter(String),
    /// The local Chirps identity is absent from the configured voter set.
    #[error("local Chirps identity is not a configured control voter")]
    LocalNotVoter,
    /// The supervised Nim worker failed.
    #[error("Nim control policy failed: {0}")]
    Boundary(#[from] BoundaryError),
    /// The durable control store failed.
    #[error("control store failed: {0}")]
    Store(#[from] StoreError),
    /// Chirps rejected or stopped a transport operation.
    #[error("Chirps transport failed: {0}")]
    Transport(#[from] MeshRuntimeError),
    /// An authenticated payload violated the control wire contract.
    #[error("invalid control frame: {0}")]
    InvalidFrame(String),
    /// Nim returned a response for another control decision.
    #[error("Nim returned an unexpected control policy result")]
    UnexpectedPolicyResult,
    /// Nim rejected the supplied facts.
    #[error("Nim rejected control facts: {0:?}")]
    PolicyRejected(ControlStateError),
    /// A proposal was sent to a node without current leader authority.
    #[error("control proposal requires the current leader")]
    LeaderRequired,
    /// A proposal was attempted without a fresh live quorum.
    #[error("control proposal requires a live quorum")]
    QuorumRequired,
    /// Another proposal or inherited entry is awaiting replication.
    #[error("one control entry is already pending")]
    PendingEntry,
    /// The bounded command queue is full.
    #[error("control command queue is full")]
    Backpressure,
    /// A forwarded proposal did not complete within the control-plane bound.
    #[error("control proposal timed out")]
    ProposalTimeout,
    /// The current leader rejected a forwarded proposal.
    #[error("current leader rejected the control proposal")]
    RemoteProposalRejected,
    /// A durable payload is not valid UTF-8 for the versioned Nim boundary.
    #[error("control store contains non-UTF-8 policy data")]
    InvalidStoredText,
    /// The control task has stopped.
    #[error("control runtime stopped")]
    Stopped,
    /// The control task panicked.
    #[error("control task failed")]
    TaskFailed,
}

enum RuntimeCommand {
    Propose {
        command_id: String,
        payload: String,
        reply: oneshot::Sender<Result<ControlEntry, ControlRuntimeError>>,
    },
}

/// Cloneable proposal, applied-entry, and health facade.
#[derive(Clone)]
pub struct ControlClient {
    commands: mpsc::Sender<RuntimeCommand>,
    status: watch::Receiver<ControlStatus>,
    applied: broadcast::Sender<AppliedControlEntry>,
    proposal_timeout: Duration,
}

impl ControlClient {
    /// Proposes one opaque command and returns only after quorum commit and apply.
    pub async fn propose(
        &self,
        command_id: impl Into<String>,
        payload: impl Into<String>,
    ) -> Result<ControlEntry, ControlRuntimeError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .try_send(RuntimeCommand::Propose {
                command_id: command_id.into(),
                payload: payload.into(),
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => ControlRuntimeError::Backpressure,
                mpsc::error::TrySendError::Closed(_) => ControlRuntimeError::Stopped,
            })?;
        match tokio::time::timeout(self.proposal_timeout, response).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(ControlRuntimeError::Stopped),
            Err(_) => Err(ControlRuntimeError::ProposalTimeout),
        }
    }

    /// Returns the latest control authority and liveness snapshot.
    pub fn status(&self) -> ControlStatus {
        self.status.borrow().clone()
    }

    /// Subscribes to locally applied committed entries.
    pub fn subscribe_applied(&self) -> broadcast::Receiver<AppliedControlEntry> {
        self.applied.subscribe()
    }
}

/// Lifecycle owner for one replicated control task.
pub struct ControlRuntime {
    client: ControlClient,
    shutdown: CancellationToken,
    task: Option<JoinHandle<Result<(), ControlRuntimeError>>>,
}

impl ControlRuntime {
    /// Recovers durable state through Nim and starts election/replication.
    pub async fn start(
        mesh: MeshClient,
        boundary: BoundaryClient,
        store: Arc<dyn ControlLogStorePort>,
        voters: impl IntoIterator<Item = String>,
        options: ControlRuntimeOptions,
    ) -> Result<Self, ControlRuntimeError> {
        options.validate()?;
        let voters = validated_voters(voters)?;
        let remote_voters = u32::try_from(voters.len().saturating_sub(1))
            .map_err(|_| ControlRuntimeError::InvalidConfiguration)?;
        if options
            .heartbeat_interval
            .saturating_mul(remote_voters)
            .saturating_mul(3)
            > options.election_timeout
        {
            return Err(ControlRuntimeError::InvalidConfiguration);
        }
        let local_node_id = node_name(mesh.local_node_id());
        if !voters.contains_key(&local_node_id) {
            return Err(ControlRuntimeError::LocalNotVoter);
        }
        let recovered = store.recover_control_state()?;
        let state = recover(
            &boundary,
            recovered,
            voters.keys().cloned().collect(),
            options,
        )
        .await?;
        if !state.valid {
            return Err(ControlRuntimeError::PolicyRejected(
                ControlStateError::CorruptRecovery,
            ));
        }
        let (commands, command_receiver) = mpsc::channel(options.command_capacity);
        let (applied, _) = broadcast::channel(options.command_capacity);
        let initial_status = status_for(&local_node_id, &state, false, true, None);
        let (status_sender, status) = watch::channel(initial_status);
        let shutdown = CancellationToken::new();
        let context = RuntimeContext {
            mesh,
            boundary,
            store,
            voters,
            local_node_id,
            options,
            state,
            status: status_sender,
            applied: applied.clone(),
            commands: command_receiver,
            election_supporters: BTreeSet::new(),
            live_supporters: BTreeSet::new(),
            peer_seen: HashMap::new(),
            replication_acks: HashMap::new(),
            candidate_term: None,
            pending: None,
            forwarded: HashMap::new(),
            forward_sequence: 0,
            election_deadline: Instant::now(),
            last_authority: None,
            catch_up_through: None,
            quorum_cache: None,
            heartbeat_cursor: 0,
        };
        let task_shutdown = shutdown.clone();
        let task = tokio::spawn(async move { context.run(task_shutdown).await });
        Ok(Self {
            client: ControlClient {
                commands,
                status,
                applied,
                proposal_timeout: options
                    .election_timeout
                    .saturating_mul(3)
                    .saturating_add(options.policy_timeout.saturating_mul(2)),
            },
            shutdown,
            task: Some(task),
        })
    }

    /// Returns a cloneable proposal and health facade.
    pub fn client(&self) -> ControlClient {
        self.client.clone()
    }

    /// Stops the background task after failing any outstanding proposal.
    pub async fn stop(mut self) -> Result<(), ControlRuntimeError> {
        self.shutdown.cancel();
        if let Some(task) = self.task.take() {
            task.await.map_err(|_| ControlRuntimeError::TaskFailed)??;
        }
        Ok(())
    }
}

impl Drop for ControlRuntime {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

struct PendingProposal {
    entry: ControlEntry,
    supporters: BTreeSet<String>,
    reply: Option<oneshot::Sender<Result<ControlEntry, ControlRuntimeError>>>,
}

struct ForwardedProposal {
    command_id: String,
    payload: String,
    reply: oneshot::Sender<Result<ControlEntry, ControlRuntimeError>>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ProposalFailure {
    LeaderRequired,
    QuorumRequired,
    PendingEntry,
    Rejected,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "body", rename_all = "snake_case")]
enum WireFrame {
    VoteRequest {
        term: u64,
        candidate_id: String,
        last_index: u64,
        last_term: u64,
    },
    Vote {
        term: u64,
        candidate_id: String,
        granted: bool,
    },
    Authority {
        term: u64,
        leader_id: String,
        election_supporters: Vec<String>,
        live_supporters: Vec<String>,
        commit_index: u64,
    },
    Alive {
        term: u64,
        leader_id: String,
    },
    Replicate {
        term: u64,
        leader_id: String,
        election_supporters: Vec<String>,
        entry: ControlEntry,
    },
    Replicated {
        term: u64,
        leader_id: String,
        index: u64,
    },
    Commit {
        term: u64,
        leader_id: String,
        election_supporters: Vec<String>,
        supporters: Vec<String>,
        index: u64,
        leader_commit_index: u64,
    },
    CatchUp {
        term: u64,
        leader_id: String,
        next_index: u64,
    },
    Proposal {
        request_id: String,
        command_id: String,
        payload: String,
    },
    ProposalResult {
        request_id: String,
        entry: Option<ControlEntry>,
        error: Option<ProposalFailure>,
    },
}

struct RuntimeContext {
    mesh: MeshClient,
    boundary: BoundaryClient,
    store: Arc<dyn ControlLogStorePort>,
    voters: BTreeMap<String, NodeId>,
    local_node_id: String,
    options: ControlRuntimeOptions,
    state: ControlState,
    status: watch::Sender<ControlStatus>,
    applied: broadcast::Sender<AppliedControlEntry>,
    commands: mpsc::Receiver<RuntimeCommand>,
    election_supporters: BTreeSet<String>,
    live_supporters: BTreeSet<String>,
    peer_seen: HashMap<String, Instant>,
    replication_acks: HashMap<u64, BTreeSet<String>>,
    candidate_term: Option<u64>,
    pending: Option<PendingProposal>,
    forwarded: HashMap<String, ForwardedProposal>,
    forward_sequence: u64,
    election_deadline: Instant,
    last_authority: Option<Instant>,
    catch_up_through: Option<u64>,
    quorum_cache: Option<QuorumCache>,
    heartbeat_cursor: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct QuorumCache {
    phase: ControlVoterPhase,
    old_voters: Vec<String>,
    new_voters: Vec<String>,
    supporters: Vec<String>,
    decision: ControlQuorumDecision,
}

impl RuntimeContext {
    async fn run(mut self, shutdown: CancellationToken) -> Result<(), ControlRuntimeError> {
        self.reset_election_deadline();
        self.apply_committed(true).await?;
        let mut messages = self.mesh.subscribe();
        let mut tick = tokio::time::interval(self.options.heartbeat_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    self.fail_pending(ControlRuntimeError::Stopped);
                    self.fail_forwarded(|| ControlRuntimeError::Stopped);
                    self.publish_status(false, false, None);
                    return Ok(());
                }
                _ = tick.tick() => {
                    if let Err(error) = self.on_tick().await {
                        self.record_error(&error);
                    }
                }
                command = self.commands.recv() => match command {
                    Some(command) => self.handle_command(command).await,
                    None => {
                        self.fail_pending(ControlRuntimeError::Stopped);
                        self.fail_forwarded(|| ControlRuntimeError::Stopped);
                        self.publish_status(false, false, None);
                        return Ok(());
                    }
                },
                message = messages.recv() => match message {
                    Ok(message) => {
                        if let Err(error) = self
                            .handle_message(message.from(), message.payload(), message.received_at())
                            .await
                        {
                            self.record_error(&error);
                        }
                    }
                    Err(MeshRuntimeError::SubscriberLagged { skipped }) => {
                        self.record_error(&ControlRuntimeError::Transport(
                            MeshRuntimeError::SubscriberLagged { skipped },
                        ));
                    }
                    Err(error) => return Err(error.into()),
                }
            }
        }
    }

    async fn on_tick(&mut self) -> Result<(), ControlRuntimeError> {
        let now = Instant::now();
        if self.is_leader() {
            self.live_supporters.clear();
            self.live_supporters.insert(self.local_node_id.clone());
            for (node, seen) in &self.peer_seen {
                if is_fresh(*seen, now, self.options.election_timeout) {
                    self.live_supporters.insert(node.clone());
                }
            }
            let quorum = self
                .check_quorum(self.live_supporters.iter().cloned().collect())
                .await?
                .granted;
            self.publish_status(true, quorum, None);
            self.send_authority_heartbeat().await?;
            if let Some(entry) = self.pending.as_ref().map(|pending| pending.entry.clone()) {
                self.broadcast_replicate(entry).await?;
            }
            return Ok(());
        }
        let authority_live = self
            .last_authority
            .is_some_and(|seen| is_fresh(seen, now, self.options.election_timeout));
        if !authority_live && now >= self.election_deadline {
            self.start_election().await?;
        } else if !authority_live {
            self.fail_forwarded(|| ControlRuntimeError::QuorumRequired);
            self.publish_status(true, false, None);
        }
        Ok(())
    }

    async fn handle_command(&mut self, command: RuntimeCommand) {
        let RuntimeCommand::Propose {
            command_id,
            payload,
            reply,
        } = command;
        let result = self.start_proposal(command_id, payload, reply).await;
        if let Err(error) = result {
            self.record_error(&error);
        }
    }

    async fn start_proposal(
        &mut self,
        command_id: String,
        payload: String,
        reply: oneshot::Sender<Result<ControlEntry, ControlRuntimeError>>,
    ) -> Result<(), ControlRuntimeError> {
        if !self.is_leader() {
            return self.forward_proposal(command_id, payload, reply).await;
        }
        if !self
            .check_quorum(self.live_supporters.iter().cloned().collect())
            .await?
            .granted
        {
            let _ = reply.send(Err(ControlRuntimeError::QuorumRequired));
            return Ok(());
        }
        if self.pending.is_some() || self.state.last_index != self.state.commit_index {
            let _ = reply.send(Err(ControlRuntimeError::PendingEntry));
            return Ok(());
        }
        let plan = self
            .plan(ControlPolicyRequest::Append {
                state: self.state.clone(),
                request: ControlAppendRequest {
                    leader_id: self.local_node_id.clone(),
                    term: self.state.term,
                    kind: ControlEntryKind::Command,
                    command_id,
                    payload,
                    target_voters: Vec::new(),
                },
            })
            .await?;
        let decision = self.settle(plan).await?;
        if decision.effect == ControlEffect::Replay {
            let entry = decision
                .applied_entry
                .ok_or(ControlRuntimeError::UnexpectedPolicyResult)?;
            let _ = reply.send(Ok(entry));
            return Ok(());
        }
        if decision.effect != ControlEffect::Append {
            return Err(ControlRuntimeError::UnexpectedPolicyResult);
        }
        let entry = decision
            .state
            .log
            .last()
            .cloned()
            .ok_or(ControlRuntimeError::UnexpectedPolicyResult)?;
        self.pending = Some(PendingProposal {
            entry: entry.clone(),
            supporters: BTreeSet::from([self.local_node_id.clone()]),
            reply: Some(reply),
        });
        self.replication_acks
            .insert(entry.index, BTreeSet::from([self.local_node_id.clone()]));
        self.broadcast_replicate(entry).await?;
        self.try_commit_pending().await
    }

    async fn forward_proposal(
        &mut self,
        command_id: String,
        payload: String,
        reply: oneshot::Sender<Result<ControlEntry, ControlRuntimeError>>,
    ) -> Result<(), ControlRuntimeError> {
        let authority_live = self
            .last_authority
            .is_some_and(|seen| is_fresh(seen, Instant::now(), self.options.election_timeout));
        if !authority_live
            || !self
                .check_quorum(self.live_supporters.iter().cloned().collect())
                .await?
                .granted
        {
            let _ = reply.send(Err(ControlRuntimeError::QuorumRequired));
            return Ok(());
        }
        if self.forwarded.len() >= self.options.command_capacity {
            let _ = reply.send(Err(ControlRuntimeError::Backpressure));
            return Ok(());
        }
        let Some(leader) = self.voters.get(&self.state.leader_id).copied() else {
            let _ = reply.send(Err(ControlRuntimeError::LeaderRequired));
            return Ok(());
        };
        self.forward_sequence = self
            .forward_sequence
            .checked_add(1)
            .ok_or(ControlRuntimeError::InvalidConfiguration)?;
        let request_id = format!(
            "{}:{}:{}",
            self.local_node_id, self.state.term, self.forward_sequence
        );
        self.forwarded.insert(
            request_id.clone(),
            ForwardedProposal {
                command_id: command_id.clone(),
                payload: payload.clone(),
                reply,
            },
        );
        let frame = WireFrame::Proposal {
            request_id: request_id.clone(),
            command_id,
            payload,
        };
        if let Err(error) = self.send(leader, frame).await {
            self.record_error(&error);
            if let Some(forwarded) = self.forwarded.remove(&request_id) {
                let _ = forwarded.reply.send(Err(error));
            }
        }
        Ok(())
    }

    async fn start_election(&mut self) -> Result<(), ControlRuntimeError> {
        self.fail_pending(ControlRuntimeError::LeaderRequired);
        self.fail_forwarded(|| ControlRuntimeError::LeaderRequired);
        let term = self
            .state
            .term
            .checked_add(1)
            .ok_or(ControlRuntimeError::InvalidConfiguration)?;
        let last_term = last_log_term(&self.state);
        let plan = self
            .plan(ControlPolicyRequest::Vote {
                state: self.state.clone(),
                request: ControlVoteRequest {
                    term,
                    candidate_id: self.local_node_id.clone(),
                    last_index: self.state.last_index,
                    last_term,
                },
            })
            .await?;
        if plan.error != ControlStateError::None {
            self.reset_election_deadline();
            return Ok(());
        }
        self.settle(plan).await?;
        self.candidate_term = Some(term);
        self.election_supporters = BTreeSet::from([self.local_node_id.clone()]);
        self.live_supporters.clear();
        self.reset_election_deadline();
        self.broadcast(WireFrame::VoteRequest {
            term,
            candidate_id: self.local_node_id.clone(),
            last_index: self.state.last_index,
            last_term,
        })
        .await?;
        self.maybe_elect().await
    }

    async fn maybe_elect(&mut self) -> Result<(), ControlRuntimeError> {
        let Some(term) = self.candidate_term else {
            return Ok(());
        };
        let supporters = self.election_supporters.iter().cloned().collect::<Vec<_>>();
        if !self.check_quorum(supporters.clone()).await?.granted {
            return Ok(());
        }
        let plan = self
            .plan(ControlPolicyRequest::Election {
                state: self.state.clone(),
                request: nimino_boundary::ControlElectionRequest {
                    term,
                    candidate_id: self.local_node_id.clone(),
                    supporters: supporters.clone(),
                },
            })
            .await?;
        self.settle(plan).await?;
        self.candidate_term = None;
        self.election_supporters = supporters.into_iter().collect();
        self.live_supporters = BTreeSet::from([self.local_node_id.clone()]);
        self.peer_seen.clear();
        self.last_authority = Some(Instant::now());
        if self.state.last_index > self.state.commit_index {
            let entry = self
                .entry(self.state.commit_index + 1)
                .cloned()
                .ok_or(ControlRuntimeError::UnexpectedPolicyResult)?;
            self.pending = Some(PendingProposal {
                entry: entry.clone(),
                supporters: BTreeSet::from([self.local_node_id.clone()]),
                reply: None,
            });
            self.replication_acks
                .insert(entry.index, BTreeSet::from([self.local_node_id.clone()]));
        }
        let quorum = self
            .check_quorum(self.live_supporters.iter().cloned().collect())
            .await?
            .granted;
        self.publish_status(true, quorum, None);
        self.broadcast_authority().await
    }

    async fn handle_message(
        &mut self,
        authenticated_peer: NodeId,
        payload: &[u8],
        received_at: Instant,
    ) -> Result<(), ControlRuntimeError> {
        let Some(payload) = payload.strip_prefix(WIRE_PREFIX) else {
            return Ok(());
        };
        let peer = node_name(authenticated_peer);
        if !self.voters.contains_key(&peer) {
            return Ok(());
        }
        let frame: WireFrame = serde_json::from_slice(payload)
            .map_err(|error| ControlRuntimeError::InvalidFrame(error.to_string()))?;
        match frame {
            WireFrame::VoteRequest {
                term,
                candidate_id,
                last_index,
                last_term,
            } => {
                if candidate_id != peer {
                    return Err(ControlRuntimeError::InvalidFrame(
                        "candidate does not match authenticated peer".into(),
                    ));
                }
                self.handle_vote_request(
                    authenticated_peer,
                    term,
                    candidate_id,
                    last_index,
                    last_term,
                )
                .await
            }
            WireFrame::Vote {
                term,
                candidate_id,
                granted,
            } => self.handle_vote(peer, term, candidate_id, granted).await,
            WireFrame::Authority {
                term,
                leader_id,
                election_supporters,
                live_supporters,
                commit_index,
            } => {
                self.require_leader_peer(&peer, &leader_id)?;
                self.handle_authority(
                    authenticated_peer,
                    term,
                    leader_id,
                    election_supporters,
                    live_supporters,
                    commit_index,
                    received_at,
                )
                .await
            }
            WireFrame::Alive { term, leader_id } => {
                if leader_id != self.local_node_id || !self.is_leader() || term != self.state.term {
                    return Ok(());
                }
                self.peer_seen.insert(peer, received_at);
                Ok(())
            }
            WireFrame::Replicate {
                term,
                leader_id,
                election_supporters,
                entry,
            } => {
                self.require_leader_peer(&peer, &leader_id)?;
                self.handle_replicate(
                    authenticated_peer,
                    term,
                    leader_id,
                    election_supporters,
                    entry,
                )
                .await
            }
            WireFrame::Replicated {
                term,
                leader_id,
                index,
            } => {
                self.handle_replicated(peer, term, leader_id, index, received_at)
                    .await
            }
            WireFrame::Commit {
                term,
                leader_id,
                election_supporters,
                supporters,
                index,
                leader_commit_index,
            } => {
                self.require_leader_peer(&peer, &leader_id)?;
                self.handle_commit(
                    authenticated_peer,
                    term,
                    leader_id,
                    election_supporters,
                    supporters,
                    index,
                    leader_commit_index,
                )
                .await
            }
            WireFrame::CatchUp {
                term,
                leader_id,
                next_index,
            } => {
                self.handle_catch_up(
                    authenticated_peer,
                    peer,
                    term,
                    leader_id,
                    next_index,
                    received_at,
                )
                .await
            }
            WireFrame::Proposal {
                request_id,
                command_id,
                payload,
            } => {
                self.handle_forwarded_proposal(authenticated_peer, request_id, command_id, payload)
                    .await
            }
            WireFrame::ProposalResult {
                request_id,
                entry,
                error,
            } => self.handle_proposal_result(peer, request_id, entry, error),
        }
    }

    async fn handle_forwarded_proposal(
        &mut self,
        peer: NodeId,
        request_id: String,
        command_id: String,
        payload: String,
    ) -> Result<(), ControlRuntimeError> {
        if !self.is_leader() {
            return self
                .send(
                    peer,
                    WireFrame::ProposalResult {
                        request_id,
                        entry: None,
                        error: Some(ProposalFailure::LeaderRequired),
                    },
                )
                .await;
        }
        let mesh = self.mesh.clone();
        let (reply, response) = oneshot::channel();
        tokio::spawn(async move {
            let (entry, error) = match response.await {
                Ok(Ok(entry)) => (Some(entry), None),
                Ok(Err(error)) => (None, Some(proposal_failure(&error))),
                Err(_) => (None, Some(ProposalFailure::Rejected)),
            };
            if let Ok(payload) = encode_wire(&WireFrame::ProposalResult {
                request_id,
                entry,
                error,
            }) {
                let _ = mesh.send(peer, payload).await;
            }
        });
        self.start_proposal(command_id, payload, reply).await
    }

    fn handle_proposal_result(
        &mut self,
        peer: String,
        request_id: String,
        entry: Option<ControlEntry>,
        error: Option<ProposalFailure>,
    ) -> Result<(), ControlRuntimeError> {
        if peer != self.state.leader_id {
            return Err(ControlRuntimeError::InvalidFrame(
                "proposal result is not from the current leader".into(),
            ));
        }
        let Some(forwarded) = self.forwarded.remove(&request_id) else {
            return Ok(());
        };
        let result = match (entry, error) {
            (Some(entry), None)
                if entry.command_id == forwarded.command_id
                    && entry.payload == forwarded.payload =>
            {
                Ok(entry)
            }
            (None, Some(error)) => Err(proposal_error(error)),
            _ => Err(ControlRuntimeError::InvalidFrame(
                "proposal result does not match the request".into(),
            )),
        };
        let _ = forwarded.reply.send(result);
        Ok(())
    }

    async fn handle_vote_request(
        &mut self,
        peer: NodeId,
        term: u64,
        candidate_id: String,
        last_index: u64,
        last_term: u64,
    ) -> Result<(), ControlRuntimeError> {
        let plan = self
            .plan(ControlPolicyRequest::Vote {
                state: self.state.clone(),
                request: ControlVoteRequest {
                    term,
                    candidate_id: candidate_id.clone(),
                    last_index,
                    last_term,
                },
            })
            .await?;
        let granted = plan.error == ControlStateError::None;
        if granted {
            self.fail_pending(ControlRuntimeError::LeaderRequired);
            self.fail_forwarded(|| ControlRuntimeError::LeaderRequired);
            self.settle(plan).await?;
            self.candidate_term = None;
            self.last_authority = None;
            self.reset_election_deadline();
            self.publish_status(true, false, None);
        }
        self.send(
            peer,
            WireFrame::Vote {
                term,
                candidate_id,
                granted,
            },
        )
        .await
    }

    async fn handle_vote(
        &mut self,
        peer: String,
        term: u64,
        candidate_id: String,
        granted: bool,
    ) -> Result<(), ControlRuntimeError> {
        if granted
            && candidate_id == self.local_node_id
            && self.candidate_term == Some(term)
            && self.state.term == term
        {
            self.election_supporters.insert(peer);
            self.maybe_elect().await?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)] // Exact authenticated frame fields plus receipt time.
    async fn handle_authority(
        &mut self,
        peer: NodeId,
        term: u64,
        leader_id: String,
        election_supporters: Vec<String>,
        live_supporters: Vec<String>,
        commit_index: u64,
        received_at: Instant,
    ) -> Result<(), ControlRuntimeError> {
        self.ensure_leader(term, &leader_id, &election_supporters)
            .await?;
        self.candidate_term = None;
        self.last_authority = Some(received_at);
        self.reset_election_deadline();
        self.election_supporters = election_supporters.into_iter().collect();
        self.live_supporters = live_supporters.into_iter().collect();
        let quorum = is_fresh(received_at, Instant::now(), self.options.election_timeout)
            && self
                .check_quorum(self.live_supporters.iter().cloned().collect())
                .await?
                .granted;
        if !quorum {
            self.fail_forwarded(|| ControlRuntimeError::QuorumRequired);
        }
        self.publish_status(true, quorum, None);
        self.send(
            peer,
            WireFrame::Alive {
                term,
                leader_id: leader_id.clone(),
            },
        )
        .await?;
        if commit_index > self.state.commit_index {
            self.catch_up_through = Some(
                self.catch_up_through
                    .map_or(commit_index, |target| target.max(commit_index)),
            );
            self.request_catch_up(peer, term, leader_id).await?;
        }
        Ok(())
    }

    async fn handle_replicate(
        &mut self,
        peer: NodeId,
        term: u64,
        leader_id: String,
        election_supporters: Vec<String>,
        entry: ControlEntry,
    ) -> Result<(), ControlRuntimeError> {
        self.ensure_leader(term, &leader_id, &election_supporters)
            .await?;
        if let Some(existing) = self.entry(entry.index) {
            if existing == &entry {
                return self
                    .send_replicated(peer, term, leader_id, entry.index)
                    .await;
            }
            if entry.index <= self.state.commit_index {
                return Err(ControlRuntimeError::InvalidFrame(
                    "leader entry conflicts with committed local prefix".into(),
                ));
            }
        }
        if entry.index > self.state.last_index.saturating_add(1) {
            return self.request_catch_up(peer, term, leader_id).await;
        }
        let previous_index = entry
            .index
            .checked_sub(1)
            .ok_or_else(|| ControlRuntimeError::InvalidFrame("control index zero".into()))?;
        let plan = self
            .plan(ControlPolicyRequest::Replicate {
                state: self.state.clone(),
                request: ControlReplicationRequest {
                    leader_id: leader_id.clone(),
                    term,
                    supporters: election_supporters,
                    previous_index,
                    entry: entry.clone(),
                },
            })
            .await?;
        self.settle(plan).await?;
        self.send_replicated(peer, term, leader_id, entry.index)
            .await
    }

    async fn handle_replicated(
        &mut self,
        peer: String,
        term: u64,
        leader_id: String,
        index: u64,
        received_at: Instant,
    ) -> Result<(), ControlRuntimeError> {
        if leader_id != self.local_node_id || !self.is_leader() || term != self.state.term {
            return Ok(());
        }
        self.peer_seen.insert(peer.clone(), received_at);
        let supporters = self.replication_acks.entry(index).or_default();
        supporters.insert(self.local_node_id.clone());
        supporters.insert(peer);
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.entry.index == index)
        {
            if let Some(pending) = &mut self.pending {
                pending.supporters = supporters.clone();
            }
            return self.try_commit_pending().await;
        }
        if index <= self.state.commit_index {
            let supporters = supporters.iter().cloned().collect::<Vec<_>>();
            if self.check_quorum(supporters.clone()).await?.granted {
                self.broadcast(WireFrame::Commit {
                    term,
                    leader_id,
                    election_supporters: self.election_supporters.iter().cloned().collect(),
                    supporters,
                    index,
                    leader_commit_index: self.state.commit_index,
                })
                .await?;
            }
        }
        Ok(())
    }

    async fn try_commit_pending(&mut self) -> Result<(), ControlRuntimeError> {
        let Some((index, supporters)) = self.pending.as_ref().map(|pending| {
            (
                pending.entry.index,
                pending.supporters.iter().cloned().collect::<Vec<_>>(),
            )
        }) else {
            return Ok(());
        };
        let plan = self
            .plan(ControlPolicyRequest::Commit {
                state: self.state.clone(),
                request: ControlCommitRequest {
                    index,
                    leader_id: self.local_node_id.clone(),
                    term: self.state.term,
                    supporters: supporters.clone(),
                },
            })
            .await?;
        if plan.error == ControlStateError::QuorumRequired {
            return Ok(());
        }
        let decision = self.settle(plan).await?;
        if decision.effect != ControlEffect::Commit {
            return Err(ControlRuntimeError::UnexpectedPolicyResult);
        }
        self.apply_committed(false).await?;
        let pending = self
            .pending
            .take()
            .ok_or(ControlRuntimeError::PendingEntry)?;
        if let Some(reply) = pending.reply {
            let _ = reply.send(Ok(pending.entry));
        }
        self.broadcast(WireFrame::Commit {
            term: self.state.term,
            leader_id: self.local_node_id.clone(),
            election_supporters: self.election_supporters.iter().cloned().collect(),
            supporters,
            index,
            leader_commit_index: self.state.commit_index,
        })
        .await?;
        self.broadcast_authority().await
    }

    #[allow(clippy::too_many_arguments)]
    async fn handle_commit(
        &mut self,
        peer: NodeId,
        term: u64,
        leader_id: String,
        election_supporters: Vec<String>,
        supporters: Vec<String>,
        index: u64,
        leader_commit_index: u64,
    ) -> Result<(), ControlRuntimeError> {
        self.ensure_leader(term, &leader_id, &election_supporters)
            .await?;
        if index > self.state.last_index || index > self.state.commit_index.saturating_add(1) {
            return self.request_catch_up(peer, term, leader_id).await;
        }
        if index > self.state.commit_index {
            let plan = self
                .plan(ControlPolicyRequest::Commit {
                    state: self.state.clone(),
                    request: ControlCommitRequest {
                        index,
                        leader_id: leader_id.clone(),
                        term,
                        supporters,
                    },
                })
                .await?;
            self.settle(plan).await?;
            let recovered = self.catch_up_through.is_some_and(|target| index <= target);
            self.apply_committed(recovered).await?;
            if self
                .catch_up_through
                .is_some_and(|target| self.state.commit_index >= target)
            {
                self.catch_up_through = None;
            }
        }
        if leader_commit_index > self.state.commit_index {
            self.request_catch_up(peer, term, leader_id).await?;
        }
        Ok(())
    }

    async fn handle_catch_up(
        &mut self,
        peer_id: NodeId,
        peer: String,
        term: u64,
        leader_id: String,
        next_index: u64,
        received_at: Instant,
    ) -> Result<(), ControlRuntimeError> {
        if leader_id != self.local_node_id || !self.is_leader() || term != self.state.term {
            return Ok(());
        }
        let Some(entry) = self.entry(next_index).cloned() else {
            return Ok(());
        };
        self.replication_acks
            .entry(next_index)
            .or_insert_with(|| BTreeSet::from([self.local_node_id.clone()]));
        self.send(
            peer_id,
            WireFrame::Replicate {
                term,
                leader_id,
                election_supporters: self.election_supporters.iter().cloned().collect(),
                entry,
            },
        )
        .await?;
        self.peer_seen.insert(peer, received_at);
        Ok(())
    }

    async fn ensure_leader(
        &mut self,
        term: u64,
        leader_id: &str,
        supporters: &[String],
    ) -> Result<(), ControlRuntimeError> {
        if self.state.term == term
            && self.state.leader_term == term
            && self.state.leader_id == leader_id
        {
            return Ok(());
        }
        let plan = self
            .plan(ControlPolicyRequest::Election {
                state: self.state.clone(),
                request: nimino_boundary::ControlElectionRequest {
                    term,
                    candidate_id: leader_id.to_owned(),
                    supporters: supporters.to_vec(),
                },
            })
            .await?;
        self.fail_pending(ControlRuntimeError::LeaderRequired);
        self.fail_forwarded(|| ControlRuntimeError::LeaderRequired);
        self.settle(plan).await?;
        Ok(())
    }

    async fn apply_committed(&mut self, recovered: bool) -> Result<(), ControlRuntimeError> {
        while self.state.applied_index < self.state.commit_index {
            let plan = self
                .plan(ControlPolicyRequest::Apply {
                    state: self.state.clone(),
                })
                .await?;
            let decision = self.settle(plan).await?;
            let entry = decision
                .applied_entry
                .ok_or(ControlRuntimeError::UnexpectedPolicyResult)?;
            let _ = self.applied.send(AppliedControlEntry { entry, recovered });
        }
        Ok(())
    }

    async fn plan(
        &self,
        request: ControlPolicyRequest,
    ) -> Result<ControlPlan, ControlRuntimeError> {
        let result = self.policy(request).await?;
        let ControlPolicyResult::Plan { plan } = result else {
            return Err(ControlRuntimeError::UnexpectedPolicyResult);
        };
        Ok(plan)
    }

    async fn check_quorum(
        &mut self,
        supporters: Vec<String>,
    ) -> Result<ControlQuorumDecision, ControlRuntimeError> {
        if let Some(cached) = &self.quorum_cache {
            if cached.phase == self.state.phase
                && cached.old_voters == self.state.old_voters
                && cached.new_voters == self.state.new_voters
                && cached.supporters == supporters
            {
                return Ok(cached.decision.clone());
            }
        }
        let result = self
            .policy(ControlPolicyRequest::Quorum {
                state: self.state.clone(),
                request: ControlQuorumRequest {
                    supporters: supporters.clone(),
                },
            })
            .await?;
        let ControlPolicyResult::Quorum { result } = result else {
            return Err(ControlRuntimeError::UnexpectedPolicyResult);
        };
        self.quorum_cache = Some(QuorumCache {
            phase: self.state.phase,
            old_voters: self.state.old_voters.clone(),
            new_voters: self.state.new_voters.clone(),
            supporters,
            decision: result.clone(),
        });
        Ok(result)
    }

    async fn settle(&mut self, plan: ControlPlan) -> Result<ControlDecision, ControlRuntimeError> {
        if plan.error != ControlStateError::None {
            let result = self
                .policy(ControlPolicyRequest::Settle {
                    plan,
                    store_succeeded: false,
                })
                .await?;
            let ControlPolicyResult::Settle { result } = result else {
                return Err(ControlRuntimeError::UnexpectedPolicyResult);
            };
            return Err(ControlRuntimeError::PolicyRejected(result.error));
        }
        let store_result = execute_store_plan(self.store.as_ref(), &plan);
        let result = self
            .policy(ControlPolicyRequest::Settle {
                plan,
                store_succeeded: store_result.is_ok(),
            })
            .await?;
        let ControlPolicyResult::Settle { result } = result else {
            return Err(ControlRuntimeError::UnexpectedPolicyResult);
        };
        if let Err(error) = store_result {
            return Err(error.into());
        }
        if result.error != ControlStateError::None {
            return Err(ControlRuntimeError::PolicyRejected(result.error));
        }
        self.state = result.state.clone();
        Ok(result)
    }

    async fn policy(
        &self,
        request: ControlPolicyRequest,
    ) -> Result<ControlPolicyResult, ControlRuntimeError> {
        let result = self
            .boundary
            .call(
                BoundaryRequest::control_policy(request),
                CallContext::with_timeout(self.options.policy_timeout),
            )
            .await?;
        let BoundaryResult::ControlPolicy(result) = result else {
            return Err(ControlRuntimeError::UnexpectedPolicyResult);
        };
        Ok(result)
    }

    async fn broadcast_authority(&self) -> Result<(), ControlRuntimeError> {
        self.broadcast(self.authority_frame()).await
    }

    async fn send_authority_heartbeat(&mut self) -> Result<(), ControlRuntimeError> {
        let remote_count = self.voters.len().saturating_sub(1);
        let Some(peer) = self
            .voters
            .iter()
            .filter_map(|(voter, peer)| (voter != &self.local_node_id).then_some(*peer))
            .nth(self.heartbeat_cursor % remote_count.max(1))
        else {
            return Ok(());
        };
        self.heartbeat_cursor = self.heartbeat_cursor.wrapping_add(1);
        self.send(peer, self.authority_frame()).await
    }

    fn authority_frame(&self) -> WireFrame {
        WireFrame::Authority {
            term: self.state.term,
            leader_id: self.local_node_id.clone(),
            election_supporters: self.election_supporters.iter().cloned().collect(),
            live_supporters: self.live_supporters.iter().cloned().collect(),
            commit_index: self.state.commit_index,
        }
    }

    async fn broadcast_replicate(&self, entry: ControlEntry) -> Result<(), ControlRuntimeError> {
        self.broadcast(WireFrame::Replicate {
            term: self.state.term,
            leader_id: self.local_node_id.clone(),
            election_supporters: self.election_supporters.iter().cloned().collect(),
            entry,
        })
        .await
    }

    async fn request_catch_up(
        &self,
        peer: NodeId,
        term: u64,
        leader_id: String,
    ) -> Result<(), ControlRuntimeError> {
        self.send(
            peer,
            WireFrame::CatchUp {
                term,
                leader_id,
                next_index: self.state.commit_index.saturating_add(1),
            },
        )
        .await
    }

    async fn send_replicated(
        &self,
        peer: NodeId,
        term: u64,
        leader_id: String,
        index: u64,
    ) -> Result<(), ControlRuntimeError> {
        self.send(
            peer,
            WireFrame::Replicated {
                term,
                leader_id,
                index,
            },
        )
        .await
    }

    async fn send(&self, peer: NodeId, frame: WireFrame) -> Result<(), ControlRuntimeError> {
        self.mesh.send(peer, encode_wire(&frame)?).await?;
        Ok(())
    }

    async fn broadcast(&self, frame: WireFrame) -> Result<(), ControlRuntimeError> {
        self.mesh.broadcast(encode_wire(&frame)?).await?;
        Ok(())
    }

    fn entry(&self, index: u64) -> Option<&ControlEntry> {
        self.state.log.iter().find(|entry| entry.index == index)
    }

    fn is_leader(&self) -> bool {
        self.state.term > 0
            && self.state.leader_term == self.state.term
            && self.state.leader_id == self.local_node_id
    }

    fn require_leader_peer(&self, peer: &str, leader: &str) -> Result<(), ControlRuntimeError> {
        if peer == leader {
            Ok(())
        } else {
            Err(ControlRuntimeError::InvalidFrame(
                "leader does not match authenticated peer".into(),
            ))
        }
    }

    fn reset_election_deadline(&mut self) {
        let bytes = self.mesh.local_node_id().as_bytes();
        let entropy = u64::from_be_bytes(bytes[..8].try_into().unwrap_or([0; 8])) ^ self.state.term;
        let jitter_bound = self.options.election_timeout.as_millis().max(1) as u64;
        self.election_deadline = Instant::now()
            + self.options.election_timeout
            + Duration::from_millis(entropy % jitter_bound);
    }

    fn fail_pending(&mut self, error: ControlRuntimeError) {
        if let Some(mut pending) = self.pending.take() {
            if let Some(reply) = pending.reply.take() {
                let _ = reply.send(Err(error));
            }
        }
    }

    fn fail_forwarded(&mut self, mut error: impl FnMut() -> ControlRuntimeError) {
        for (_, forwarded) in self.forwarded.drain() {
            let _ = forwarded.reply.send(Err(error()));
        }
    }

    fn record_error(&self, error: &ControlRuntimeError) {
        self.publish_status(true, false, Some(error.to_string()));
    }

    fn publish_status(&self, running: bool, quorum: bool, error: Option<String>) {
        self.status.send_replace(status_for(
            &self.local_node_id,
            &self.state,
            quorum,
            running,
            error,
        ));
    }
}

async fn recover(
    boundary: &BoundaryClient,
    recovered: RecoveredControlState,
    initial_voters: Vec<String>,
    options: ControlRuntimeOptions,
) -> Result<ControlState, ControlRuntimeError> {
    let request = ControlPolicyRequest::Recover {
        input: recovery_input(recovered, initial_voters)?,
    };
    let result = boundary
        .call(
            BoundaryRequest::control_policy(request),
            CallContext::with_timeout(options.policy_timeout),
        )
        .await?;
    let BoundaryResult::ControlPolicy(ControlPolicyResult::Recover { result }) = result else {
        return Err(ControlRuntimeError::UnexpectedPolicyResult);
    };
    if result.error != ControlStateError::None {
        return Err(ControlRuntimeError::PolicyRejected(result.error));
    }
    Ok(result.state)
}

fn execute_store_plan(
    store: &dyn ControlLogStorePort,
    plan: &ControlPlan,
) -> Result<(), StoreError> {
    for action in &plan.actions {
        match action.kind {
            ControlStoreActionKind::Metadata => {
                store.compare_and_set_control_metadata(
                    action.expected_metadata_revision,
                    ControlMetadata {
                        term: plan.next_state.term,
                        voted_for: plan.next_state.voted_for.clone(),
                        commit_index: plan.next_state.commit_index,
                        applied_index: plan.next_state.applied_index,
                    },
                )?;
            }
            ControlStoreActionKind::Log => {
                let entries = plan
                    .next_state
                    .log
                    .iter()
                    .filter(|entry| entry.index > action.previous_index)
                    .map(store_entry)
                    .collect::<Vec<_>>();
                store.replace_control_suffix(action.previous_index, entries)?;
            }
            ControlStoreActionKind::Snapshot => {
                let snapshot = plan
                    .next_state
                    .snapshot
                    .as_ref()
                    .ok_or(StoreError::InvalidInput("control snapshot plan is empty"))?;
                store.install_control_snapshot(
                    action.expected_metadata_revision,
                    store_snapshot(snapshot),
                )?;
            }
        }
    }
    Ok(())
}

fn recovery_input(
    recovered: RecoveredControlState,
    initial_voters: Vec<String>,
) -> Result<ControlRecoveryInput, ControlRuntimeError> {
    Ok(ControlRecoveryInput {
        metadata_revision: recovered.metadata.revision,
        term: recovered.metadata.state.term,
        voted_for: recovered.metadata.state.voted_for,
        commit_index: recovered.metadata.state.commit_index,
        applied_index: recovered.metadata.state.applied_index,
        initial_voters,
        snapshot: recovered.snapshot.map(boundary_snapshot).transpose()?,
        entries: recovered
            .entries
            .into_iter()
            .map(boundary_entry)
            .collect::<Result<Vec<_>, _>>()?,
    })
}

fn store_entry(entry: &ControlEntry) -> ControlLogEntry {
    ControlLogEntry {
        index: entry.index,
        term: entry.term,
        voter_epoch: entry.voter_epoch,
        kind: entry_kind_name(entry.kind).to_owned(),
        command_id: entry.command_id.clone(),
        payload: entry.payload.as_bytes().to_vec(),
        target_voters: entry.target_voters.clone(),
    }
}

fn boundary_entry(entry: ControlLogEntry) -> Result<ControlEntry, ControlRuntimeError> {
    Ok(ControlEntry {
        index: entry.index,
        term: entry.term,
        voter_epoch: entry.voter_epoch,
        kind: parse_entry_kind(&entry.kind)?,
        command_id: entry.command_id,
        payload: String::from_utf8(entry.payload)
            .map_err(|_| ControlRuntimeError::InvalidStoredText)?,
        target_voters: entry.target_voters,
    })
}

fn store_snapshot(snapshot: &ControlSnapshotState) -> ControlSnapshot {
    ControlSnapshot {
        last_included_index: snapshot.last_included_index,
        last_included_term: snapshot.last_included_term,
        voter_epoch: snapshot.voter_epoch,
        voter_phase: phase_name(snapshot.phase).to_owned(),
        old_voters: snapshot.old_voters.clone(),
        new_voters: snapshot.new_voters.clone(),
        state: snapshot.state_payload.as_bytes().to_vec(),
    }
}

fn boundary_snapshot(
    snapshot: ControlSnapshot,
) -> Result<ControlSnapshotState, ControlRuntimeError> {
    Ok(ControlSnapshotState {
        last_included_index: snapshot.last_included_index,
        last_included_term: snapshot.last_included_term,
        voter_epoch: snapshot.voter_epoch,
        phase: parse_phase(&snapshot.voter_phase)?,
        old_voters: snapshot.old_voters,
        new_voters: snapshot.new_voters,
        state_payload: String::from_utf8(snapshot.state)
            .map_err(|_| ControlRuntimeError::InvalidStoredText)?,
    })
}

fn validated_voters(
    voters: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, NodeId>, ControlRuntimeError> {
    let mut result = BTreeMap::new();
    for voter in voters {
        let bytes =
            hex::decode(&voter).map_err(|_| ControlRuntimeError::InvalidVoter(voter.clone()))?;
        let bytes: [u8; 16] = bytes
            .try_into()
            .map_err(|_| ControlRuntimeError::InvalidVoter(voter.clone()))?;
        let canonical = hex::encode(bytes);
        if canonical != voter
            || result
                .insert(voter.clone(), NodeId::from_bytes(bytes))
                .is_some()
        {
            return Err(ControlRuntimeError::InvalidVoter(voter));
        }
    }
    if result.is_empty() {
        return Err(ControlRuntimeError::InvalidConfiguration);
    }
    Ok(result)
}

fn encode_wire(frame: &WireFrame) -> Result<Vec<u8>, ControlRuntimeError> {
    let encoded = serde_json::to_vec(frame)
        .map_err(|error| ControlRuntimeError::InvalidFrame(error.to_string()))?;
    let mut payload = Vec::with_capacity(WIRE_PREFIX.len() + encoded.len());
    payload.extend_from_slice(WIRE_PREFIX);
    payload.extend_from_slice(&encoded);
    Ok(payload)
}

fn node_name(node: NodeId) -> String {
    hex::encode(node.as_bytes())
}

fn last_log_term(state: &ControlState) -> u64 {
    state
        .log
        .last()
        .map(|entry| entry.term)
        .or_else(|| {
            state
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.last_included_term)
        })
        .unwrap_or(0)
}

fn is_fresh(seen: Instant, now: Instant, timeout: Duration) -> bool {
    now.saturating_duration_since(seen) <= timeout
}

fn proposal_failure(error: &ControlRuntimeError) -> ProposalFailure {
    match error {
        ControlRuntimeError::LeaderRequired => ProposalFailure::LeaderRequired,
        ControlRuntimeError::QuorumRequired => ProposalFailure::QuorumRequired,
        ControlRuntimeError::PendingEntry => ProposalFailure::PendingEntry,
        _ => ProposalFailure::Rejected,
    }
}

fn proposal_error(error: ProposalFailure) -> ControlRuntimeError {
    match error {
        ProposalFailure::LeaderRequired => ControlRuntimeError::LeaderRequired,
        ProposalFailure::QuorumRequired => ControlRuntimeError::QuorumRequired,
        ProposalFailure::PendingEntry => ControlRuntimeError::PendingEntry,
        ProposalFailure::Rejected => ControlRuntimeError::RemoteProposalRejected,
    }
}

fn entry_kind_name(kind: ControlEntryKind) -> &'static str {
    match kind {
        ControlEntryKind::Command => "command",
        ControlEntryKind::BeginJoint => "begin_joint",
        ControlEntryKind::Finalize => "finalize",
    }
}

fn parse_entry_kind(kind: &str) -> Result<ControlEntryKind, ControlRuntimeError> {
    match kind {
        "command" => Ok(ControlEntryKind::Command),
        "begin_joint" => Ok(ControlEntryKind::BeginJoint),
        "finalize" => Ok(ControlEntryKind::Finalize),
        _ => Err(ControlRuntimeError::InvalidFrame(
            "stored control entry kind is invalid".into(),
        )),
    }
}

fn phase_name(phase: ControlVoterPhase) -> &'static str {
    match phase {
        ControlVoterPhase::StableOld => "stable_old",
        ControlVoterPhase::Joint => "joint",
        ControlVoterPhase::StableNew => "stable_new",
    }
}

fn parse_phase(phase: &str) -> Result<ControlVoterPhase, ControlRuntimeError> {
    match phase {
        "stable_old" => Ok(ControlVoterPhase::StableOld),
        "joint" => Ok(ControlVoterPhase::Joint),
        "stable_new" => Ok(ControlVoterPhase::StableNew),
        _ => Err(ControlRuntimeError::InvalidFrame(
            "stored control voter phase is invalid".into(),
        )),
    }
}

fn status_for(
    local_node_id: &str,
    state: &ControlState,
    quorum_available: bool,
    running: bool,
    last_error: Option<String>,
) -> ControlStatus {
    ControlStatus {
        running,
        local_node_id: local_node_id.to_owned(),
        term: state.term,
        voter_epoch: state.voter_epoch,
        leader_id: (!state.leader_id.is_empty()).then(|| state.leader_id.clone()),
        quorum_available,
        commit_index: state.commit_index,
        applied_index: state.applied_index,
        last_error,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_noncanonical_voters_and_unsafe_timing() {
        assert!(validated_voters(["00".repeat(16)]).is_ok());
        assert!(validated_voters(["AA".repeat(16)]).is_err());
        assert!(validated_voters(["00".repeat(15)]).is_err());
        let defaults = ControlRuntimeOptions::default();
        assert!(
            defaults
                .heartbeat_interval
                .saturating_mul(4)
                .saturating_mul(3)
                <= defaults.election_timeout
        );
        assert!(ControlRuntimeOptions::new(
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(1),
            1,
        )
        .validate()
        .is_err());
    }

    #[test]
    fn wire_prefix_keeps_control_frames_out_of_other_chirps_consumers() {
        let encoded = encode_wire(&WireFrame::Alive {
            term: 1,
            leader_id: "00".repeat(16),
        })
        .unwrap();
        assert!(encoded.starts_with(WIRE_PREFIX));
        assert!(serde_json::from_slice::<WireFrame>(&encoded[WIRE_PREFIX.len()..]).is_ok());
    }

    #[test]
    fn liveness_uses_receive_time_not_dequeue_time() {
        let received = Instant::now();
        let timeout = Duration::from_secs(1);

        assert!(is_fresh(received, received + timeout, timeout));
        assert!(!is_fresh(
            received,
            received + timeout + Duration::from_millis(1),
            timeout
        ));
    }
}
