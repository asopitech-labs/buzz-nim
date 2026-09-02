//! Typed workflow-policy facts, effect descriptors, and decisions.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Trigger recognized by the workflow domain engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTriggerKind {
    /// A channel message was posted.
    MessagePosted,
    /// A reaction was added.
    ReactionAdded,
    /// A diff event was posted.
    DiffPosted,
    /// A schedule fired.
    Schedule,
    /// A workflow webhook was invoked.
    Webhook,
}

/// Side effect selected by Nimino and executed by a Rust port.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowAction {
    /// Action discriminator.
    pub kind: WorkflowActionKind,
    /// Message or DM body template.
    pub text: String,
    /// Optional channel override, represented as an empty string when absent.
    pub channel: String,
    /// Whether a message replies to the trigger thread.
    pub reply_in_thread: bool,
    /// DM recipient template.
    pub recipient: String,
    /// Channel topic template.
    pub topic: String,
    /// Reaction emoji template.
    pub emoji: String,
    /// Webhook URL template.
    pub url: String,
    /// Webhook method; empty means POST.
    pub http_method: String,
    /// Webhook headers.
    pub headers: BTreeMap<String, String>,
    /// Webhook body template.
    pub body: String,
    /// Approval target template.
    pub approver: String,
    /// Approval prompt template.
    pub message: String,
    /// Approval timeout.
    pub timeout: String,
    /// Delay duration.
    pub duration: String,
}

/// Workflow action kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowActionKind {
    /// Publish a channel message.
    SendMessage,
    /// Publish a direct message.
    SendDm,
    /// Change a channel topic.
    SetChannelTopic,
    /// Add a reaction.
    AddReaction,
    /// Call an outbound webhook.
    CallWebhook,
    /// Suspend for approval.
    RequestApproval,
    /// Delay execution.
    Delay,
}

/// Normalized trigger definition supplied by the YAML/JSON codec adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowTrigger {
    /// Trigger discriminator.
    pub kind: WorkflowTriggerKind,
    /// Optional trigger condition, empty when absent.
    pub filter: String,
    /// Optional reaction emoji, empty when absent.
    pub emoji: String,
    /// Optional cron expression, empty when absent.
    pub cron: String,
    /// Optional interval, empty when absent.
    pub interval: String,
}

/// One normalized workflow step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowStep {
    /// Unique step identifier.
    pub id: String,
    /// Optional display name, empty when absent.
    pub name: String,
    /// Step condition, empty when absent.
    pub condition: String,
    /// Step timeout seconds; zero selects the adapter default.
    pub timeout_secs: u64,
    /// Action descriptor owned by the definition.
    pub action: WorkflowAction,
}

/// Normalized workflow definition evaluated by Nimino.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowDefinition {
    /// Human-readable workflow name.
    pub name: String,
    /// Optional description, empty when absent.
    pub description: String,
    /// Trigger definition.
    pub trigger: WorkflowTrigger,
    /// Ordered steps.
    pub steps: Vec<WorkflowStep>,
    /// Whether trigger adapters may run this definition.
    pub enabled: bool,
}

/// Durable run status presented to Nimino.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunStatus {
    /// Created but not started.
    Pending,
    /// Actively planning or executing.
    Running,
    /// Waiting for an approval effect result.
    WaitingApproval,
    /// Successfully completed.
    Completed,
    /// Failed.
    Failed,
    /// Cancelled.
    Cancelled,
}

/// Versioned workflow run state used by the persistence CAS port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowRunState {
    /// Current lifecycle status.
    pub status: WorkflowRunStatus,
    /// Zero-based next or suspended step index.
    pub current_step: u32,
    /// Monotonic CAS revision.
    pub revision: u64,
}

/// Inputs required to plan the current step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowPlanRequest {
    /// Definition loaded for this run.
    pub definition: WorkflowDefinition,
    /// Current durable state.
    pub state: WorkflowRunState,
    /// Workflow's durable channel binding; empty for a global workflow.
    pub bound_channel: String,
    /// Trigger values keyed without the `trigger_` prefix.
    pub trigger: HashMap<String, Value>,
    /// Prior step output fields by step identifier.
    pub step_outputs: HashMap<String, HashMap<String, Value>>,
}

/// Allowed run transition command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTransitionCommand {
    /// Start a pending run.
    Start,
    /// Record a condition skip.
    SkipStep,
    /// Record successful effect execution.
    EffectCompleted,
    /// Suspend at an approval effect.
    AwaitApproval,
    /// Resume after approval.
    Resume,
    /// Complete after all steps.
    Complete,
    /// Fail a non-terminal run.
    Fail,
    /// Cancel a non-terminal run.
    Cancel,
}

/// Verified transition facts supplied by the persistence adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowTransitionRequest {
    /// Current durable state.
    pub state: WorkflowRunState,
    /// Revision observed by the caller.
    pub expected_revision: u64,
    /// Idempotency identifier stored by the adapter.
    pub transition_id: String,
    /// Whether that identifier already exists for this run.
    pub transition_already_applied: bool,
    /// Requested transition.
    pub command: WorkflowTransitionCommand,
    /// Definition step count.
    pub step_count: u32,
    /// Step associated with the transition.
    pub step_index: u32,
}

/// Typed workflow policy request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorkflowPolicyRequest {
    /// Validate a normalized definition.
    Definition {
        /// Normalized definition to validate.
        definition: WorkflowDefinition,
    },
    /// Evaluate one condition against flat typed values.
    Condition {
        /// Evalexpr-compatible expression.
        expression: String,
        /// Trigger and step-output values under their canonical variable names.
        values: HashMap<String, Value>,
    },
    /// Plan the current step and return an effect descriptor.
    Plan {
        /// Definition, state, trigger, and prior output facts.
        request: WorkflowPlanRequest,
    },
    /// Decide one versioned run transition.
    Transition {
        /// Version and lifecycle facts for one transition attempt.
        request: WorkflowTransitionRequest,
    },
}

/// Stable workflow policy failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowPolicyError {
    /// No error.
    None,
    /// Definition name is empty.
    NameRequired,
    /// Definition has no steps.
    StepsRequired,
    /// Step identifier or timeout is invalid.
    InvalidStep,
    /// Step identifier is duplicated.
    DuplicateStep,
    /// Schedule has no cadence.
    ScheduleMissing,
    /// Schedule supplies cron and interval together.
    ScheduleConflict,
    /// Schedule syntax or interval is invalid.
    InvalidSchedule,
    /// A non-message trigger requested a threaded reply.
    ReplyRequiresMessage,
    /// Trigger fields contradict the trigger kind.
    InvalidTrigger,
    /// Action fields contradict the action kind.
    InvalidAction,
    /// A disabled definition cannot be planned.
    DefinitionDisabled,
    /// Condition syntax or complexity is invalid.
    InvalidCondition,
    /// Condition referenced an unavailable fact.
    UnknownVariable,
    /// Condition operands have incompatible types.
    TypeMismatch,
    /// Template filter or argument is invalid.
    InvalidTemplate,
    /// Planning requires a running state.
    RunNotRunning,
    /// Step index is outside the definition.
    InvalidStepIndex,
    /// The persistence revision changed.
    StaleRevision,
    /// This transition identifier already committed.
    DuplicateTransition,
    /// The requested lifecycle edge is invalid.
    InvalidTransition,
    /// Terminal runs are immutable.
    TerminalState,
    /// Transition identifier is empty or too long.
    InvalidTransitionId,
}

/// Step planner directive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowDirective {
    /// Input was rejected.
    Reject,
    /// Execute the returned effect through the port.
    ExecuteEffect,
    /// Persist a skipped step transition without an external effect.
    SkipStep,
    /// Persist completion because no step remains.
    CompleteRun,
}

/// Transition-side port effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowPortEffect {
    /// No persistence is allowed.
    None,
    /// Persist `nextState` with revision and transition-id CAS.
    PersistTransition,
}

/// Typed workflow policy result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "decision",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorkflowPolicyResult {
    /// Definition validation result.
    Definition {
        /// Whether the definition is valid.
        valid: bool,
        /// Stable validation result.
        error: WorkflowPolicyError,
        /// Whether saving/running requires owner/admin authority.
        requires_elevated_authority: bool,
    },
    /// Condition evaluation result.
    Condition {
        /// Boolean result; false on failure.
        value: bool,
        /// Stable evaluation result.
        error: WorkflowPolicyError,
    },
    /// Current-step plan.
    Plan {
        /// Selected adapter action.
        directive: WorkflowDirective,
        /// Stable planning result.
        error: WorkflowPolicyError,
        /// Current step identifier, empty when no step remains.
        step_id: String,
        /// Resolved effect descriptor, present only for `execute_effect`.
        effect: Option<Box<WorkflowAction>>,
    },
    /// Versioned state transition result.
    Transition {
        /// Whether the CAS port may persist the returned state.
        allowed: bool,
        /// Stable transition result.
        error: WorkflowPolicyError,
        /// Next state, or the unchanged current state on rejection.
        next_state: WorkflowRunState,
        /// Persistence effect selected by Nimino.
        port_effect: WorkflowPortEffect,
    },
}
