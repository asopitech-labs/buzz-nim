//! Typed CLI command and failure policy exchanged with the Nimino core.

use serde::{Deserialize, Serialize};

/// Stable CLI failure class supplied by an I/O adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliFailureKind {
    /// Command syntax or input validation failed.
    Usage,
    /// The relay returned an HTTP error status.
    Relay,
    /// Transport, DNS, timeout, or response decoding failed.
    Network,
    /// Authentication material is missing or rejected.
    Auth,
    /// The supplied signing key is invalid.
    Key,
    /// A versioned write lost a compare-and-set race.
    Conflict,
    /// The requested entity does not exist.
    NotFound,
    /// A non-idempotent delivery may have committed without a response.
    DeliveryUnknown,
    /// An unclassified adapter failure occurred.
    Other,
}

/// Input to the Nim-owned CLI policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CliPolicyRequest {
    /// Resolve one canonical leaf command path.
    Command {
        /// Dot-separated command path, such as `workflows.create`.
        path: String,
    },
    /// Classify one adapter failure for JSON output and process exit.
    Failure {
        /// Stable adapter failure class.
        kind: CliFailureKind,
        /// HTTP status for relay failures, otherwise zero.
        status: u16,
        /// Whether the transport adapter proved this network failure retryable.
        transport_retryable: bool,
    },
}

/// Command grammar rejection returned by Nimino.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliCommandError {
    /// The command is in the v1 grammar.
    None,
    /// The command path is not part of v1.
    UnknownCommand,
}

/// External I/O class selected for a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliIoMode {
    /// No relay or other remote service is required.
    Local,
    /// Read-only relay access.
    RelayRead,
    /// A relay mutation may occur.
    RelayWrite,
}

/// Nim domain operation that owns any product decision behind a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CliPolicyOperation {
    /// The command is adapter-only.
    #[serde(rename = "none")]
    None,
    /// Event and addressable-record policy.
    #[serde(rename = "domain.event.policy")]
    Event,
    /// Community lifecycle policy.
    #[serde(rename = "domain.community.policy")]
    Community,
    /// Membership and ownership policy.
    #[serde(rename = "domain.membership.policy")]
    Membership,
    /// Direct-message policy.
    #[serde(rename = "domain.dm.policy")]
    Dm,
    /// Workflow policy.
    #[serde(rename = "domain.workflow.policy")]
    Workflow,
    /// Moderation policy.
    #[serde(rename = "domain.moderation.policy")]
    Moderation,
}

/// Stable JSON error category written by the CLI adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CliFailureCategory {
    /// Invalid user input.
    UserError,
    /// Relay-side failure.
    RelayError,
    /// Network-side failure.
    NetworkError,
    /// Missing or rejected authentication.
    AuthError,
    /// Invalid key material.
    KeyError,
    /// Write conflict.
    Conflict,
    /// Missing entity.
    NotFound,
    /// Non-idempotent delivery has an unknown outcome.
    DeliveryUnknown,
    /// Other failure.
    Error,
}

/// Typed result of a Nim-owned CLI policy decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum CliPolicyResult {
    /// Canonical command plan.
    Command {
        /// Whether the command is present in v1.
        accepted: bool,
        /// Stable grammar result.
        error: CliCommandError,
        /// Selected external I/O class.
        io_mode: CliIoMode,
        /// Whether the adapter must require the signing identity.
        requires_auth: bool,
        /// Versioned stdout contract.
        output_contract: String,
        /// Nim domain operation that owns product decisions.
        policy_operation: CliPolicyOperation,
    },
    /// JSON error and process-exit plan.
    Failure {
        /// Machine-readable JSON error category.
        category: CliFailureCategory,
        /// Process exit status.
        exit_code: u8,
        /// Whether the exact failed operation may be retried.
        retryable: bool,
    },
}
