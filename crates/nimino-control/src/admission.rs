//! Quorum-backed request admission projected from the Nimino control log.

use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use nimino_boundary::{
    AdmissionPolicyError, AdmissionPolicyRequest, AdmissionPolicyResult,
    AuthorizationInvalidationCommand, AuthorizationInvalidationEffect,
    AuthorizationInvalidationError, AuthorizationInvalidationKind, AuthorizationInvalidationState,
    BoundaryClient, BoundaryError, BoundaryRequest, BoundaryResult, CallContext, ControlEntry,
    RateLimitCommand, RateLimitState, ReplayClaimCommand, ReplayClaimState,
};
use nimino_store::{ControlLogStorePort, StoreError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    sync::{broadcast, mpsc, oneshot, watch, Mutex},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{ControlClient, ControlRuntimeError};

const ADMISSION_COMMAND_VERSION: u8 = 1;
const ADMISSION_COMMAND_DOMAIN: &str = "nimino.admission.command";
const PRUNE_BATCH: usize = 256;
const RATE_BATCH_SIZE: usize = 64;
const RATE_BATCH_CAPACITY: usize = 4_096;
const MAX_RATE_BATCH_OUTCOMES: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum AdmissionCommand {
    ReplayClaim {
        scope: String,
        event_id: String,
        observed_at_ms: u64,
        ttl_secs: u64,
    },
    RateLimit {
        namespace: String,
        key: String,
        observed_at_ms: u64,
        window_secs: u64,
        limit: u64,
    },
    RateLimitBatch {
        attempts: Vec<RateLimitAttempt>,
    },
    AuthorizationInvalidation {
        scope: String,
        kind: AuthorizationInvalidationKind,
        subject: String,
        channel_id: String,
        fact_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RateLimitAttempt {
    namespace: String,
    key: String,
    observed_at_ms: u64,
    window_secs: u64,
    limit: u64,
}

struct RateRequest {
    attempt: RateLimitAttempt,
    reply: oneshot::Sender<Result<RateAdmissionDecision, AdmissionRuntimeError>>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionEnvelope {
    domain: String,
    version: u8,
    command: AdmissionCommand,
}

#[derive(Default)]
struct AdmissionProjection {
    control_index: u64,
    replay: BTreeMap<(String, String), ReplayClaimState>,
    rate: BTreeMap<(String, String), RateLimitState>,
    rate_batch_outcomes: BTreeMap<u64, Vec<RateAdmissionDecision>>,
    invalidations: BTreeMap<
        (AuthorizationInvalidationKind, String, String, String),
        AuthorizationInvalidationState,
    >,
}

/// One cluster-wide fixed-window consume result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateAdmissionDecision {
    /// Whether the attempt remains inside the committed budget.
    pub allowed: bool,
    /// Counter value after this attempt.
    pub current: u64,
    /// Limit fixed for this window.
    pub limit: u64,
    /// Whole seconds until reset.
    pub reset_in_secs: u64,
}

/// Observable cluster admission projection health.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionRuntimeStatus {
    /// Whether the projector accepts admission decisions.
    pub running: bool,
    /// Highest control index observed by this projector.
    pub control_index: u64,
    /// Most recent terminal projector failure.
    pub last_error: Option<String>,
}

/// Cluster admission projection or proposal failure.
#[derive(Debug, Error)]
pub enum AdmissionRuntimeError {
    /// The supervised Nim worker failed.
    #[error("Nim admission policy failed: {0}")]
    Boundary(#[from] BoundaryError),
    /// Replicated control rejected an admission command.
    #[error("admission control proposal failed: {0}")]
    Control(#[from] ControlRuntimeError),
    /// The durable control log failed.
    #[error("admission control log failed: {0}")]
    Store(#[from] StoreError),
    /// Nim rejected the supplied admission facts.
    #[error("Nim rejected admission facts: {0:?}")]
    PolicyRejected(AdmissionPolicyError),
    /// Nim rejected committed authorization invalidation facts.
    #[error("Nim rejected authorization invalidation facts: {0:?}")]
    AuthorizationPolicyRejected(AuthorizationInvalidationError),
    /// Nim returned a response for another admission decision.
    #[error("Nim returned an unexpected admission policy result")]
    UnexpectedPolicyResult,
    /// A committed admission payload violated its versioned envelope.
    #[error("invalid committed admission command: {0}")]
    InvalidCommand(String),
    /// A control snapshot exists but lacks an admission projection.
    #[error("admission recovery does not support compacted control logs")]
    CompactedControlLog,
    /// System time cannot be represented by the v1 policy.
    #[error("system time is unavailable for admission")]
    InvalidClock,
    /// The projector is stopped or failed closed.
    #[error("admission runtime stopped")]
    Stopped,
    /// The projector is failed closed with its last observed cause.
    #[error("admission runtime unavailable: {0}")]
    Unavailable(String),
    /// A committed command did not reach this process-local projection in time.
    #[error("admission projection did not reach committed control index {expected} (at {actual})")]
    ProjectionTimeout {
        /// Committed index returned by replicated control.
        expected: u64,
        /// Highest index visible in the local projection.
        actual: u64,
    },
    /// One coalesced rate proposal failed before every waiter received a decision.
    #[error("rate admission batch failed: {0}")]
    RateBatch(String),
    /// The projector task panicked.
    #[error("admission runtime task failed")]
    TaskFailed,
}

/// Cloneable facade for quorum-backed replay claims.
#[derive(Clone)]
pub struct AdmissionClient {
    control: ControlClient,
    projection: Arc<Mutex<AdmissionProjection>>,
    rate_requests: mpsc::Sender<RateRequest>,
    command_epoch: String,
    command_sequence: Arc<AtomicU64>,
    status: watch::Receiver<AdmissionRuntimeStatus>,
    invalidations: broadcast::Sender<AuthorizationInvalidationState>,
}

impl AdmissionClient {
    /// Atomically claims one scoped NIP-98 event id using the current wall clock.
    pub async fn claim_replay(
        &self,
        scope: impl Into<String>,
        event_id: impl Into<String>,
        ttl_secs: u64,
    ) -> Result<bool, AdmissionRuntimeError> {
        let observed_at_ms = unix_ms()?;
        self.claim_replay_at(scope, event_id, ttl_secs, observed_at_ms)
            .await
    }

    /// Atomically claims one scoped NIP-98 event id at an explicit testable time.
    pub async fn claim_replay_at(
        &self,
        scope: impl Into<String>,
        event_id: impl Into<String>,
        ttl_secs: u64,
        observed_at_ms: u64,
    ) -> Result<bool, AdmissionRuntimeError> {
        self.require_running()?;
        let scope = scope.into();
        let event_id = event_id.into();
        let command = AdmissionCommand::ReplayClaim {
            scope: scope.clone(),
            event_id: event_id.clone(),
            observed_at_ms,
            ttl_secs,
        };
        let entry = self.propose("replay", command).await?;
        self.project_through(entry.index).await?;
        Ok(self
            .projection
            .lock()
            .await
            .replay
            .get(&(scope, event_id))
            .is_some_and(|state| state.last_control_index == entry.index))
    }

    /// Consumes one cluster-wide fixed-window budget using the current wall clock.
    pub async fn consume_rate(
        &self,
        namespace: impl Into<String>,
        key: impl Into<String>,
        window_secs: u64,
        limit: u64,
    ) -> Result<RateAdmissionDecision, AdmissionRuntimeError> {
        self.consume_rate_at(namespace, key, window_secs, limit, unix_ms()?)
            .await
    }

    /// Consumes one cluster-wide fixed-window budget at an explicit testable time.
    pub async fn consume_rate_at(
        &self,
        namespace: impl Into<String>,
        key: impl Into<String>,
        window_secs: u64,
        limit: u64,
        observed_at_ms: u64,
    ) -> Result<RateAdmissionDecision, AdmissionRuntimeError> {
        self.require_running()?;
        let (reply, response) = oneshot::channel();
        self.rate_requests
            .try_send(RateRequest {
                attempt: RateLimitAttempt {
                    namespace: namespace.into(),
                    key: key.into(),
                    observed_at_ms,
                    window_secs,
                    limit,
                },
                reply,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => {
                    AdmissionRuntimeError::Control(ControlRuntimeError::Backpressure)
                }
                mpsc::error::TrySendError::Closed(_) => AdmissionRuntimeError::Stopped,
            })?;
        match tokio::time::timeout(self.control.proposal_timeout, response).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AdmissionRuntimeError::Stopped),
            Err(_) => Err(AdmissionRuntimeError::Control(
                ControlRuntimeError::ProposalTimeout,
            )),
        }
    }

    /// Commits one authorization revalidation signal and returns its monotonic revision.
    pub async fn publish_invalidation(
        &self,
        scope: impl Into<String>,
        kind: AuthorizationInvalidationKind,
        subject: impl Into<String>,
        channel_id: impl Into<String>,
        fact_id: impl Into<String>,
    ) -> Result<u64, AdmissionRuntimeError> {
        self.require_running()?;
        let entry = self
            .propose(
                "auth",
                AdmissionCommand::AuthorizationInvalidation {
                    scope: scope.into(),
                    kind,
                    subject: subject.into(),
                    channel_id: channel_id.into(),
                    fact_id: fact_id.into(),
                },
            )
            .await?;
        self.project_through(entry.index).await?;
        Ok(entry.index)
    }

    /// Subscribes to newly applied remote/local invalidation revisions.
    pub fn subscribe_invalidations(&self) -> broadcast::Receiver<AuthorizationInvalidationState> {
        self.invalidations.subscribe()
    }

    async fn propose(
        &self,
        kind: &str,
        command: AdmissionCommand,
    ) -> Result<ControlEntry, AdmissionRuntimeError> {
        let payload = serde_json::to_string(&AdmissionEnvelope {
            domain: ADMISSION_COMMAND_DOMAIN.to_owned(),
            version: ADMISSION_COMMAND_VERSION,
            command,
        })
        .map_err(|error| AdmissionRuntimeError::InvalidCommand(error.to_string()))?;
        let sequence = self
            .command_sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(|_| {
                AdmissionRuntimeError::InvalidCommand("command sequence exhausted".to_owned())
            })?
            + 1;
        let command_id = format!("admission:{kind}:{}:{sequence}", self.command_epoch);
        let deadline = tokio::time::Instant::now() + self.control.proposal_timeout;
        loop {
            match self
                .control
                .propose(command_id.clone(), payload.clone())
                .await
            {
                Err(
                    ControlRuntimeError::PendingEntry
                    | ControlRuntimeError::LeaderRequired
                    | ControlRuntimeError::QuorumRequired,
                ) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
                result => break Ok(result?),
            }
        }
    }

    async fn project_through(&self, expected: u64) -> Result<(), AdmissionRuntimeError> {
        let deadline = tokio::time::Instant::now() + self.control.proposal_timeout;
        let mut status = self.status.clone();
        loop {
            let actual = status.borrow().control_index;
            if actual >= expected {
                return Ok(());
            }
            match tokio::time::timeout_at(deadline, status.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return Err(AdmissionRuntimeError::Stopped),
                Err(_) => {
                    return Err(AdmissionRuntimeError::ProjectionTimeout {
                        expected,
                        actual: status.borrow().control_index,
                    });
                }
            }
        }
    }

    async fn consume_rate_batch(
        &self,
        attempts: Vec<RateLimitAttempt>,
    ) -> Result<Vec<RateAdmissionDecision>, AdmissionRuntimeError> {
        let entry = self
            .propose("rate", AdmissionCommand::RateLimitBatch { attempts })
            .await?;
        self.project_through(entry.index).await?;
        self.projection
            .lock()
            .await
            .rate_batch_outcomes
            .remove(&entry.index)
            .ok_or(AdmissionRuntimeError::UnexpectedPolicyResult)
    }

    /// Returns projection health without I/O.
    pub fn status(&self) -> AdmissionRuntimeStatus {
        self.status.borrow().clone()
    }

    fn require_running(&self) -> Result<(), AdmissionRuntimeError> {
        let status = self.status.borrow();
        if status.running {
            Ok(())
        } else {
            Err(AdmissionRuntimeError::Unavailable(
                status
                    .last_error
                    .clone()
                    .unwrap_or_else(|| AdmissionRuntimeError::Stopped.to_string()),
            ))
        }
    }
}

/// Lifecycle owner for one process-local view of committed admission state.
pub struct AdmissionRuntime {
    client: AdmissionClient,
    shutdown: CancellationToken,
    task: Option<JoinHandle<Result<(), AdmissionRuntimeError>>>,
    rate_task: Option<JoinHandle<()>>,
}

impl AdmissionRuntime {
    /// Recovers admission state, then follows live committed control entries.
    pub async fn start(
        control: ControlClient,
        boundary: BoundaryClient,
        store: Arc<dyn ControlLogStorePort>,
        policy_timeout: Duration,
    ) -> Result<Self, AdmissionRuntimeError> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| AdmissionRuntimeError::InvalidClock)?
            .as_nanos();
        let command_epoch = format!(
            "{}:{}:{nonce}",
            control.status().local_node_id,
            std::process::id()
        );
        let projection = Arc::new(Mutex::new(AdmissionProjection::default()));
        let mut applied = control.subscribe_applied();
        let (invalidations, _) = broadcast::channel(256);
        let (rate_requests, rate_receiver) = mpsc::channel(RATE_BATCH_CAPACITY);
        replay_committed(&boundary, store.as_ref(), &projection, policy_timeout, None).await?;
        let control_index = projection.lock().await.control_index;
        let (status_sender, status) = watch::channel(AdmissionRuntimeStatus {
            running: true,
            control_index,
            last_error: None,
        });
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();
        let task_boundary = boundary.clone();
        let task_store = store.clone();
        let task_projection = projection.clone();
        let task_invalidations = invalidations.clone();
        let task = tokio::spawn(async move {
            let mut retry = tokio::time::interval(policy_timeout);
            retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            retry.tick().await;
            let mut retry_needed = false;
            loop {
                tokio::select! {
                    _ = task_shutdown.cancelled() => {
                        status_sender.send_replace(AdmissionRuntimeStatus {
                            running: false,
                            control_index: task_projection.lock().await.control_index,
                            last_error: None,
                        });
                        return Ok(());
                    }
                    _ = retry.tick(), if retry_needed => {}
                    result = applied.recv() => match result {
                        Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => {
                            status_sender.send_replace(AdmissionRuntimeStatus {
                                running: false,
                                control_index: task_projection.lock().await.control_index,
                                last_error: Some(AdmissionRuntimeError::Stopped.to_string()),
                            });
                            return Err(AdmissionRuntimeError::Stopped);
                        }
                    }
                }
                match replay_committed(
                    &task_boundary,
                    task_store.as_ref(),
                    &task_projection,
                    policy_timeout,
                    Some(&task_invalidations),
                )
                .await
                {
                    Ok(()) => {
                        retry_needed = false;
                        status_sender.send_replace(AdmissionRuntimeStatus {
                            running: true,
                            control_index: task_projection.lock().await.control_index,
                            last_error: None,
                        });
                    }
                    Err(error) => {
                        retry_needed = true;
                        status_sender.send_replace(AdmissionRuntimeStatus {
                            running: false,
                            control_index: task_projection.lock().await.control_index,
                            last_error: Some(error.to_string()),
                        });
                    }
                }
            }
        });
        let client = AdmissionClient {
            control,
            projection,
            rate_requests,
            command_epoch,
            command_sequence: Arc::new(AtomicU64::new(0)),
            status,
            invalidations,
        };
        let rate_task = tokio::spawn(run_rate_batches(
            client.clone(),
            rate_receiver,
            shutdown.clone(),
        ));
        Ok(Self {
            client,
            shutdown,
            task: Some(task),
            rate_task: Some(rate_task),
        })
    }

    /// Returns a cloneable admission facade.
    pub fn client(&self) -> AdmissionClient {
        self.client.clone()
    }

    /// Stops and joins the admission projector.
    pub async fn stop(mut self) -> Result<(), AdmissionRuntimeError> {
        self.shutdown.cancel();
        if let Some(task) = self.rate_task.take() {
            task.await.map_err(|_| AdmissionRuntimeError::TaskFailed)?;
        }
        if let Some(task) = self.task.take() {
            task.await
                .map_err(|_| AdmissionRuntimeError::TaskFailed)??;
        }
        Ok(())
    }
}

async fn run_rate_batches(
    client: AdmissionClient,
    mut requests: mpsc::Receiver<RateRequest>,
    shutdown: CancellationToken,
) {
    loop {
        let first = tokio::select! {
            _ = shutdown.cancelled() => return,
            request = requests.recv() => match request {
                Some(request) => request,
                None => return,
            },
        };
        let mut batch = vec![first];
        tokio::task::yield_now().await;
        while batch.len() < RATE_BATCH_SIZE {
            match requests.try_recv() {
                Ok(request) => batch.push(request),
                Err(mpsc::error::TryRecvError::Empty) => break,
                Err(mpsc::error::TryRecvError::Disconnected) => break,
            }
        }
        let (attempts, replies): (Vec<_>, Vec<_>) = batch
            .into_iter()
            .map(|request| (request.attempt, request.reply))
            .unzip();
        match client.consume_rate_batch(attempts).await {
            Ok(decisions) if decisions.len() == replies.len() => {
                for (reply, decision) in replies.into_iter().zip(decisions) {
                    let _ = reply.send(Ok(decision));
                }
            }
            Ok(_) => fail_rate_batch(replies, "Nim returned an incomplete rate batch"),
            Err(error) => fail_rate_batch(replies, &error.to_string()),
        }
    }
}

fn fail_rate_batch(
    replies: Vec<oneshot::Sender<Result<RateAdmissionDecision, AdmissionRuntimeError>>>,
    message: &str,
) {
    for reply in replies {
        let _ = reply.send(Err(AdmissionRuntimeError::RateBatch(message.to_owned())));
    }
}

impl Drop for AdmissionRuntime {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn replay_committed(
    boundary: &BoundaryClient,
    store: &dyn ControlLogStorePort,
    projection: &Mutex<AdmissionProjection>,
    policy_timeout: Duration,
    invalidations: Option<&broadcast::Sender<AuthorizationInvalidationState>>,
) -> Result<(), AdmissionRuntimeError> {
    let recovered = store.recover_control_state()?;
    if recovered.snapshot.is_some() {
        return Err(AdmissionRuntimeError::CompactedControlLog);
    }
    let commit_index = recovered.metadata.state.commit_index;
    let mut projection = projection.lock().await;
    let after_index = projection.control_index;
    // ponytail: scan the un-compacted suffix; carry this projection in snapshots
    // when control-log compaction is enabled.
    for entry in recovered
        .entries
        .iter()
        .filter(|entry| entry.index > after_index && entry.index <= commit_index)
    {
        if let Some(command) = decode_command(entry)? {
            apply_command(
                boundary,
                &mut projection,
                command,
                entry.index,
                policy_timeout,
                invalidations,
            )
            .await?;
        }
        projection.control_index = entry.index;
    }
    Ok(())
}

async fn apply_command(
    boundary: &BoundaryClient,
    projection: &mut AdmissionProjection,
    command: AdmissionCommand,
    control_index: u64,
    policy_timeout: Duration,
    invalidations: Option<&broadcast::Sender<AuthorizationInvalidationState>>,
) -> Result<(), AdmissionRuntimeError> {
    match command {
        AdmissionCommand::ReplayClaim {
            scope,
            event_id,
            observed_at_ms,
            ttl_secs,
        } => {
            apply_replay(
                boundary,
                projection,
                scope,
                event_id,
                observed_at_ms,
                ttl_secs,
                control_index,
                policy_timeout,
            )
            .await
        }
        AdmissionCommand::RateLimit {
            namespace,
            key,
            observed_at_ms,
            window_secs,
            limit,
        } => {
            apply_rate(
                boundary,
                projection,
                namespace,
                key,
                observed_at_ms,
                window_secs,
                limit,
                control_index,
                policy_timeout,
            )
            .await?;
            Ok(())
        }
        AdmissionCommand::RateLimitBatch { attempts } => {
            if attempts.is_empty() || attempts.len() > RATE_BATCH_SIZE {
                return Err(AdmissionRuntimeError::InvalidCommand(
                    "rate batch size is out of bounds".to_owned(),
                ));
            }
            let mut current = BTreeMap::new();
            for attempt in &attempts {
                let key = (attempt.namespace.clone(), attempt.key.clone());
                if let Some(state) = projection.rate.get(&key) {
                    current.insert(key, state.clone());
                }
            }
            let expected = attempts.len();
            let result = call_policy(
                boundary,
                AdmissionPolicyRequest::ApplyRateLimitBatch {
                    states: current.into_values().collect(),
                    commands: attempts
                        .into_iter()
                        .map(|attempt| RateLimitCommand {
                            namespace: attempt.namespace,
                            key: attempt.key,
                            observed_at_ms: attempt.observed_at_ms,
                            window_secs: attempt.window_secs,
                            limit: attempt.limit,
                        })
                        .collect(),
                    control_index,
                },
                policy_timeout,
            )
            .await?;
            let AdmissionPolicyResult::ApplyRateLimitBatch { result } = result else {
                return Err(AdmissionRuntimeError::UnexpectedPolicyResult);
            };
            if result.error != AdmissionPolicyError::None {
                return Err(AdmissionRuntimeError::PolicyRejected(result.error));
            }
            if result.results.len() != expected {
                return Err(AdmissionRuntimeError::UnexpectedPolicyResult);
            }
            let mut decisions = Vec::with_capacity(expected);
            let mut updates = Vec::with_capacity(expected);
            for result in result.results {
                if result.error != AdmissionPolicyError::None {
                    return Err(AdmissionRuntimeError::PolicyRejected(result.error));
                }
                let state = result
                    .state
                    .ok_or(AdmissionRuntimeError::UnexpectedPolicyResult)?;
                updates.push(((state.namespace.clone(), state.key.clone()), state));
                decisions.push(RateAdmissionDecision {
                    allowed: result.allowed,
                    current: result.current,
                    limit: result.limit,
                    reset_in_secs: result.reset_in_secs,
                });
            }
            projection.rate.extend(updates);
            projection
                .rate_batch_outcomes
                .insert(control_index, decisions);
            while projection.rate_batch_outcomes.len() > MAX_RATE_BATCH_OUTCOMES {
                projection.rate_batch_outcomes.pop_first();
            }
            Ok(())
        }
        AdmissionCommand::AuthorizationInvalidation {
            scope,
            kind,
            subject,
            channel_id,
            fact_id,
        } => {
            apply_authorization_invalidation(
                boundary,
                projection,
                AuthorizationInvalidationCommand {
                    scope,
                    kind,
                    subject,
                    channel_id,
                    fact_id,
                },
                control_index,
                policy_timeout,
                invalidations,
            )
            .await
        }
    }
}

async fn apply_authorization_invalidation(
    boundary: &BoundaryClient,
    projection: &mut AdmissionProjection,
    command: AuthorizationInvalidationCommand,
    revision: u64,
    policy_timeout: Duration,
    invalidations: Option<&broadcast::Sender<AuthorizationInvalidationState>>,
) -> Result<(), AdmissionRuntimeError> {
    let key = (
        command.kind,
        command.scope.clone(),
        command.subject.clone(),
        command.channel_id.clone(),
    );
    let result = call_policy(
        boundary,
        AdmissionPolicyRequest::ApplyAuthorizationInvalidation {
            state: projection.invalidations.get(&key).cloned(),
            command,
            revision,
        },
        policy_timeout,
    )
    .await?;
    let AdmissionPolicyResult::ApplyAuthorizationInvalidation { result } = result else {
        return Err(AdmissionRuntimeError::UnexpectedPolicyResult);
    };
    if result.error != AuthorizationInvalidationError::None {
        return Err(AdmissionRuntimeError::AuthorizationPolicyRejected(
            result.error,
        ));
    }
    if result.effect == AuthorizationInvalidationEffect::Apply {
        let state = result
            .state
            .ok_or(AdmissionRuntimeError::UnexpectedPolicyResult)?;
        projection.invalidations.insert(key, state.clone());
        if let Some(invalidations) = invalidations {
            let _ = invalidations.send(state);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn apply_replay(
    boundary: &BoundaryClient,
    projection: &mut AdmissionProjection,
    scope: String,
    event_id: String,
    observed_at_ms: u64,
    ttl_secs: u64,
    control_index: u64,
    policy_timeout: Duration,
) -> Result<(), AdmissionRuntimeError> {
    prune_replay(boundary, projection, observed_at_ms, policy_timeout).await?;
    let key = (scope.clone(), event_id.clone());
    let result = call_policy(
        boundary,
        AdmissionPolicyRequest::ApplyReplayClaim {
            state: projection.replay.get(&key).cloned(),
            command: ReplayClaimCommand {
                scope,
                event_id,
                observed_at_ms,
                ttl_secs,
            },
            control_index,
        },
        policy_timeout,
    )
    .await?;
    let AdmissionPolicyResult::ApplyReplayClaim { result } = result else {
        return Err(AdmissionRuntimeError::UnexpectedPolicyResult);
    };
    if result.error != AdmissionPolicyError::None {
        return Err(AdmissionRuntimeError::PolicyRejected(result.error));
    }
    let state = result
        .state
        .ok_or(AdmissionRuntimeError::UnexpectedPolicyResult)?;
    projection.replay.insert(key, state);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn apply_rate(
    boundary: &BoundaryClient,
    projection: &mut AdmissionProjection,
    namespace: String,
    key: String,
    observed_at_ms: u64,
    window_secs: u64,
    limit: u64,
    control_index: u64,
    policy_timeout: Duration,
) -> Result<RateAdmissionDecision, AdmissionRuntimeError> {
    let projection_key = (namespace.clone(), key.clone());
    let result = call_policy(
        boundary,
        AdmissionPolicyRequest::ApplyRateLimit {
            state: projection.rate.get(&projection_key).cloned(),
            command: RateLimitCommand {
                namespace,
                key,
                observed_at_ms,
                window_secs,
                limit,
            },
            control_index,
        },
        policy_timeout,
    )
    .await?;
    let AdmissionPolicyResult::ApplyRateLimit { result } = result else {
        return Err(AdmissionRuntimeError::UnexpectedPolicyResult);
    };
    if result.error != AdmissionPolicyError::None {
        return Err(AdmissionRuntimeError::PolicyRejected(result.error));
    }
    let state = result
        .state
        .ok_or(AdmissionRuntimeError::UnexpectedPolicyResult)?;
    projection.rate.insert(projection_key, state);
    Ok(RateAdmissionDecision {
        allowed: result.allowed,
        current: result.current,
        limit: result.limit,
        reset_in_secs: result.reset_in_secs,
    })
}

async fn prune_replay(
    boundary: &BoundaryClient,
    projection: &mut AdmissionProjection,
    before_ms: u64,
    policy_timeout: Duration,
) -> Result<(), AdmissionRuntimeError> {
    if projection.replay.is_empty() {
        return Ok(());
    }
    let states = projection.replay.values().cloned().collect::<Vec<_>>();
    let mut retained = BTreeMap::new();
    for batch in states.chunks(PRUNE_BATCH) {
        let result = call_policy(
            boundary,
            AdmissionPolicyRequest::PruneReplay {
                states: batch.to_vec(),
                before_ms,
            },
            policy_timeout,
        )
        .await?;
        let AdmissionPolicyResult::PruneReplay { result } = result else {
            return Err(AdmissionRuntimeError::UnexpectedPolicyResult);
        };
        if result.error != AdmissionPolicyError::None {
            return Err(AdmissionRuntimeError::PolicyRejected(result.error));
        }
        retained.extend(
            result
                .retained
                .into_iter()
                .map(|state| ((state.scope.clone(), state.event_id.clone()), state)),
        );
    }
    projection.replay = retained;
    Ok(())
}

fn decode_command(
    entry: &nimino_store::ControlLogEntry,
) -> Result<Option<AdmissionCommand>, AdmissionRuntimeError> {
    if entry.kind != "command" {
        return Ok(None);
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&entry.payload) else {
        return Ok(None);
    };
    if value.get("domain").and_then(serde_json::Value::as_str) != Some(ADMISSION_COMMAND_DOMAIN) {
        return Ok(None);
    }
    let envelope: AdmissionEnvelope = serde_json::from_value(value)
        .map_err(|error| AdmissionRuntimeError::InvalidCommand(error.to_string()))?;
    if envelope.version != ADMISSION_COMMAND_VERSION {
        return Err(AdmissionRuntimeError::InvalidCommand(
            "unsupported admission command version".to_owned(),
        ));
    }
    let prefix = match envelope.command {
        AdmissionCommand::ReplayClaim { .. } => "admission:replay:",
        AdmissionCommand::RateLimit { .. } | AdmissionCommand::RateLimitBatch { .. } => {
            "admission:rate:"
        }
        AdmissionCommand::AuthorizationInvalidation { .. } => "admission:auth:",
    };
    if !entry.command_id.starts_with(prefix) {
        return Err(AdmissionRuntimeError::InvalidCommand(
            "admission command id has an invalid domain".to_owned(),
        ));
    }
    Ok(Some(envelope.command))
}

fn unix_ms() -> Result<u64, AdmissionRuntimeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AdmissionRuntimeError::InvalidClock)?
        .as_millis()
        .try_into()
        .map_err(|_| AdmissionRuntimeError::InvalidClock)
}

async fn call_policy(
    boundary: &BoundaryClient,
    request: AdmissionPolicyRequest,
    timeout: Duration,
) -> Result<AdmissionPolicyResult, AdmissionRuntimeError> {
    let result = boundary
        .call(
            BoundaryRequest::admission_policy(request),
            CallContext::with_timeout(timeout),
        )
        .await?;
    let BoundaryResult::AdmissionPolicy(result) = result else {
        return Err(AdmissionRuntimeError::UnexpectedPolicyResult);
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppliedControlEntry, ControlClient, ControlStatus};

    #[tokio::test]
    async fn waits_for_background_projection_recovery() {
        let (commands, _command_receiver) = tokio::sync::mpsc::channel(1);
        let (_control_status_sender, control_status) = watch::channel(ControlStatus {
            running: true,
            local_node_id: "node".to_owned(),
            term: 1,
            voter_epoch: 1,
            leader_id: Some("node".to_owned()),
            quorum_available: true,
            commit_index: 2,
            applied_index: 2,
            last_error: None,
        });
        let (applied, _) = broadcast::channel::<AppliedControlEntry>(1);
        let control = ControlClient {
            commands,
            status: control_status,
            applied,
            proposal_timeout: Duration::from_secs(1),
        };
        let (status_sender, status) = watch::channel(AdmissionRuntimeStatus {
            running: false,
            control_index: 1,
            last_error: Some("transient boundary timeout".to_owned()),
        });
        let (invalidations, _) = broadcast::channel(1);
        let (rate_requests, _rate_receiver) = mpsc::channel(1);
        let client = AdmissionClient {
            control,
            projection: Arc::new(Mutex::new(AdmissionProjection::default())),
            rate_requests,
            command_epoch: "test".to_owned(),
            command_sequence: Arc::new(AtomicU64::new(0)),
            status,
            invalidations,
        };
        assert!(matches!(
            client.require_running(),
            Err(AdmissionRuntimeError::Unavailable(cause))
                if cause == "transient boundary timeout"
        ));

        let waiting = tokio::spawn(async move { client.project_through(2).await });
        tokio::task::yield_now().await;
        assert!(!waiting.is_finished());
        status_sender.send_replace(AdmissionRuntimeStatus {
            running: true,
            control_index: 2,
            last_error: None,
        });

        assert!(waiting.await.expect("projection waiter joins").is_ok());
    }
}
