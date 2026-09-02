//! Bounded presence and typing convergence over authenticated Chirps messages.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use nimino_boundary::{
    BoundaryClient, BoundaryError, BoundaryRequest, BoundaryResult, CallContext, EphemeralCommand,
    EphemeralDecision, EphemeralEffect, EphemeralKind, EphemeralPolicyError,
    EphemeralPolicyRequest, EphemeralPolicyResult, EphemeralState,
};
use nimino_chirps::{MeshClient, MeshRuntimeError, NodeId};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;

const WIRE_PREFIX: &[u8] = b"NIMINO-EPHEMERAL/1\n";
const MAX_CAPACITY: usize = 4_096;
const MAX_EVENT_JSON_BYTES: usize = 32 * 1024;
const ADVERTISEMENT_BATCH: usize = 64;
const PRESENCE_TTL_SECS: u64 = 180;
const TYPING_TTL_SECS: u64 = 10;
const TOMBSTONE_TTL_SECS: u64 = 600;

/// Timing and capacity bounds for presence and typing convergence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EphemeralRuntimeOptions {
    advertisement_interval: Duration,
    policy_timeout: Duration,
    command_capacity: usize,
    max_states: usize,
}

impl EphemeralRuntimeOptions {
    /// Creates explicit runtime bounds validated by [`EphemeralRuntime::start`].
    pub fn new(
        advertisement_interval: Duration,
        policy_timeout: Duration,
        command_capacity: usize,
        max_states: usize,
    ) -> Self {
        Self {
            advertisement_interval,
            policy_timeout,
            command_capacity,
            max_states,
        }
    }

    fn validate(self) -> Result<(), EphemeralRuntimeError> {
        if self.advertisement_interval.is_zero()
            || self.policy_timeout.is_zero()
            || self.command_capacity == 0
            || self.command_capacity > MAX_CAPACITY
            || self.max_states == 0
            || self.max_states > MAX_CAPACITY
        {
            return Err(EphemeralRuntimeError::InvalidConfiguration);
        }
        Ok(())
    }
}

impl Default for EphemeralRuntimeOptions {
    fn default() -> Self {
        Self::new(Duration::from_secs(5), Duration::from_secs(2), 256, 4_096)
    }
}

/// One authenticated remote transition ready for relay-local projection/fan-out.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EphemeralUpdate {
    /// Nim-owned converged state.
    pub state: EphemeralState,
    /// Original verified Nostr event JSON for an active transition.
    pub event_json: Option<String>,
}

/// Typed lifecycle, policy, capacity, and wire failure.
#[derive(Debug, Error)]
pub enum EphemeralRuntimeError {
    /// Timings or queue/state bounds are empty or unsafe.
    #[error("invalid ephemeral runtime configuration")]
    InvalidConfiguration,
    /// The supervised Nim worker failed.
    #[error("Nim ephemeral policy failed: {0}")]
    Boundary(#[from] BoundaryError),
    /// Chirps rejected or stopped a transport operation.
    #[error("Chirps transport failed: {0}")]
    Transport(#[from] MeshRuntimeError),
    /// Nim returned a response for another policy operation.
    #[error("Nim returned an unexpected ephemeral policy result")]
    UnexpectedPolicyResult,
    /// Nim rejected the supplied state or transition.
    #[error("Nim rejected ephemeral facts: {0:?}")]
    PolicyRejected(EphemeralPolicyError),
    /// An authenticated payload violated the versioned wire contract.
    #[error("invalid ephemeral frame: {0}")]
    InvalidFrame(String),
    /// The community is not active on this relay.
    #[error("ephemeral community scope is unavailable")]
    ScopeUnavailable,
    /// The bounded projection has reached its fixed capacity.
    #[error("ephemeral projection capacity reached")]
    Capacity,
    /// The bounded command queue is full.
    #[error("ephemeral command queue is full")]
    Backpressure,
    /// The runtime has stopped.
    #[error("ephemeral runtime stopped")]
    Stopped,
    /// Wall-clock conversion failed.
    #[error("system clock is before the Unix epoch")]
    InvalidClock,
    /// The background task panicked.
    #[error("ephemeral runtime task failed")]
    TaskFailed,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct EphemeralKey {
    scope: String,
    kind: EphemeralKind,
    subject: String,
    context: String,
    origin_node_id: String,
}

impl From<&EphemeralCommand> for EphemeralKey {
    fn from(command: &EphemeralCommand) -> Self {
        Self {
            scope: command.scope.clone(),
            kind: command.kind,
            subject: command.subject.clone(),
            context: command.context.clone(),
            origin_node_id: command.origin_node_id.clone(),
        }
    }
}

#[derive(Clone)]
struct Projection {
    state: EphemeralState,
    event_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireFrame {
    transitions: Vec<WireTransition>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct WireTransition {
    command: EphemeralCommand,
    event_json: Option<String>,
}

enum RuntimeCommand {
    Publish {
        kind: EphemeralKind,
        scope: String,
        subject: String,
        context: String,
        value: String,
        observed_at_ms: u64,
        transition_id: String,
        event_json: String,
        reply: oneshot::Sender<Result<EphemeralDecision, EphemeralRuntimeError>>,
    },
    Clear {
        kind: EphemeralKind,
        scope: String,
        subject: String,
        context: String,
        observed_at_ms: Option<u64>,
        transition_id: Option<String>,
        reply: oneshot::Sender<Result<EphemeralDecision, EphemeralRuntimeError>>,
    },
    Presence {
        scope: String,
        subjects: Vec<String>,
        reply: oneshot::Sender<Result<HashMap<String, String>, EphemeralRuntimeError>>,
    },
}

/// Cloneable publish, query, scope, and subscription facade.
#[derive(Clone)]
pub struct EphemeralClient {
    commands: mpsc::Sender<RuntimeCommand>,
    scopes: watch::Sender<BTreeSet<String>>,
    updates: broadcast::Sender<EphemeralUpdate>,
    stopped: watch::Receiver<bool>,
    response_timeout: Duration,
}

impl EphemeralClient {
    /// Publishes one verified presence event.
    pub async fn publish_presence(
        &self,
        scope: impl Into<String>,
        subject: impl Into<String>,
        value: impl Into<String>,
        observed_at_ms: u64,
        transition_id: impl Into<String>,
        event_json: impl Into<String>,
    ) -> Result<EphemeralDecision, EphemeralRuntimeError> {
        self.publish(
            EphemeralKind::Presence,
            scope.into(),
            subject.into(),
            String::new(),
            value.into(),
            observed_at_ms,
            transition_id.into(),
            event_json.into(),
        )
        .await
    }

    /// Publishes one verified channel-scoped typing event.
    #[allow(clippy::too_many_arguments)] // Exact signed-event facts; a wrapper would only relocate them.
    pub async fn publish_typing(
        &self,
        scope: impl Into<String>,
        subject: impl Into<String>,
        context: impl Into<String>,
        value: impl Into<String>,
        observed_at_ms: u64,
        transition_id: impl Into<String>,
        event_json: impl Into<String>,
    ) -> Result<EphemeralDecision, EphemeralRuntimeError> {
        self.publish(
            EphemeralKind::Typing,
            scope.into(),
            subject.into(),
            context.into(),
            value.into(),
            observed_at_ms,
            transition_id.into(),
            event_json.into(),
        )
        .await
    }

    /// Publishes a tombstone for one user's presence.
    pub async fn clear_presence(
        &self,
        scope: impl Into<String>,
        subject: impl Into<String>,
    ) -> Result<EphemeralDecision, EphemeralRuntimeError> {
        self.clear(
            EphemeralKind::Presence,
            scope.into(),
            subject.into(),
            String::new(),
            None,
            None,
        )
        .await
    }

    /// Publishes a signed offline-event tombstone with its stable event ordering facts.
    pub async fn clear_presence_at(
        &self,
        scope: impl Into<String>,
        subject: impl Into<String>,
        observed_at_ms: u64,
        transition_id: impl Into<String>,
    ) -> Result<EphemeralDecision, EphemeralRuntimeError> {
        self.clear(
            EphemeralKind::Presence,
            scope.into(),
            subject.into(),
            String::new(),
            Some(observed_at_ms),
            Some(transition_id.into()),
        )
        .await
    }

    /// Returns live presence values for an exact community scope.
    pub async fn presence(
        &self,
        scope: impl Into<String>,
        subjects: Vec<String>,
    ) -> Result<HashMap<String, String>, EphemeralRuntimeError> {
        self.request(|reply| RuntimeCommand::Presence {
            scope: scope.into(),
            subjects,
            reply,
        })
        .await
    }

    /// Replaces the exact set of active community scopes.
    pub fn replace_scopes(
        &self,
        scopes: impl IntoIterator<Item = String>,
    ) -> Result<(), EphemeralRuntimeError> {
        self.scopes
            .send(scopes.into_iter().collect())
            .map_err(|_| EphemeralRuntimeError::Stopped)
    }

    /// Subscribes to newly applied authenticated remote transitions.
    pub fn subscribe_remote(&self) -> broadcast::Receiver<EphemeralUpdate> {
        self.updates.subscribe()
    }

    /// Returns whether the runtime task remains live.
    pub fn is_running(&self) -> bool {
        !*self.stopped.borrow()
    }

    #[allow(clippy::too_many_arguments)]
    async fn publish(
        &self,
        kind: EphemeralKind,
        scope: String,
        subject: String,
        context: String,
        value: String,
        observed_at_ms: u64,
        transition_id: String,
        event_json: String,
    ) -> Result<EphemeralDecision, EphemeralRuntimeError> {
        if event_json.len() > MAX_EVENT_JSON_BYTES {
            return Err(EphemeralRuntimeError::InvalidFrame(
                "event JSON exceeds fixed limit".to_owned(),
            ));
        }
        self.request(|reply| RuntimeCommand::Publish {
            kind,
            scope,
            subject,
            context,
            value,
            observed_at_ms,
            transition_id,
            event_json,
            reply,
        })
        .await
    }

    async fn clear(
        &self,
        kind: EphemeralKind,
        scope: String,
        subject: String,
        context: String,
        observed_at_ms: Option<u64>,
        transition_id: Option<String>,
    ) -> Result<EphemeralDecision, EphemeralRuntimeError> {
        self.request(|reply| RuntimeCommand::Clear {
            kind,
            scope,
            subject,
            context,
            observed_at_ms,
            transition_id,
            reply,
        })
        .await
    }

    async fn request<T>(
        &self,
        make_command: impl FnOnce(oneshot::Sender<Result<T, EphemeralRuntimeError>>) -> RuntimeCommand,
    ) -> Result<T, EphemeralRuntimeError> {
        if *self.stopped.borrow() {
            return Err(EphemeralRuntimeError::Stopped);
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .try_send(make_command(reply))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => EphemeralRuntimeError::Backpressure,
                mpsc::error::TrySendError::Closed(_) => EphemeralRuntimeError::Stopped,
            })?;
        match tokio::time::timeout(self.response_timeout, response).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) | Err(_) => Err(EphemeralRuntimeError::Stopped),
        }
    }
}

/// Lifecycle owner for one bounded ephemeral convergence task.
pub struct EphemeralRuntime {
    client: EphemeralClient,
    shutdown: CancellationToken,
    task: Option<JoinHandle<Result<(), EphemeralRuntimeError>>>,
}

impl EphemeralRuntime {
    /// Starts presence/typing convergence for the supplied community scopes.
    pub fn start(
        mesh: MeshClient,
        boundary: BoundaryClient,
        scopes: impl IntoIterator<Item = String>,
        options: EphemeralRuntimeOptions,
    ) -> Result<Self, EphemeralRuntimeError> {
        options.validate()?;
        let (commands, command_receiver) = mpsc::channel(options.command_capacity);
        let (scope_sender, scope_receiver) = watch::channel(scopes.into_iter().collect());
        let (updates, _) = broadcast::channel(options.command_capacity);
        let (stopped_sender, stopped) = watch::channel(false);
        let shutdown = CancellationToken::new();
        let context = RuntimeContext {
            local_node_id: hex::encode(mesh.local_node_id().as_bytes()),
            mesh,
            boundary,
            scopes: scope_receiver,
            projections: BTreeMap::new(),
            commands: command_receiver,
            updates: updates.clone(),
            stopped: stopped_sender,
            options,
            sequence: 0,
            advertisement_cursor: 0,
        };
        let task_shutdown = shutdown.clone();
        let task = tokio::spawn(async move { context.run(task_shutdown).await });
        Ok(Self {
            client: EphemeralClient {
                commands,
                scopes: scope_sender,
                updates,
                stopped,
                response_timeout: options.policy_timeout.saturating_mul(2),
            },
            shutdown,
            task: Some(task),
        })
    }

    /// Returns a cloneable publish/query facade.
    pub fn client(&self) -> EphemeralClient {
        self.client.clone()
    }

    /// Publishes local tombstones, stops the task, and releases its subscriptions.
    pub async fn stop(mut self) -> Result<(), EphemeralRuntimeError> {
        self.shutdown.cancel();
        if let Some(task) = self.task.take() {
            task.await
                .map_err(|_| EphemeralRuntimeError::TaskFailed)??;
        }
        Ok(())
    }
}

impl Drop for EphemeralRuntime {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

struct RuntimeContext {
    local_node_id: String,
    mesh: MeshClient,
    boundary: BoundaryClient,
    scopes: watch::Receiver<BTreeSet<String>>,
    projections: BTreeMap<EphemeralKey, Projection>,
    commands: mpsc::Receiver<RuntimeCommand>,
    updates: broadcast::Sender<EphemeralUpdate>,
    stopped: watch::Sender<bool>,
    options: EphemeralRuntimeOptions,
    sequence: u64,
    advertisement_cursor: usize,
}

impl RuntimeContext {
    async fn run(mut self, shutdown: CancellationToken) -> Result<(), EphemeralRuntimeError> {
        let mut messages = self.mesh.subscribe();
        let mut tick = tokio::time::interval(self.options.advertisement_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    let cleanup = self.publish_shutdown_tombstones().await;
                    self.stopped.send_replace(true);
                    return cleanup;
                }
                _ = tick.tick() => {
                    if let Err(error) = self.on_tick().await {
                        tracing_error(&error);
                    }
                }
                command = self.commands.recv() => match command {
                    Some(command) => self.handle_command(command).await,
                    None => {
                        self.stopped.send_replace(true);
                        return Ok(());
                    }
                },
                message = messages.recv() => match message {
                    Ok(message) => {
                        if let Err(error) = self.handle_message(message.from(), message.payload()).await {
                            tracing_error(&error);
                        }
                    }
                    Err(MeshRuntimeError::SubscriberLagged { skipped }) => {
                        tracing_error(&EphemeralRuntimeError::Transport(
                            MeshRuntimeError::SubscriberLagged { skipped },
                        ));
                    }
                    Err(error) => {
                        self.stopped.send_replace(true);
                        return Err(error.into());
                    }
                }
            }
        }
    }

    async fn handle_command(&mut self, command: RuntimeCommand) {
        match command {
            RuntimeCommand::Publish {
                kind,
                scope,
                subject,
                context,
                value,
                observed_at_ms,
                transition_id,
                event_json,
                reply,
            } => {
                let ttl_secs = match kind {
                    EphemeralKind::Presence => PRESENCE_TTL_SECS,
                    EphemeralKind::Typing => TYPING_TTL_SECS,
                };
                let command = EphemeralCommand {
                    scope,
                    kind,
                    subject,
                    context,
                    value,
                    active: true,
                    observed_at_ms,
                    ttl_secs,
                    origin_node_id: self.local_node_id.clone(),
                    transition_id,
                };
                let result = self.apply_local(command, Some(event_json)).await;
                let _ = reply.send(result);
            }
            RuntimeCommand::Clear {
                kind,
                scope,
                subject,
                context,
                observed_at_ms,
                transition_id,
                reply,
            } => {
                let result = self
                    .tombstone(kind, scope, subject, context, observed_at_ms, transition_id)
                    .await;
                let _ = reply.send(result);
            }
            RuntimeCommand::Presence {
                scope,
                subjects,
                reply,
            } => {
                let now = unix_ms();
                let result = now.map(|now| {
                    subjects
                        .into_iter()
                        .filter_map(|subject| {
                            self.projections
                                .values()
                                .filter(|projection| {
                                    projection.state.scope == scope
                                        && projection.state.kind == EphemeralKind::Presence
                                        && projection.state.subject == subject
                                        && projection.state.active
                                        && projection.state.expires_at_ms >= now
                                })
                                .max_by_key(|projection| {
                                    (
                                        projection.state.observed_at_ms,
                                        projection.state.origin_node_id.as_str(),
                                        projection.state.transition_id.as_str(),
                                    )
                                })
                                .map(|projection| (subject, projection.state.value.clone()))
                        })
                        .collect()
                });
                let _ = reply.send(result);
            }
        }
    }

    async fn apply_local(
        &mut self,
        command: EphemeralCommand,
        event_json: Option<String>,
    ) -> Result<EphemeralDecision, EphemeralRuntimeError> {
        if !self.scopes.borrow().contains(&command.scope) {
            return Err(EphemeralRuntimeError::ScopeUnavailable);
        }
        let transition = WireTransition {
            command: command.clone(),
            event_json: event_json.clone(),
        };
        let decision = self.apply(command, event_json, false).await?;
        if decision.effect == EphemeralEffect::Apply {
            if let Err(error) = self.broadcast(vec![transition]).await {
                tracing_error(&error);
            }
        }
        Ok(decision)
    }

    async fn handle_message(
        &mut self,
        authenticated_peer: NodeId,
        payload: &[u8],
    ) -> Result<(), EphemeralRuntimeError> {
        let Some(payload) = payload.strip_prefix(WIRE_PREFIX) else {
            return Ok(());
        };
        let frame: WireFrame = serde_json::from_slice(payload)
            .map_err(|error| EphemeralRuntimeError::InvalidFrame(error.to_string()))?;
        if frame.transitions.is_empty() || frame.transitions.len() > ADVERTISEMENT_BATCH {
            return Err(EphemeralRuntimeError::InvalidFrame(
                "transition batch exceeds fixed limit".to_owned(),
            ));
        }
        let peer = hex::encode(authenticated_peer.as_bytes());
        for transition in frame.transitions {
            if transition
                .event_json
                .as_ref()
                .is_some_and(|event| event.len() > MAX_EVENT_JSON_BYTES)
            {
                return Err(EphemeralRuntimeError::InvalidFrame(
                    "event JSON exceeds fixed limit".to_owned(),
                ));
            }
            if transition.command.origin_node_id != peer {
                return Err(EphemeralRuntimeError::InvalidFrame(
                    "origin does not match authenticated peer".to_owned(),
                ));
            }
            if self.scopes.borrow().contains(&transition.command.scope) {
                self.apply(transition.command, transition.event_json, true)
                    .await?;
            }
        }
        Ok(())
    }

    async fn apply(
        &mut self,
        command: EphemeralCommand,
        event_json: Option<String>,
        remote: bool,
    ) -> Result<EphemeralDecision, EphemeralRuntimeError> {
        let key = EphemeralKey::from(&command);
        if !self.projections.contains_key(&key) && self.projections.len() >= self.options.max_states
        {
            return Err(EphemeralRuntimeError::Capacity);
        }
        let current = self.projections.get(&key).map(|entry| entry.state.clone());
        let result = call_policy(
            &self.boundary,
            EphemeralPolicyRequest::Apply {
                state: current,
                command,
                now_ms: unix_ms()?,
            },
            self.options.policy_timeout,
        )
        .await?;
        let EphemeralPolicyResult::Apply { result } = result else {
            return Err(EphemeralRuntimeError::UnexpectedPolicyResult);
        };
        if result.error != EphemeralPolicyError::None {
            return Err(EphemeralRuntimeError::PolicyRejected(result.error));
        }
        if result.effect == EphemeralEffect::Apply {
            let state = result
                .state
                .clone()
                .ok_or(EphemeralRuntimeError::UnexpectedPolicyResult)?;
            self.projections.insert(
                key,
                Projection {
                    state: state.clone(),
                    event_json: event_json.clone(),
                },
            );
            if remote {
                let _ = self.updates.send(EphemeralUpdate { state, event_json });
            }
        }
        Ok(result)
    }

    async fn on_tick(&mut self) -> Result<(), EphemeralRuntimeError> {
        let now = unix_ms()?;
        self.projections
            .retain(|_, projection| projection.state.expires_at_ms >= now);
        let local = self
            .projections
            .values()
            .filter(|projection| projection.state.origin_node_id == self.local_node_id)
            .cloned()
            .collect::<Vec<_>>();
        if local.is_empty() {
            self.advertisement_cursor = 0;
            return Ok(());
        }
        let start = self.advertisement_cursor.min(local.len() - 1);
        let transitions = local
            .iter()
            .cycle()
            .skip(start)
            .take(ADVERTISEMENT_BATCH.min(local.len()))
            .map(|projection| WireTransition {
                command: state_command(&projection.state),
                event_json: projection.event_json.clone(),
            })
            .collect();
        if let Err(error) = self.broadcast(transitions).await {
            tracing_error(&error);
        }
        self.advertisement_cursor = (start + ADVERTISEMENT_BATCH).min(local.len()) % local.len();
        Ok(())
    }

    async fn publish_shutdown_tombstones(&mut self) -> Result<(), EphemeralRuntimeError> {
        let local = self
            .projections
            .values()
            .filter(|projection| {
                projection.state.origin_node_id == self.local_node_id && projection.state.active
            })
            .map(|projection| projection.state.clone())
            .collect::<Vec<_>>();
        for state in local {
            self.tombstone(
                state.kind,
                state.scope,
                state.subject,
                state.context,
                None,
                None,
            )
            .await?;
        }
        Ok(())
    }

    async fn tombstone(
        &mut self,
        kind: EphemeralKind,
        scope: String,
        subject: String,
        context: String,
        observed_at_ms: Option<u64>,
        transition_id: Option<String>,
    ) -> Result<EphemeralDecision, EphemeralRuntimeError> {
        let (observed_at_ms, transition_id) = match (observed_at_ms, transition_id) {
            (Some(observed_at_ms), Some(transition_id)) => (observed_at_ms, transition_id),
            (None, None) => {
                self.sequence = self
                    .sequence
                    .checked_add(1)
                    .ok_or(EphemeralRuntimeError::InvalidConfiguration)?;
                (
                    unix_ms()?,
                    format!("{}{:032x}", self.local_node_id, self.sequence),
                )
            }
            _ => return Err(EphemeralRuntimeError::InvalidConfiguration),
        };
        let command = EphemeralCommand {
            scope,
            kind,
            subject,
            context,
            value: String::new(),
            active: false,
            observed_at_ms,
            ttl_secs: TOMBSTONE_TTL_SECS,
            origin_node_id: self.local_node_id.clone(),
            transition_id,
        };
        self.apply_local(command, None).await
    }

    async fn broadcast(
        &self,
        transitions: Vec<WireTransition>,
    ) -> Result<(), EphemeralRuntimeError> {
        let frame = WireFrame { transitions };
        let encoded = serde_json::to_vec(&frame)
            .map_err(|error| EphemeralRuntimeError::InvalidFrame(error.to_string()))?;
        let mut payload = Vec::with_capacity(WIRE_PREFIX.len() + encoded.len());
        payload.extend_from_slice(WIRE_PREFIX);
        payload.extend_from_slice(&encoded);
        self.mesh.broadcast(payload).await?;
        Ok(())
    }
}

fn state_command(state: &EphemeralState) -> EphemeralCommand {
    EphemeralCommand {
        scope: state.scope.clone(),
        kind: state.kind,
        subject: state.subject.clone(),
        context: state.context.clone(),
        value: state.value.clone(),
        active: state.active,
        observed_at_ms: state.observed_at_ms,
        ttl_secs: (state.expires_at_ms - state.observed_at_ms) / 1_000,
        origin_node_id: state.origin_node_id.clone(),
        transition_id: state.transition_id.clone(),
    }
}

fn unix_ms() -> Result<u64, EphemeralRuntimeError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| EphemeralRuntimeError::InvalidClock)?
        .as_millis()
        .try_into()
        .map_err(|_| EphemeralRuntimeError::InvalidClock)
}

async fn call_policy(
    boundary: &BoundaryClient,
    request: EphemeralPolicyRequest,
    timeout: Duration,
) -> Result<EphemeralPolicyResult, EphemeralRuntimeError> {
    let result = boundary
        .call(
            BoundaryRequest::ephemeral_policy(request),
            CallContext::with_timeout(timeout),
        )
        .await?;
    let BoundaryResult::EphemeralPolicy(result) = result else {
        return Err(EphemeralRuntimeError::UnexpectedPolicyResult);
    };
    Ok(result)
}

fn tracing_error(error: &EphemeralRuntimeError) {
    tracing::warn!(%error, "ephemeral convergence operation failed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_advertisement_preserves_nim_owned_ttl() {
        let state = EphemeralState {
            scope: "community-a".to_owned(),
            kind: EphemeralKind::Presence,
            subject: "01".repeat(32),
            context: String::new(),
            value: "online".to_owned(),
            active: true,
            observed_at_ms: 1_000,
            expires_at_ms: 181_000,
            origin_node_id: "11".repeat(16),
            transition_id: "aa".repeat(32),
        };
        let command = state_command(&state);
        assert_eq!(command.ttl_secs, PRESENCE_TTL_SECS);
        assert_eq!(command.transition_id, state.transition_id);
    }
}
