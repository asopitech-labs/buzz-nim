//! Typed agent/persona facts and decisions exchanged with the Nimino core.

use serde::{Deserialize, Serialize};

/// Optional persona trigger fields before precedence resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonaTriggers {
    /// Whether direct mentions trigger the persona.
    pub mentions: Option<bool>,
    /// Case-sensitive keyword triggers.
    pub keywords: Option<Vec<String>>,
    /// Whether every message triggers the persona.
    pub all_messages: Option<bool>,
}

/// Normalized persona behavior supplied by the file codec adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PersonaBehavior {
    /// Requested model identifier.
    pub model: Option<String>,
    /// Requested model temperature.
    pub temperature: Option<f64>,
    /// Requested context ceiling.
    pub max_context_tokens: Option<u64>,
    /// Channel subscriptions; an empty list intentionally subscribes nowhere.
    pub subscribe: Option<Vec<String>>,
    /// Trigger configuration; a present object shallowly replaces defaults.
    pub triggers: Option<PersonaTriggers>,
    /// Whether thread replies are enabled.
    pub thread_replies: Option<bool>,
    /// Whether broadcast replies are enabled.
    pub broadcast_replies: Option<bool>,
}

/// Resolved trigger configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedPersonaTriggers {
    /// Effective mention behavior.
    pub mentions: bool,
    /// Effective keyword list.
    pub keywords: Vec<String>,
    /// Effective all-message behavior.
    pub all_messages: bool,
}

/// Effective persona behavior selected by Nimino.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedPersonaBehavior {
    /// Effective model identifier.
    pub model: Option<String>,
    /// Effective model temperature.
    pub temperature: Option<f64>,
    /// Effective context ceiling.
    pub max_context_tokens: Option<u64>,
    /// Effective subscriptions.
    pub subscribe: Option<Vec<String>>,
    /// Effective triggers.
    pub triggers: Option<ResolvedPersonaTriggers>,
    /// Effective thread reply behavior.
    pub thread_replies: bool,
    /// Effective broadcast reply behavior.
    pub broadcast_replies: bool,
}

/// Event facts extracted and verified by the Nostr adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentEventFacts {
    /// Event content.
    pub content: String,
    /// Verified event author.
    pub author: String,
    /// Nostr event kind.
    pub kind: u32,
    /// Host-scoped channel identifier.
    pub channel_id: String,
    /// Event timestamp.
    pub timestamp: u64,
    /// Whether a verified `p` tag mentions this agent.
    pub mentioned: bool,
}

/// One ordered subscription rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentTriggerRule {
    /// Stable rule name.
    pub name: String,
    /// Whether the rule covers every channel.
    pub all_channels: bool,
    /// Explicit channel allowlist when `all_channels` is false.
    pub channels: Vec<String>,
    /// Allowed event kinds; empty is a wildcard.
    pub kinds: Vec<u32>,
    /// Whether a verified mention is required.
    pub require_mention: bool,
    /// Nim condition expression; empty means no expression.
    pub filter: String,
    /// Prompt tag; empty falls back to the rule name.
    pub prompt_tag: String,
}

/// Agent process/session lifecycle phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    /// No process exists.
    Absent,
    /// A process start is in flight.
    Starting,
    /// The process is ready for a turn.
    Ready,
    /// A turn is active.
    Running,
    /// Cancellation was sent and completion is being drained.
    Cancelling,
    /// The adapter must wait before restarting.
    RestartWait,
    /// Explicit shutdown is terminal.
    Stopped,
}

/// Lifecycle command observed by the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLifecycleCommand {
    /// Start lazily when work exists.
    Start,
    /// Accept a successful matching start.
    Started,
    /// Record a failed matching start.
    StartFailed,
    /// Begin a turn.
    BeginTurn,
    /// Cancel the current turn.
    Cancel,
    /// Finish the current turn or cancellation drain.
    TurnFinished,
    /// Cancellation did not drain within its deadline.
    CancelTimeout,
    /// The supervised process exited.
    ProcessExited,
    /// Retry a due failed start.
    Retry,
    /// Stop permanently.
    Shutdown,
}

/// Side effect the Rust ACP/process adapter may execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLifecycleAction {
    /// Reject without an effect.
    Reject,
    /// No effect is required.
    Noop,
    /// Spawn and negotiate an ACP process.
    Spawn,
    /// Publish the matching process as ready.
    AcceptStart,
    /// Send a prompt for the returned turn.
    BeginTurn,
    /// Send ACP cancellation for the returned turn.
    SendCancel,
    /// Return the process to the ready pool.
    ReturnReady,
    /// Kill/reap the process and wait for the retry deadline.
    ReapAndWait,
    /// Stop and reap without restart.
    Stop,
}

/// Stable agent policy failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPolicyError {
    /// No error.
    None,
    /// A trigger rule is malformed.
    InvalidRule,
    /// A trigger filter is invalid or references unavailable facts.
    InvalidFilter,
    /// The lifecycle edge is not allowed.
    InvalidTransition,
    /// A start result belongs to an older attempt.
    StaleAttempt,
    /// A turn completion/cancellation targets another turn.
    InvalidTurn,
}

/// Complete lifecycle state returned after every decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentLifecycleState {
    /// Current phase.
    pub phase: AgentPhase,
    /// Current consecutive start attempt, or zero after a successful start.
    pub attempt: u64,
    /// Earliest restart time in Unix milliseconds.
    pub retry_at_ms: u64,
    /// Active turn identifier, empty outside running/cancelling.
    pub turn_id: String,
}

/// Facts for one lifecycle transition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentLifecycleRequest {
    /// Current durable/in-memory state.
    pub state: AgentLifecycleState,
    /// Requested transition.
    pub command: AgentLifecycleCommand,
    /// Attempt token carried by a start result.
    pub command_attempt: u64,
    /// Turn identifier carried by a turn command.
    pub command_turn_id: String,
    /// Whether work is waiting.
    pub pending_work: bool,
    /// Adapter clock in Unix milliseconds.
    pub now_ms: u64,
}

/// Typed agent/persona policy request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentPolicyRequest {
    /// Resolve persona behavior with persona-over-pack precedence.
    Persona {
        /// Per-persona behavior.
        persona: PersonaBehavior,
        /// Pack defaults.
        defaults: PersonaBehavior,
    },
    /// Match verified event facts against ordered subscription rules.
    Trigger {
        /// Verified event facts.
        event: AgentEventFacts,
        /// Ordered rules; first match wins.
        rules: Vec<AgentTriggerRule>,
    },
    /// Decide one cancel/restart lifecycle edge.
    Lifecycle {
        /// Lifecycle facts.
        request: AgentLifecycleRequest,
    },
}

/// Typed agent/persona policy result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentPolicyResult {
    /// Resolved persona behavior.
    Persona {
        /// Effective behavior.
        behavior: ResolvedPersonaBehavior,
    },
    /// Ordered trigger decision.
    Trigger {
        /// Whether a rule matched.
        matched: bool,
        /// Zero-based rule index, or -1 when unmatched/rejected.
        rule_index: i32,
        /// Effective prompt tag.
        prompt_tag: String,
        /// Stable trigger error.
        error: AgentPolicyError,
    },
    /// Lifecycle transition decision.
    Lifecycle {
        /// Whether the transition is accepted.
        allowed: bool,
        /// Stable lifecycle error.
        error: AgentPolicyError,
        /// Adapter effect.
        action: AgentLifecycleAction,
        /// Resulting state.
        next_state: AgentLifecycleState,
    },
}
