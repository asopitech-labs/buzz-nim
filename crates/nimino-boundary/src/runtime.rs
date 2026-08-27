use std::{ffi::OsString, path::PathBuf, process::Stdio, time::Duration};

use serde_json::Value;
use tokio::{
    process::{Child, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot},
    task::JoinHandle,
    time::Instant,
};
use tokio_util::sync::CancellationToken;

use crate::{
    codec::{read_json_frame, write_json_frame, CodecError},
    contract::{BoundaryResult, ReadyPayload, RemoteErrorCode, MAX_FRAME_BYTES, MAX_INFLIGHT},
    BoundaryError, BoundaryRequest, BoundaryResponse, PROTOCOL_NAME, PROTOCOL_VERSION, SCHEMA_HASH,
    WORKER_ROLE,
};

const DEFAULT_QUEUE_CAPACITY: usize = 64;
const MAX_QUEUE_CAPACITY: usize = 1_024;
const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(2);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);

/// Immutable process and queue configuration for the v1 boundary.
#[derive(Debug, Clone)]
pub struct BoundaryConfig {
    worker_program: PathBuf,
    worker_args: Vec<OsString>,
    queue_capacity: usize,
    startup_timeout: Duration,
}

impl BoundaryConfig {
    /// Creates a config for a worker executable with production v1 defaults.
    pub fn new(worker_program: impl Into<PathBuf>) -> Self {
        Self {
            worker_program: worker_program.into(),
            worker_args: Vec::new(),
            queue_capacity: DEFAULT_QUEUE_CAPACITY,
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
        }
    }

    /// Replaces the fixed argument vector passed to the worker executable.
    pub fn with_worker_args(mut self, worker_args: impl IntoIterator<Item = OsString>) -> Self {
        self.worker_args = worker_args.into_iter().collect();
        self
    }

    /// Sets the bounded waiting-call capacity. Zero is rejected by `start`.
    pub fn with_queue_capacity(mut self, queue_capacity: usize) -> Self {
        self.queue_capacity = queue_capacity;
        self
    }

    /// Sets the exact-match startup and handshake deadline.
    pub fn with_startup_timeout(mut self, startup_timeout: Duration) -> Self {
        self.startup_timeout = startup_timeout;
        self
    }

    fn validate(&self) -> Result<(), BoundaryError> {
        if self.queue_capacity == 0 {
            return Err(BoundaryError::InvalidConfiguration(
                "queue capacity must be greater than zero".to_owned(),
            ));
        }
        if self.queue_capacity > MAX_QUEUE_CAPACITY {
            return Err(BoundaryError::InvalidConfiguration(format!(
                "queue capacity must not exceed {MAX_QUEUE_CAPACITY}"
            )));
        }
        if self.startup_timeout.is_zero() {
            return Err(BoundaryError::InvalidConfiguration(
                "startup timeout must be greater than zero".to_owned(),
            ));
        }
        if Instant::now().checked_add(self.startup_timeout).is_none() {
            return Err(BoundaryError::InvalidConfiguration(
                "startup timeout exceeds the monotonic clock range".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Monotonic deadline and cancellation signal for one call.
#[derive(Debug, Clone)]
pub struct CallContext {
    timeout: Duration,
    cancellation: CancellationToken,
}

impl CallContext {
    /// Creates an independently cancellable call with a fixed timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            cancellation: CancellationToken::new(),
        }
    }

    /// Creates a call using a cancellation token owned by the caller.
    pub fn with_timeout_and_cancellation(
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Self {
        Self {
            timeout,
            cancellation,
        }
    }
}

/// Cloneable request facade backed by one supervised worker lifecycle owner.
#[derive(Debug, Clone)]
pub struct BoundaryClient {
    sender: mpsc::Sender<ManagerCommand>,
}

impl BoundaryClient {
    /// Enqueues one typed request or rejects immediately when the queue is full.
    pub async fn call(
        &self,
        request: BoundaryRequest,
        context: CallContext,
    ) -> Result<BoundaryResult, BoundaryError> {
        if context.cancellation.is_cancelled() {
            return Err(BoundaryError::Cancelled);
        }
        if context.timeout.is_zero() {
            return Err(BoundaryError::DeadlineExceeded);
        }
        let deadline = Instant::now().checked_add(context.timeout).ok_or_else(|| {
            BoundaryError::InvalidConfiguration(
                "call timeout exceeds the monotonic clock range".to_owned(),
            )
        })?;
        let cancellation = context.cancellation.clone();
        let abandonment = CancellationToken::new();
        let abandonment_guard = abandonment.clone().drop_guard();
        let (response_sender, response_receiver) = oneshot::channel();
        self.sender
            .try_send(ManagerCommand::Call {
                request,
                deadline,
                cancellation: cancellation.clone(),
                abandonment,
                response: response_sender,
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => BoundaryError::Backpressure,
                mpsc::error::TrySendError::Closed(_) => BoundaryError::Shutdown,
            })?;

        tokio::select! {
            biased;
            result = response_receiver => {
                abandonment_guard.disarm();
                result.unwrap_or(Err(BoundaryError::Shutdown))
            },
            _ = cancellation.cancelled() => Err(BoundaryError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => Err(BoundaryError::DeadlineExceeded),
        }
    }
}

/// Lifecycle owner that starts, drains, kills, and reaps the Nim worker.
#[derive(Debug)]
pub struct BoundaryRuntime {
    sender: mpsc::Sender<ManagerCommand>,
    shutdown: CancellationToken,
    manager: Option<JoinHandle<Result<(), BoundaryError>>>,
}

impl BoundaryRuntime {
    /// Starts a worker and accepts calls only after the exact v1 handshake.
    pub async fn start(config: BoundaryConfig) -> Result<Self, BoundaryError> {
        config.validate()?;
        let startup_deadline = Instant::now()
            .checked_add(config.startup_timeout)
            .ok_or_else(|| {
                BoundaryError::InvalidConfiguration(
                    "startup timeout exceeds the monotonic clock range".to_owned(),
                )
            })?;
        let worker = WorkerProcess::start(&config, startup_deadline).await?;
        let (sender, receiver) = mpsc::channel(config.queue_capacity);
        let shutdown = CancellationToken::new();
        let manager = tokio::spawn(run_manager(config, worker, receiver, shutdown.clone()));
        Ok(Self {
            sender,
            shutdown,
            manager: Some(manager),
        })
    }

    /// Returns a cloneable, bounded request facade.
    pub fn client(&self) -> BoundaryClient {
        BoundaryClient {
            sender: self.sender.clone(),
        }
    }

    /// Cancels queued and active calls, then kills and reaps the worker.
    pub async fn shutdown(mut self) -> Result<(), BoundaryError> {
        self.shutdown.cancel();
        if let Some(manager) = self.manager.take() {
            manager.await.map_err(|_| BoundaryError::Shutdown)??;
        }
        Ok(())
    }
}

impl Drop for BoundaryRuntime {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

#[derive(Debug)]
enum ManagerCommand {
    Call {
        request: BoundaryRequest,
        deadline: Instant,
        cancellation: CancellationToken,
        abandonment: CancellationToken,
        response: oneshot::Sender<Result<BoundaryResult, BoundaryError>>,
    },
}

async fn run_manager(
    config: BoundaryConfig,
    initial_worker: WorkerProcess,
    mut receiver: mpsc::Receiver<ManagerCommand>,
    shutdown: CancellationToken,
) -> Result<(), BoundaryError> {
    let mut worker = Some(initial_worker);
    loop {
        let command = tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                recycle_worker(&mut worker).await?;
                return Ok(());
            }
            command = receiver.recv() => command,
        };
        let Some(command) = command else {
            recycle_worker(&mut worker).await?;
            return Ok(());
        };
        match command {
            ManagerCommand::Call {
                request,
                deadline,
                cancellation,
                abandonment,
                response,
            } => {
                let result = execute_call(
                    &config,
                    &mut worker,
                    request,
                    deadline,
                    cancellation,
                    abandonment,
                    shutdown.clone(),
                )
                .await;
                let terminal = result
                    .as_ref()
                    .err()
                    .filter(|error| matches!(error, BoundaryError::CleanupFailed(_)))
                    .cloned();
                let _ = response.send(result);
                if let Some(error) = terminal {
                    return Err(error);
                }
            }
        }
    }
}

enum CallOutcome {
    Response(Result<BoundaryResponse, CodecError>),
    DeadlineExceeded,
    Cancelled,
    Shutdown,
}

async fn execute_call(
    config: &BoundaryConfig,
    worker: &mut Option<WorkerProcess>,
    request: BoundaryRequest,
    deadline: Instant,
    cancellation: CancellationToken,
    abandonment: CancellationToken,
    shutdown: CancellationToken,
) -> Result<BoundaryResult, BoundaryError> {
    if shutdown.is_cancelled() {
        return Err(BoundaryError::Shutdown);
    }
    if cancellation.is_cancelled() || abandonment.is_cancelled() {
        return Err(BoundaryError::Cancelled);
    }
    if Instant::now() >= deadline {
        return Err(BoundaryError::DeadlineExceeded);
    }
    if worker.is_none() {
        let startup_limit = Instant::now()
            .checked_add(config.startup_timeout)
            .unwrap_or(deadline);
        match WorkerProcess::start(config, deadline.min(startup_limit)).await {
            Ok(started) => *worker = Some(started),
            Err(BoundaryError::StartupTimeout) if Instant::now() >= deadline => {
                return Err(BoundaryError::DeadlineExceeded);
            }
            Err(error) => return Err(error),
        }
    }

    let Some(active) = worker.as_mut() else {
        return Err(BoundaryError::ProtocolViolation(
            "worker lifecycle did not enter ready state".to_owned(),
        ));
    };
    let request_id = request.request_id().to_owned();
    let operation = request.operation_name().to_owned();
    let outcome = {
        let exchange = active.exchange(&request);
        tokio::pin!(exchange);
        tokio::select! {
            biased;
            response = &mut exchange => CallOutcome::Response(response),
            _ = cancellation.cancelled() => CallOutcome::Cancelled,
            _ = abandonment.cancelled() => CallOutcome::Cancelled,
            _ = shutdown.cancelled() => CallOutcome::Shutdown,
            _ = tokio::time::sleep_until(deadline) => CallOutcome::DeadlineExceeded,
        }
    };

    match outcome {
        CallOutcome::Cancelled => {
            recycle_worker(worker).await?;
            Err(BoundaryError::Cancelled)
        }
        CallOutcome::DeadlineExceeded => {
            recycle_worker(worker).await?;
            Err(BoundaryError::DeadlineExceeded)
        }
        CallOutcome::Shutdown => {
            recycle_worker(worker).await?;
            Err(BoundaryError::Shutdown)
        }
        CallOutcome::Response(Ok(response)) => {
            if response.protocol() != PROTOCOL_NAME
                || response.version() != PROTOCOL_VERSION
                || response.request_id() != request_id
                || response.operation() != operation
            {
                recycle_worker(worker).await?;
                return Err(BoundaryError::ProtocolViolation(
                    "response metadata did not match the active request".to_owned(),
                ));
            }
            response.into_result().map_err(BoundaryError::Remote)
        }
        CallOutcome::Response(Err(error)) => {
            let status = worker_status_after_failure(worker).await;
            recycle_worker(worker).await?;
            if let Some(status) = status {
                Err(BoundaryError::WorkerExited { status })
            } else {
                Err(BoundaryError::ProtocolViolation(error.to_string()))
            }
        }
    }
}

async fn worker_status_after_failure(worker: &mut Option<WorkerProcess>) -> Option<Option<i32>> {
    let active = worker.as_mut()?;
    if let Ok(Some(status)) = active.child.try_wait() {
        return Some(status.code());
    }
    match tokio::time::timeout(Duration::from_millis(100), active.child.wait()).await {
        Ok(Ok(status)) => Some(status.code()),
        _ => None,
    }
}

async fn recycle_worker(worker: &mut Option<WorkerProcess>) -> Result<(), BoundaryError> {
    if let Some(mut active) = worker.take() {
        active.kill_and_reap().await?;
    }
    Ok(())
}

struct WorkerProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl WorkerProcess {
    async fn start(config: &BoundaryConfig, deadline: Instant) -> Result<Self, BoundaryError> {
        let mut command = Command::new(&config.worker_program);
        command
            .args(&config.worker_args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|error| BoundaryError::SpawnFailed(error.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| BoundaryError::SpawnFailed("worker stdin was not piped".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BoundaryError::SpawnFailed("worker stdout was not piped".to_owned()))?;
        let mut worker = Self {
            child,
            stdin,
            stdout,
        };
        let hello = BoundaryRequest::hello();
        let request_id = hello.request_id().to_owned();
        let response = match tokio::time::timeout_at(deadline, worker.exchange_value(&hello)).await
        {
            Ok(Ok(response)) => match serde_json::from_value::<BoundaryResponse>(response) {
                Ok(response) => response,
                Err(_) => {
                    return Err(worker
                        .fail_after_cleanup(BoundaryError::ContractMismatch)
                        .await)
                }
            },
            Ok(Err(error)) => {
                return Err(worker
                    .fail_after_cleanup(BoundaryError::ProtocolViolation(error.to_string()))
                    .await);
            }
            Err(_) => {
                return Err(worker
                    .fail_after_cleanup(BoundaryError::StartupTimeout)
                    .await);
            }
        };
        if response.protocol() != PROTOCOL_NAME
            || response.version() != PROTOCOL_VERSION
            || response.request_id() != request_id
            || response.operation() != "system.hello"
        {
            return Err(worker
                .fail_after_cleanup(BoundaryError::ContractMismatch)
                .await);
        }
        let ready = match response.into_result() {
            Ok(BoundaryResult::Ready(ready)) => ready,
            Ok(_) => {
                return Err(worker
                    .fail_after_cleanup(BoundaryError::ContractMismatch)
                    .await);
            }
            Err(fault) if fault.code == RemoteErrorCode::ContractMismatch => {
                return Err(worker
                    .fail_after_cleanup(BoundaryError::ContractMismatch)
                    .await);
            }
            Err(fault) => {
                return Err(worker
                    .fail_after_cleanup(BoundaryError::Remote(fault))
                    .await);
            }
        };
        if !ready_matches_contract(&ready) {
            return Err(worker
                .fail_after_cleanup(BoundaryError::ContractMismatch)
                .await);
        }
        Ok(worker)
    }

    async fn exchange(
        &mut self,
        request: &BoundaryRequest,
    ) -> Result<BoundaryResponse, CodecError> {
        write_json_frame(&mut self.stdin, request, MAX_FRAME_BYTES).await?;
        read_json_frame(&mut self.stdout, MAX_FRAME_BYTES).await
    }

    async fn exchange_value(&mut self, request: &BoundaryRequest) -> Result<Value, CodecError> {
        write_json_frame(&mut self.stdin, request, MAX_FRAME_BYTES).await?;
        read_json_frame(&mut self.stdout, MAX_FRAME_BYTES).await
    }

    async fn fail_after_cleanup(&mut self, primary: BoundaryError) -> BoundaryError {
        match self.kill_and_reap().await {
            Ok(()) => primary,
            Err(cleanup) => cleanup,
        }
    }

    async fn kill_and_reap(&mut self) -> Result<(), BoundaryError> {
        match self.child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) => {}
            Err(error) => return Err(BoundaryError::CleanupFailed(error.to_string())),
        }
        if let Err(kill_error) = self.child.start_kill() {
            match self.child.try_wait() {
                Ok(Some(_)) => return Ok(()),
                _ => return Err(BoundaryError::CleanupFailed(kill_error.to_string())),
            }
        }
        match tokio::time::timeout(CLEANUP_TIMEOUT, self.child.wait()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(error)) => Err(BoundaryError::CleanupFailed(error.to_string())),
            Err(_) => Err(BoundaryError::CleanupFailed(
                "worker reap timed out".to_owned(),
            )),
        }
    }
}

fn ready_matches_contract(ready: &ReadyPayload) -> bool {
    ready.protocol_version == PROTOCOL_VERSION
        && ready.schema_hash == SCHEMA_HASH
        && ready.worker_role == WORKER_ROLE
        && ready.max_frame_bytes == MAX_FRAME_BYTES
        && ready.max_inflight == MAX_INFLIGHT
        && ready
            .capabilities
            .iter()
            .any(|name| name == "boundary.echo")
        && ready
            .capabilities
            .iter()
            .any(|name| name == "domain.event.policy")
        && ready
            .capabilities
            .iter()
            .any(|name| name == "domain.community.policy")
        && !ready.worker_version.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> ReadyPayload {
        ReadyPayload {
            worker_version: "0.1.0".to_owned(),
            protocol_version: PROTOCOL_VERSION,
            schema_hash: SCHEMA_HASH.to_owned(),
            worker_role: WORKER_ROLE.to_owned(),
            max_frame_bytes: MAX_FRAME_BYTES,
            max_inflight: MAX_INFLIGHT,
            capabilities: vec![
                "boundary.echo".to_owned(),
                "domain.event.policy".to_owned(),
                "domain.community.policy".to_owned(),
            ],
        }
    }

    #[test]
    fn handshake_requires_every_exact_contract_fact() {
        assert!(ready_matches_contract(&ready()));

        let mut candidate = ready();
        candidate.protocol_version += 1;
        assert!(!ready_matches_contract(&candidate));
        let mut candidate = ready();
        candidate.schema_hash = "0".repeat(64);
        assert!(!ready_matches_contract(&candidate));
        let mut candidate = ready();
        candidate.worker_role = "wrong".to_owned();
        assert!(!ready_matches_contract(&candidate));
        let mut candidate = ready();
        candidate.max_frame_bytes -= 1;
        assert!(!ready_matches_contract(&candidate));
        let mut candidate = ready();
        candidate.max_inflight += 1;
        assert!(!ready_matches_contract(&candidate));
        let mut candidate = ready();
        candidate.capabilities.clear();
        assert!(!ready_matches_contract(&candidate));
        let mut candidate = ready();
        candidate
            .capabilities
            .retain(|name| name != "domain.event.policy");
        assert!(!ready_matches_contract(&candidate));
        let mut candidate = ready();
        candidate
            .capabilities
            .retain(|name| name != "domain.community.policy");
        assert!(!ready_matches_contract(&candidate));
        let mut candidate = ready();
        candidate.worker_version.clear();
        assert!(!ready_matches_contract(&candidate));
    }

    #[test]
    fn queue_capacity_has_a_fixed_safe_upper_bound() {
        assert!(BoundaryConfig::new("worker")
            .with_queue_capacity(MAX_QUEUE_CAPACITY + 1)
            .validate()
            .is_err());
    }
}
