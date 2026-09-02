//! Process-bound leases derived from the committed Nimino control log.

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nimino_boundary::{
    ActiveLease, BoundaryClient, BoundaryError, BoundaryRequest, BoundaryResult, CallContext,
    CommittedLeaseFact, LeaseApplyMode, LeaseAuthority, LeaseCommand, LeaseEffect, LeaseFenceError,
    LeasePolicyRequest, LeasePolicyResult, LeaseRoute, LeaseState, ServingLeaseFact,
    SingletonEffectAttempt, SingletonEffectDecision,
};
use nimino_store::{ControlLogStorePort, StoreError};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    sync::{broadcast, watch, Mutex},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

use crate::{ControlClient, ControlRuntimeError};

const LEASE_COMMAND_VERSION: u8 = 1;
const LEASE_COMMAND_DOMAIN: &str = "nimino.lease.command";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LeaseEnvelope {
    domain: String,
    version: u8,
    command: LeaseCommand,
}

#[derive(Default)]
struct LeaseProjection {
    control_index: u64,
    states: BTreeMap<String, LeaseState>,
}

#[derive(Clone)]
struct ProcessClock {
    epoch: String,
    started: Instant,
}

impl ProcessClock {
    fn new(node_id: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            epoch: format!("{node_id}:{}:{nonce}", std::process::id()),
            started: Instant::now(),
        }
    }

    fn tick(&self) -> u64 {
        self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
}

/// Observable process-local lease projection health.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaseRuntimeStatus {
    /// Whether the projector is accepting lease decisions.
    pub running: bool,
    /// Highest control index observed by the lease projector.
    pub control_index: u64,
    /// Most recent terminal projector failure.
    pub last_error: Option<String>,
}

/// Lease projection, planning, or routing failure.
#[derive(Debug, Error)]
pub enum LeaseRuntimeError {
    /// The supervised Nim worker failed.
    #[error("Nim lease policy failed: {0}")]
    Boundary(#[from] BoundaryError),
    /// Replicated control rejected a lease command.
    #[error("lease control proposal failed: {0}")]
    Control(#[from] ControlRuntimeError),
    /// The durable control log failed.
    #[error("lease control log failed: {0}")]
    Store(#[from] StoreError),
    /// Nim rejected the supplied lease facts.
    #[error("Nim rejected lease facts: {0:?}")]
    PolicyRejected(LeaseFenceError),
    /// Nim returned a response for another lease decision.
    #[error("Nim returned an unexpected lease policy result")]
    UnexpectedPolicyResult,
    /// A committed lease payload violated its versioned envelope.
    #[error("invalid committed lease command: {0}")]
    InvalidCommand(String),
    /// A control snapshot exists but does not contain a lease projection.
    #[error("lease recovery does not support compacted control logs")]
    CompactedControlLog,
    /// The lease projector is stopped or failed closed.
    #[error("lease runtime stopped")]
    Stopped,
    /// The projector task panicked.
    #[error("lease runtime task failed")]
    TaskFailed,
}

/// Cloneable facade for quorum-backed lease grants and fenced decisions.
#[derive(Clone)]
pub struct LeaseClient {
    boundary: BoundaryClient,
    control: ControlClient,
    store: Arc<dyn ControlLogStorePort>,
    projection: Arc<Mutex<LeaseProjection>>,
    clock: ProcessClock,
    policy_timeout: Duration,
    status: watch::Receiver<LeaseRuntimeStatus>,
}

impl LeaseClient {
    /// Returns this process's stable Chirps identity.
    pub fn local_node_id(&self) -> String {
        self.control.status().local_node_id
    }

    /// Returns the exact state and serving facts used by Nim effect policy.
    pub async fn policy_context(
        &self,
        resource_id: &str,
    ) -> Result<(LeaseState, ServingLeaseFact), LeaseRuntimeError> {
        self.require_running()?;
        Ok((
            self.state_or_initial(resource_id).await,
            self.serving_fact(),
        ))
    }

    /// Proposes a deterministic lease grant and returns its active incarnation.
    pub async fn grant(
        &self,
        resource_id: impl Into<String>,
        transition_id: impl Into<String>,
        eligible_owners: Vec<String>,
        duration_ticks: u64,
    ) -> Result<ActiveLease, LeaseRuntimeError> {
        self.require_running()?;
        let resource_id = resource_id.into();
        let transition_id = transition_id.into();
        let state = self.state_or_initial(&resource_id).await;
        let status = self.control.status();
        let result = call_policy(
            &self.boundary,
            LeasePolicyRequest::PlanGrant {
                state,
                authority: LeaseAuthority {
                    leader_id: status.leader_id.unwrap_or_default(),
                    term: status.term,
                    voter_epoch: status.voter_epoch,
                    quorum_available: status.quorum_available,
                },
                transition_id,
                eligible_owners,
                duration_ticks,
            },
            self.policy_timeout,
        )
        .await?;
        let LeasePolicyResult::PlanGrant { result } = result else {
            return Err(LeaseRuntimeError::UnexpectedPolicyResult);
        };
        if result.error != LeaseFenceError::None {
            return Err(LeaseRuntimeError::PolicyRejected(result.error));
        }
        let command = result
            .command
            .ok_or(LeaseRuntimeError::UnexpectedPolicyResult)?;
        if result.effect == LeaseEffect::Propose {
            let payload = serde_json::to_string(&LeaseEnvelope {
                domain: LEASE_COMMAND_DOMAIN.to_owned(),
                version: LEASE_COMMAND_VERSION,
                command: command.clone(),
            })
            .map_err(|error| LeaseRuntimeError::InvalidCommand(error.to_string()))?;
            self.control
                .propose(command.transition_id.clone(), payload)
                .await?;
            replay_committed(
                &self.boundary,
                self.store.as_ref(),
                &self.projection,
                &self.clock,
                self.policy_timeout,
                LeaseApplyMode::Live,
            )
            .await?;
        } else if result.effect != LeaseEffect::Replay {
            return Err(LeaseRuntimeError::UnexpectedPolicyResult);
        }
        self.state(&resource_id)
            .await
            .and_then(|state| state.active_lease)
            .ok_or(LeaseRuntimeError::PolicyRejected(
                LeaseFenceError::NoActiveLease,
            ))
    }

    /// Routes one singleton request through the current Nim lease decision.
    pub async fn route(&self, resource_id: &str) -> Result<LeaseRoute, LeaseRuntimeError> {
        self.require_running()?;
        let result = call_policy(
            &self.boundary,
            LeasePolicyRequest::Route {
                state: self.state_or_initial(resource_id).await,
                fact: self.serving_fact(),
            },
            self.policy_timeout,
        )
        .await?;
        let LeasePolicyResult::Route { result } = result else {
            return Err(LeaseRuntimeError::UnexpectedPolicyResult);
        };
        Ok(result)
    }

    /// Authorizes one owner/fence pair immediately before a singleton effect.
    pub async fn authorize(
        &self,
        attempt: SingletonEffectAttempt,
    ) -> Result<SingletonEffectDecision, LeaseRuntimeError> {
        self.require_running()?;
        let result = call_policy(
            &self.boundary,
            LeasePolicyRequest::Authorize {
                state: self.state_or_initial(&attempt.resource_id).await,
                attempt,
                fact: self.serving_fact(),
            },
            self.policy_timeout,
        )
        .await?;
        let LeasePolicyResult::Authorize { result } = result else {
            return Err(LeaseRuntimeError::UnexpectedPolicyResult);
        };
        Ok(result)
    }

    /// Returns the latest projected state for diagnostics and tests.
    pub async fn state(&self, resource_id: &str) -> Option<LeaseState> {
        self.projection
            .lock()
            .await
            .states
            .get(resource_id)
            .cloned()
    }

    /// Returns projector health without performing I/O.
    pub fn status(&self) -> LeaseRuntimeStatus {
        self.status.borrow().clone()
    }

    async fn state_or_initial(&self, resource_id: &str) -> LeaseState {
        self.state(resource_id).await.unwrap_or_else(|| LeaseState {
            valid: !resource_id.is_empty(),
            resource_id: resource_id.to_owned(),
            last_fence_token: 0,
            last_control_index: 0,
            last_command: None,
            active_lease: None,
        })
    }

    fn serving_fact(&self) -> ServingLeaseFact {
        let status = self.control.status();
        ServingLeaseFact {
            quorum_available: status.quorum_available,
            leader_id: status.leader_id.unwrap_or_default(),
            term: status.term,
            voter_epoch: status.voter_epoch,
            clock_epoch: self.clock.epoch.clone(),
            now_tick: self.clock.tick(),
        }
    }

    fn require_running(&self) -> Result<(), LeaseRuntimeError> {
        if self.status.borrow().running {
            Ok(())
        } else {
            Err(LeaseRuntimeError::Stopped)
        }
    }
}

/// Lifecycle owner for the process-local view of replicated leases.
pub struct LeaseRuntime {
    client: LeaseClient,
    shutdown: CancellationToken,
    task: Option<JoinHandle<Result<(), LeaseRuntimeError>>>,
}

impl LeaseRuntime {
    /// Recovers fences without reviving leases, then follows live control commits.
    pub async fn start(
        control: ControlClient,
        boundary: BoundaryClient,
        store: Arc<dyn ControlLogStorePort>,
        policy_timeout: Duration,
    ) -> Result<Self, LeaseRuntimeError> {
        let clock = ProcessClock::new(&control.status().local_node_id);
        let projection = Arc::new(Mutex::new(LeaseProjection::default()));
        let mut applied = control.subscribe_applied();
        replay_committed(
            &boundary,
            store.as_ref(),
            &projection,
            &clock,
            policy_timeout,
            LeaseApplyMode::Recovery,
        )
        .await?;
        let control_index = projection.lock().await.control_index;
        let (status_sender, status) = watch::channel(LeaseRuntimeStatus {
            running: true,
            control_index,
            last_error: None,
        });
        let shutdown = CancellationToken::new();
        let task_shutdown = shutdown.clone();
        let task_boundary = boundary.clone();
        let task_store = store.clone();
        let task_projection = projection.clone();
        let task_clock = clock.clone();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = task_shutdown.cancelled() => {
                        status_sender.send_replace(LeaseRuntimeStatus {
                            running: false,
                            control_index: task_projection.lock().await.control_index,
                            last_error: None,
                        });
                        return Ok(());
                    }
                    result = applied.recv() => match result {
                        Ok(event) => {
                            if let Err(error) = apply_committed_entry(
                                &task_boundary,
                                &task_projection,
                                &event.entry,
                                &task_clock,
                                policy_timeout,
                                if event.recovered {
                                    LeaseApplyMode::Recovery
                                } else {
                                    LeaseApplyMode::Live
                                },
                            ).await {
                                status_sender.send_replace(LeaseRuntimeStatus {
                                    running: false,
                                    control_index: task_projection.lock().await.control_index,
                                    last_error: Some(error.to_string()),
                                });
                                return Err(error);
                            }
                            status_sender.send_replace(LeaseRuntimeStatus {
                                running: true,
                                control_index: task_projection.lock().await.control_index,
                                last_error: None,
                            });
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if let Err(error) = replay_committed(
                                &task_boundary,
                                task_store.as_ref(),
                                &task_projection,
                                &task_clock,
                                policy_timeout,
                                LeaseApplyMode::Recovery,
                            ).await {
                                status_sender.send_replace(LeaseRuntimeStatus {
                                    running: false,
                                    control_index: task_projection.lock().await.control_index,
                                    last_error: Some(error.to_string()),
                                });
                                return Err(error);
                            }
                            status_sender.send_replace(LeaseRuntimeStatus {
                                running: true,
                                control_index: task_projection.lock().await.control_index,
                                last_error: None,
                            });
                        }
                        Err(broadcast::error::RecvError::Closed) => {
                            status_sender.send_replace(LeaseRuntimeStatus {
                                running: false,
                                control_index: task_projection.lock().await.control_index,
                                last_error: Some(LeaseRuntimeError::Stopped.to_string()),
                            });
                            return Err(LeaseRuntimeError::Stopped);
                        }
                    }
                }
            }
        });
        Ok(Self {
            client: LeaseClient {
                boundary,
                control,
                store,
                projection,
                clock,
                policy_timeout,
                status,
            },
            shutdown,
            task: Some(task),
        })
    }

    /// Returns a cloneable lease facade.
    pub fn client(&self) -> LeaseClient {
        self.client.clone()
    }

    /// Stops the projector and joins its task.
    pub async fn stop(mut self) -> Result<(), LeaseRuntimeError> {
        self.shutdown.cancel();
        if let Some(task) = self.task.take() {
            task.await.map_err(|_| LeaseRuntimeError::TaskFailed)??;
        }
        Ok(())
    }
}

impl Drop for LeaseRuntime {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

async fn apply_committed_entry(
    boundary: &BoundaryClient,
    projection: &Mutex<LeaseProjection>,
    entry: &nimino_boundary::ControlEntry,
    clock: &ProcessClock,
    policy_timeout: Duration,
    mode: LeaseApplyMode,
) -> Result<(), LeaseRuntimeError> {
    let entry = crate::store_entry(entry);
    let mut projection = projection.lock().await;
    if entry.index <= projection.control_index {
        return Ok(());
    }
    if entry.index != projection.control_index.saturating_add(1) {
        return Err(LeaseRuntimeError::InvalidCommand(
            "committed lease projection has an index gap".to_owned(),
        ));
    }
    apply_stored_entry(
        boundary,
        &mut projection,
        &entry,
        clock,
        policy_timeout,
        mode,
    )
    .await
}

async fn replay_committed(
    boundary: &BoundaryClient,
    store: &dyn ControlLogStorePort,
    projection: &Mutex<LeaseProjection>,
    clock: &ProcessClock,
    policy_timeout: Duration,
    mode: LeaseApplyMode,
) -> Result<(), LeaseRuntimeError> {
    let recovered = store.recover_control_state()?;
    if recovered.snapshot.is_some() {
        return Err(LeaseRuntimeError::CompactedControlLog);
    }
    let commit_index = recovered.metadata.state.commit_index;
    let mut projection = projection.lock().await;
    let after_index = projection.control_index;
    // ponytail: scan the un-compacted control suffix; add snapshot-carried lease state when
    // control-log compaction is enabled.
    for entry in recovered
        .entries
        .iter()
        .filter(|entry| entry.index > after_index && entry.index <= commit_index)
    {
        apply_stored_entry(
            boundary,
            &mut projection,
            entry,
            clock,
            policy_timeout,
            mode,
        )
        .await?;
    }
    Ok(())
}

async fn apply_stored_entry(
    boundary: &BoundaryClient,
    projection: &mut LeaseProjection,
    entry: &nimino_store::ControlLogEntry,
    clock: &ProcessClock,
    policy_timeout: Duration,
    mode: LeaseApplyMode,
) -> Result<(), LeaseRuntimeError> {
    if let Some(command) = decode_command(entry)? {
        let state = projection
            .states
            .get(&command.resource_id)
            .cloned()
            .unwrap_or_else(|| LeaseState {
                valid: !command.resource_id.is_empty(),
                resource_id: command.resource_id.clone(),
                last_fence_token: 0,
                last_control_index: 0,
                last_command: None,
                active_lease: None,
            });
        let result = call_policy(
            boundary,
            LeasePolicyRequest::ApplyCommitted {
                state,
                fact: CommittedLeaseFact {
                    committed: true,
                    control_index: entry.index,
                    leader_id: command.leader_id.clone(),
                    term: entry.term,
                    voter_epoch: entry.voter_epoch,
                    clock_epoch: clock.epoch.clone(),
                    now_tick: clock.tick(),
                },
                command,
                mode,
            },
            policy_timeout,
        )
        .await?;
        let LeasePolicyResult::ApplyCommitted { result } = result else {
            return Err(LeaseRuntimeError::UnexpectedPolicyResult);
        };
        if result.error != LeaseFenceError::None {
            return Err(LeaseRuntimeError::PolicyRejected(result.error));
        }
        projection
            .states
            .insert(result.state.resource_id.clone(), result.state);
    }
    projection.control_index = entry.index;
    Ok(())
}

fn decode_command(
    entry: &nimino_store::ControlLogEntry,
) -> Result<Option<LeaseCommand>, LeaseRuntimeError> {
    if entry.kind != "command" {
        return Ok(None);
    }
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&entry.payload) else {
        return Ok(None);
    };
    if value.get("domain").and_then(serde_json::Value::as_str) != Some(LEASE_COMMAND_DOMAIN) {
        return Ok(None);
    }
    let envelope: LeaseEnvelope = serde_json::from_value(value)
        .map_err(|error| LeaseRuntimeError::InvalidCommand(error.to_string()))?;
    if envelope.version != LEASE_COMMAND_VERSION {
        return Err(LeaseRuntimeError::InvalidCommand(
            "unsupported lease command version".to_owned(),
        ));
    }
    if envelope.command.transition_id != entry.command_id
        || envelope.command.term != entry.term
        || envelope.command.voter_epoch != entry.voter_epoch
    {
        return Err(LeaseRuntimeError::InvalidCommand(
            "lease command does not match its control entry".to_owned(),
        ));
    }
    Ok(Some(envelope.command))
}

async fn call_policy(
    boundary: &BoundaryClient,
    request: LeasePolicyRequest,
    timeout: Duration,
) -> Result<LeasePolicyResult, LeaseRuntimeError> {
    let result = boundary
        .call(
            BoundaryRequest::lease_policy(request),
            CallContext::with_timeout(timeout),
        )
        .await?;
    let BoundaryResult::LeasePolicy(result) = result else {
        return Err(LeaseRuntimeError::UnexpectedPolicyResult);
    };
    Ok(result)
}
