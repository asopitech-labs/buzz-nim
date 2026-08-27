use std::fmt;
use std::thread;

use tokio::sync::{broadcast, mpsc, oneshot, watch};

use crate::{upstream, NodeConfig, NodeConfigError, NodeId};

const DEFAULT_COMMAND_CAPACITY: usize = 64;
const DEFAULT_EVENT_CAPACITY: usize = 256;
const MAX_CAPACITY: usize = 4096;
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;

/// Bounded queues for the Chirps runtime adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MeshRuntimeOptions {
    command_capacity: usize,
    event_capacity: usize,
}

impl MeshRuntimeOptions {
    /// Creates queue limits validated by [`MeshRuntime::start`].
    pub fn new(command_capacity: usize, event_capacity: usize) -> Self {
        Self {
            command_capacity,
            event_capacity,
        }
    }
}

impl Default for MeshRuntimeOptions {
    fn default() -> Self {
        Self::new(DEFAULT_COMMAND_CAPACITY, DEFAULT_EVENT_CAPACITY)
    }
}

/// One secure user message received from a Chirps peer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MeshMessage {
    from: NodeId,
    payload: Vec<u8>,
}

impl MeshMessage {
    pub(crate) fn new(from: NodeId, payload: Vec<u8>) -> Self {
        Self { from, payload }
    }

    /// Returns the authenticated transport peer identity.
    pub fn from(&self) -> NodeId {
        self.from
    }

    /// Returns the opaque payload; interpretation belongs to Nimino protocols.
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Consumes the message and returns its opaque payload.
    pub fn into_payload(self) -> Vec<u8> {
        self.payload
    }
}

/// Typed lifecycle, capacity, and transport failure.
#[derive(Debug)]
pub enum MeshRuntimeError {
    /// Runtime queue capacity is zero or exceeds the fixed safety ceiling.
    InvalidCapacity {
        /// Rejected capacity.
        capacity: usize,
    },
    /// A message exceeds the fixed one-mebibyte adapter ceiling.
    MessageTooLarge {
        /// Payload bytes.
        size: usize,
        /// Accepted maximum.
        max: usize,
    },
    /// The bounded command queue is full.
    Backpressure,
    /// A slow subscriber missed one or more messages.
    SubscriberLagged {
        /// Number of overwritten messages.
        skipped: u64,
    },
    /// Node identity or production mTLS preparation failed.
    Config(NodeConfigError),
    /// Chirps could not start or execute a transport operation.
    Transport(String),
    /// The dedicated runtime thread could not be created.
    ThreadStart(String),
    /// The runtime has stopped and accepts no more work.
    Stopped,
    /// The dedicated runtime thread panicked.
    WorkerPanicked,
}

impl fmt::Display for MeshRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCapacity { capacity } => {
                write!(formatter, "invalid mesh queue capacity: {capacity}")
            }
            Self::MessageTooLarge { size, max } => {
                write!(formatter, "mesh message exceeds limit ({size} > {max})")
            }
            Self::Backpressure => write!(formatter, "mesh command queue is full"),
            Self::SubscriberLagged { skipped } => {
                write!(formatter, "mesh subscriber missed {skipped} messages")
            }
            Self::Config(error) => write!(formatter, "mesh configuration failed: {error}"),
            Self::Transport(reason) => write!(formatter, "mesh transport failed: {reason}"),
            Self::ThreadStart(reason) => write!(formatter, "mesh thread start failed: {reason}"),
            Self::Stopped => write!(formatter, "mesh runtime is stopped"),
            Self::WorkerPanicked => write!(formatter, "mesh runtime thread panicked"),
        }
    }
}

impl std::error::Error for MeshRuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            _ => None,
        }
    }
}

impl From<NodeConfigError> for MeshRuntimeError {
    fn from(error: NodeConfigError) -> Self {
        Self::Config(error)
    }
}

pub(crate) enum RuntimeCommand {
    Send {
        target: NodeId,
        payload: Vec<u8>,
        reply: oneshot::Sender<Result<(), MeshRuntimeError>>,
    },
    Broadcast {
        payload: Vec<u8>,
        reply: oneshot::Sender<Result<usize, MeshRuntimeError>>,
    },
    Peers {
        reply: oneshot::Sender<Result<Vec<NodeId>, MeshRuntimeError>>,
    },
}

pub(crate) struct RuntimeWorker {
    pub(crate) commands: mpsc::Receiver<RuntimeCommand>,
    pub(crate) events: broadcast::Sender<MeshMessage>,
    pub(crate) shutdown: oneshot::Receiver<()>,
    pub(crate) stopped: watch::Sender<bool>,
    pub(crate) startup: oneshot::Sender<Result<NodeId, MeshRuntimeError>>,
}

/// Cloneable command and subscription handle for a running mesh.
#[derive(Clone)]
pub struct MeshClient {
    local_node_id: NodeId,
    commands: mpsc::Sender<RuntimeCommand>,
    events: broadcast::Sender<MeshMessage>,
    stopped: watch::Receiver<bool>,
}

impl MeshClient {
    /// Returns this runtime's stable node identity.
    pub fn local_node_id(&self) -> NodeId {
        self.local_node_id
    }

    /// Sends one opaque payload to a transport peer.
    pub async fn send(&self, target: NodeId, payload: Vec<u8>) -> Result<(), MeshRuntimeError> {
        validate_message_size(payload.len())?;
        self.request(|reply| RuntimeCommand::Send {
            target,
            payload,
            reply,
        })
        .await
    }

    /// Broadcasts one opaque payload and returns the Chirps accepted-peer count.
    pub async fn broadcast(&self, payload: Vec<u8>) -> Result<usize, MeshRuntimeError> {
        validate_message_size(payload.len())?;
        self.request(|reply| RuntimeCommand::Broadcast { payload, reply })
            .await
    }

    /// Returns current Chirps reachability hints, never ownership authority.
    pub async fn peers(&self) -> Result<Vec<NodeId>, MeshRuntimeError> {
        self.request(|reply| RuntimeCommand::Peers { reply }).await
    }

    /// Subscribes to opaque user messages with the configured bounded capacity.
    pub fn subscribe(&self) -> MeshSubscription {
        MeshSubscription {
            receiver: self.events.subscribe(),
            stopped: self.stopped.clone(),
        }
    }

    async fn request<T>(
        &self,
        make_command: impl FnOnce(oneshot::Sender<Result<T, MeshRuntimeError>>) -> RuntimeCommand,
    ) -> Result<T, MeshRuntimeError> {
        if *self.stopped.borrow() {
            return Err(MeshRuntimeError::Stopped);
        }
        let (reply, response) = oneshot::channel();
        match self.commands.try_send(make_command(reply)) {
            Ok(()) => response.await.unwrap_or(Err(MeshRuntimeError::Stopped)),
            Err(mpsc::error::TrySendError::Full(_)) => Err(MeshRuntimeError::Backpressure),
            Err(mpsc::error::TrySendError::Closed(_)) => Err(MeshRuntimeError::Stopped),
        }
    }
}

/// Bounded subscription to inbound Chirps user messages.
pub struct MeshSubscription {
    receiver: broadcast::Receiver<MeshMessage>,
    stopped: watch::Receiver<bool>,
}

impl MeshSubscription {
    /// Receives the next message or reports lag/shutdown explicitly.
    pub async fn recv(&mut self) -> Result<MeshMessage, MeshRuntimeError> {
        if *self.stopped.borrow() {
            return Err(MeshRuntimeError::Stopped);
        }
        tokio::select! {
            biased;
            stopped = self.stopped.changed() => {
                let _ = stopped;
                Err(MeshRuntimeError::Stopped)
            }
            message = self.receiver.recv() => match message {
                Ok(message) => Ok(message),
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    Err(MeshRuntimeError::SubscriberLagged { skipped })
                }
                Err(broadcast::error::RecvError::Closed) => Err(MeshRuntimeError::Stopped),
            }
        }
    }
}

/// Owner of one dedicated Chirps runtime thread.
pub struct MeshRuntime {
    client: MeshClient,
    shutdown: Option<oneshot::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl MeshRuntime {
    /// Starts a validated Chirps node on its isolated Tokio runtime thread.
    pub async fn start(
        config: NodeConfig,
        options: MeshRuntimeOptions,
    ) -> Result<Self, MeshRuntimeError> {
        validate_capacity(options.command_capacity)?;
        validate_capacity(options.event_capacity)?;
        let (commands, command_receiver) = mpsc::channel(options.command_capacity);
        let (events, _) = broadcast::channel(options.event_capacity);
        let (shutdown, shutdown_receiver) = oneshot::channel();
        let (stopped_sender, stopped) = watch::channel(false);
        let (startup, started) = oneshot::channel();
        let worker = upstream::spawn_runtime(
            config,
            options.command_capacity,
            RuntimeWorker {
                commands: command_receiver,
                events: events.clone(),
                shutdown: shutdown_receiver,
                stopped: stopped_sender,
                startup,
            },
        )
        .map_err(|error| MeshRuntimeError::ThreadStart(error.to_string()))?;
        let local_node_id = match started.await {
            Ok(Ok(node_id)) => node_id,
            Ok(Err(error)) => {
                join_thread(worker).await?;
                return Err(error);
            }
            Err(_) => {
                join_thread(worker).await?;
                return Err(MeshRuntimeError::Stopped);
            }
        };
        Ok(Self {
            client: MeshClient {
                local_node_id,
                commands,
                events,
                stopped,
            },
            shutdown: Some(shutdown),
            worker: Some(worker),
        })
    }

    /// Returns this runtime's stable node identity.
    pub fn local_node_id(&self) -> NodeId {
        self.client.local_node_id()
    }

    /// Returns a cloneable command/subscription handle.
    pub fn client(&self) -> MeshClient {
        self.client.clone()
    }

    /// Stops the isolated runtime and waits until all tasks and sockets close.
    pub async fn stop(mut self) -> Result<(), MeshRuntimeError> {
        self.signal_shutdown();
        if let Some(worker) = self.worker.take() {
            join_thread(worker).await?;
        }
        Ok(())
    }

    fn signal_shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }
}

impl Drop for MeshRuntime {
    fn drop(&mut self) {
        self.signal_shutdown();
    }
}

fn validate_capacity(capacity: usize) -> Result<(), MeshRuntimeError> {
    if capacity == 0 || capacity > MAX_CAPACITY {
        return Err(MeshRuntimeError::InvalidCapacity { capacity });
    }
    Ok(())
}

fn validate_message_size(size: usize) -> Result<(), MeshRuntimeError> {
    if size > MAX_MESSAGE_BYTES {
        return Err(MeshRuntimeError::MessageTooLarge {
            size,
            max: MAX_MESSAGE_BYTES,
        });
    }
    Ok(())
}

async fn join_thread(worker: thread::JoinHandle<()>) -> Result<(), MeshRuntimeError> {
    tokio::task::spawn_blocking(move || worker.join())
        .await
        .map_err(|_| MeshRuntimeError::WorkerPanicked)?
        .map_err(|_| MeshRuntimeError::WorkerPanicked)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn full_command_queue_fails_without_waiting() {
        let (commands, receiver) = mpsc::channel(1);
        let (events, _) = broadcast::channel(1);
        let (_stopped_sender, stopped) = watch::channel(false);
        let client = MeshClient {
            local_node_id: NodeId::from_bytes([0; 16]),
            commands,
            events,
            stopped,
        };
        let (reply, _response) = oneshot::channel();
        client
            .commands
            .try_send(RuntimeCommand::Peers { reply })
            .expect("fill command queue");
        assert!(matches!(
            client.peers().await,
            Err(MeshRuntimeError::Backpressure)
        ));
        drop(receiver);
    }

    #[test]
    fn capacities_and_message_size_are_bounded() {
        for capacity in [0, MAX_CAPACITY + 1] {
            assert!(matches!(
                validate_capacity(capacity),
                Err(MeshRuntimeError::InvalidCapacity { capacity: rejected })
                    if rejected == capacity
            ));
        }
        assert!(validate_capacity(1).is_ok());
        assert!(validate_capacity(MAX_CAPACITY).is_ok());
        assert!(validate_message_size(MAX_MESSAGE_BYTES).is_ok());
        assert!(matches!(
            validate_message_size(MAX_MESSAGE_BYTES + 1),
            Err(MeshRuntimeError::MessageTooLarge { size, max })
                if size == MAX_MESSAGE_BYTES + 1 && max == MAX_MESSAGE_BYTES
        ));
    }
}
