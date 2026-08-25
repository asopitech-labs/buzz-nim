//! Supervised process boundary between Rust host adapters and the Nimino core.
//!
//! This crate owns framing, deadlines, cancellation, bounded queuing, and the
//! child-process lifecycle. It deliberately contains no product, database,
//! replication, or cluster-authority policy.

#![deny(missing_docs)]

mod codec;
mod contract;
mod error;
mod runtime;

pub use contract::{
    BoundaryFault, BoundaryRequest, BoundaryResponse, BoundaryResult, EchoPayload, ReadyPayload,
    RemoteErrorCode, RetryDisposition, MAX_FRAME_BYTES, MAX_INFLIGHT, PROTOCOL_NAME,
    PROTOCOL_VERSION, SCHEMA_HASH, WORKER_ROLE,
};
pub use error::{BoundaryError, HOST_ERROR_CODES};
pub use runtime::{BoundaryClient, BoundaryConfig, BoundaryRuntime, CallContext};
