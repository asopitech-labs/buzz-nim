use crate::BoundaryFault;

/// Complete v1 host-side error-code inventory in manifest order.
pub const HOST_ERROR_CODES: &[&str] = &[
    "INVALID_CONFIGURATION",
    "SPAWN_FAILED",
    "STARTUP_TIMEOUT",
    "CONTRACT_MISMATCH",
    "PROTOCOL_VIOLATION",
    "WORKER_EXITED",
    "DEADLINE_EXCEEDED",
    "CANCELLED",
    "BACKPRESSURE",
    "CLEANUP_FAILED",
    "SHUTDOWN",
];

/// Typed host-side and remote errors produced by the supervised boundary.
#[derive(Debug, Clone, thiserror::Error)]
pub enum BoundaryError {
    /// Runtime configuration violates a fixed v1 invariant.
    #[error("invalid boundary configuration: {0}")]
    InvalidConfiguration(String),
    /// The Nim worker process could not be created.
    #[error("failed to spawn Nimino core worker: {0}")]
    SpawnFailed(String),
    /// The worker did not complete its exact-match handshake in time.
    #[error("Nimino core worker startup timed out")]
    StartupTimeout,
    /// The worker and host do not carry the same immutable v1 contract.
    #[error("Nimino core worker contract does not match the host")]
    ContractMismatch,
    /// A frame or response violated the versioned boundary contract.
    #[error("Nimino core protocol violation: {0}")]
    ProtocolViolation(String),
    /// The worker exited before returning the active response.
    #[error("Nimino core worker exited with status {status:?}")]
    WorkerExited {
        /// Platform exit code, when the process supplied one.
        status: Option<i32>,
    },
    /// The call did not finish within the caller's monotonic budget.
    #[error("Nimino core call exceeded its deadline")]
    DeadlineExceeded,
    /// The call was cancelled before a response was committed.
    #[error("Nimino core call was cancelled")]
    Cancelled,
    /// The bounded host queue was full; nothing was silently dropped.
    #[error("Nimino core boundary queue is full")]
    Backpressure,
    /// The previous worker could not be proven reaped, so replacement is forbidden.
    #[error("Nimino core worker cleanup failed: {0}")]
    CleanupFailed(String),
    /// The Nim worker returned a stable typed failure.
    #[error("Nimino core rejected the call: {0:?}")]
    Remote(BoundaryFault),
    /// The lifecycle owner is shutting down or has already stopped.
    #[error("Nimino core boundary is shut down")]
    Shutdown,
}

impl BoundaryError {
    /// Returns the stable code recorded by the versioned error manifest.
    pub fn code(&self) -> &str {
        match self {
            Self::InvalidConfiguration(_) => "INVALID_CONFIGURATION",
            Self::SpawnFailed(_) => "SPAWN_FAILED",
            Self::StartupTimeout => "STARTUP_TIMEOUT",
            Self::ContractMismatch => "CONTRACT_MISMATCH",
            Self::ProtocolViolation(_) => "PROTOCOL_VIOLATION",
            Self::WorkerExited { .. } => "WORKER_EXITED",
            Self::DeadlineExceeded => "DEADLINE_EXCEEDED",
            Self::Cancelled => "CANCELLED",
            Self::Backpressure => "BACKPRESSURE",
            Self::CleanupFailed(_) => "CLEANUP_FAILED",
            Self::Remote(fault) => fault.code.as_str(),
            Self::Shutdown => "SHUTDOWN",
        }
    }
}
