## Strict JSON adapter for the typed workflow-policy operation.

import std/[json, strutils, tables]

import ../domain/workflow_policy
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError(
    "INVALID_REQUEST", message, requestId, "domain.workflow.policy"
  )

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "workflow policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "workflow policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc boolAt(node: JsonNode; key, requestId: string): bool =
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean")
  node[key].getBool()

proc nonNegativeIntAt(node: JsonNode; key, requestId: string): int64 =
  if node[key].kind != JInt:
    invalid(requestId, key & " must be a non-negative integer")
  result = node[key].getBiggestInt().int64
  if result < 0:
    invalid(requestId, key & " must be a non-negative integer")

proc triggerKindAt(
    node: JsonNode; key, requestId: string
): WorkflowTriggerKind =
  case stringAt(node, key, requestId)
  of "message_posted": wtMessagePosted
  of "reaction_added": wtReactionAdded
  of "diff_posted": wtDiffPosted
  of "schedule": wtSchedule
  of "webhook": wtWebhook
  else: invalid(requestId, key & " is not a workflow trigger")

proc actionKindAt(
    node: JsonNode; key, requestId: string
): WorkflowActionKind =
  case stringAt(node, key, requestId)
  of "send_message": waSendMessage
  of "send_dm": waSendDm
  of "set_channel_topic": waSetChannelTopic
  of "add_reaction": waAddReaction
  of "call_webhook": waCallWebhook
  of "request_approval": waRequestApproval
  of "delay": waDelay
  else: invalid(requestId, key & " is not a workflow action")

proc runStatusAt(node: JsonNode; key, requestId: string): WorkflowRunStatus =
  case stringAt(node, key, requestId)
  of "pending": wrPending
  of "running": wrRunning
  of "waiting_approval": wrWaitingApproval
  of "completed": wrCompleted
  of "failed": wrFailed
  of "cancelled": wrCancelled
  else: invalid(requestId, key & " is not a workflow run status")

proc transitionCommandAt(
    node: JsonNode; key, requestId: string
): WorkflowTransitionCommand =
  case stringAt(node, key, requestId)
  of "start": wcStart
  of "skip_step": wcSkipStep
  of "effect_completed": wcEffectCompleted
  of "await_approval": wcAwaitApproval
  of "resume": wcResume
  of "complete": wcComplete
  of "fail": wcFail
  of "cancel": wcCancel
  else: invalid(requestId, key & " is not a workflow transition")

proc stringTableAt(
    node: JsonNode; key, requestId: string
): Table[string, string] =
  let source = node[key]
  if source.kind != JObject:
    invalid(requestId, key & " must be an object")
  for name, value in source:
    if value.kind != JString:
      invalid(requestId, key & " values must be strings")
    result[name] = value.getStr()

proc valueAt(node: JsonNode; requestId: string): WorkflowValue =
  case node.kind
  of JNull:
    emptyValue()
  of JString:
    stringValue(node.getStr())
  of JBool:
    boolValue(node.getBool())
  of JInt:
    intValue(node.getBiggestInt().int64)
  of JFloat:
    floatValue(node.getFloat())
  else:
    invalid(requestId, "workflow values must be JSON scalars")

proc valueTableAt(
    node: JsonNode; requestId: string
): Table[string, WorkflowValue] =
  if node.kind != JObject:
    invalid(requestId, "workflow values must be an object")
  for name, value in node:
    if name.len == 0 or name.len > 256:
      invalid(requestId, "workflow value name is invalid")
    for ch in name:
      if not (ch.isAlphaNumeric() or ch == '_') or ch.ord > 127:
        invalid(requestId, "workflow value name is invalid")
    result[name] = valueAt(value, requestId)

proc nestedValueTableAt(
    node: JsonNode; requestId: string
): Table[string, Table[string, WorkflowValue]] =
  if node.kind != JObject:
    invalid(requestId, "stepOutputs must be an object")
  for stepId, output in node:
    if stepId.len == 0 or stepId.len > MaxWorkflowStepIdBytes:
      invalid(requestId, "step output id is invalid")
    result[stepId] = valueTableAt(output, requestId)

proc triggerAt(node: JsonNode; requestId: string): WorkflowTrigger =
  requireShape(node, ["kind", "filter", "emoji", "cron", "interval"], requestId)
  WorkflowTrigger(
    kind: triggerKindAt(node, "kind", requestId),
    filter: stringAt(node, "filter", requestId),
    emoji: stringAt(node, "emoji", requestId),
    cron: stringAt(node, "cron", requestId),
    interval: stringAt(node, "interval", requestId),
  )

proc actionAt(node: JsonNode; requestId: string): WorkflowAction =
  requireShape(
    node,
    [
      "kind",
      "text",
      "channel",
      "replyInThread",
      "recipient",
      "topic",
      "emoji",
      "url",
      "httpMethod",
      "headers",
      "body",
      "approver",
      "message",
      "timeout",
      "duration",
    ],
    requestId,
  )
  WorkflowAction(
    kind: actionKindAt(node, "kind", requestId),
    text: stringAt(node, "text", requestId),
    channel: stringAt(node, "channel", requestId),
    replyInThread: boolAt(node, "replyInThread", requestId),
    recipient: stringAt(node, "recipient", requestId),
    topic: stringAt(node, "topic", requestId),
    emoji: stringAt(node, "emoji", requestId),
    url: stringAt(node, "url", requestId),
    httpMethod: stringAt(node, "httpMethod", requestId),
    headers: stringTableAt(node, "headers", requestId),
    body: stringAt(node, "body", requestId),
    approver: stringAt(node, "approver", requestId),
    message: stringAt(node, "message", requestId),
    timeout: stringAt(node, "timeout", requestId),
    duration: stringAt(node, "duration", requestId),
  )

proc stepAt(node: JsonNode; requestId: string): WorkflowStep =
  requireShape(
    node, ["id", "name", "condition", "timeoutSecs", "action"], requestId
  )
  let timeoutSecs = nonNegativeIntAt(node, "timeoutSecs", requestId)
  WorkflowStep(
    id: stringAt(node, "id", requestId),
    name: stringAt(node, "name", requestId),
    condition: stringAt(node, "condition", requestId),
    timeoutSecs: timeoutSecs,
    action: actionAt(node["action"], requestId),
  )

proc definitionAt(node: JsonNode; requestId: string): WorkflowDefinition =
  requireShape(
    node, ["name", "description", "trigger", "steps", "enabled"], requestId
  )
  if node["steps"].kind != JArray:
    invalid(requestId, "steps must be an array")
  var steps: seq[WorkflowStep]
  for step in node["steps"]:
    steps.add stepAt(step, requestId)
  WorkflowDefinition(
    name: stringAt(node, "name", requestId),
    description: stringAt(node, "description", requestId),
    trigger: triggerAt(node["trigger"], requestId),
    steps: steps,
    enabled: boolAt(node, "enabled", requestId),
  )

proc stateAt(node: JsonNode; requestId: string): WorkflowRunState =
  requireShape(node, ["status", "currentStep", "revision"], requestId)
  let currentStep = nonNegativeIntAt(node, "currentStep", requestId)
  if currentStep > int64(high(int32)):
    invalid(requestId, "currentStep exceeds the 32-bit limit")
  WorkflowRunState(
    status: runStatusAt(node, "status", requestId),
    currentStep: currentStep.int,
    revision: nonNegativeIntAt(node, "revision", requestId),
  )

proc planRequestAt(node: JsonNode; requestId: string): WorkflowPlanRequest =
  requireShape(
    node,
    ["definition", "state", "boundChannel", "trigger", "stepOutputs"],
    requestId,
  )
  WorkflowPlanRequest(
    definition: definitionAt(node["definition"], requestId),
    state: stateAt(node["state"], requestId),
    boundChannel: stringAt(node, "boundChannel", requestId),
    trigger: valueTableAt(node["trigger"], requestId),
    stepOutputs: nestedValueTableAt(node["stepOutputs"], requestId),
  )

proc transitionRequestAt(
    node: JsonNode; requestId: string
): WorkflowTransitionRequest =
  requireShape(
    node,
    [
      "state",
      "expectedRevision",
      "transitionId",
      "transitionAlreadyApplied",
      "command",
      "stepCount",
      "stepIndex",
    ],
    requestId,
  )
  let stepCount = nonNegativeIntAt(node, "stepCount", requestId)
  let stepIndex = nonNegativeIntAt(node, "stepIndex", requestId)
  if stepCount > int64(high(int32)) or stepIndex > int64(high(int32)):
    invalid(requestId, "transition step index exceeds the 32-bit limit")
  WorkflowTransitionRequest(
    state: stateAt(node["state"], requestId),
    expectedRevision: nonNegativeIntAt(node, "expectedRevision", requestId),
    transitionId: stringAt(node, "transitionId", requestId),
    transitionAlreadyApplied: boolAt(
      node, "transitionAlreadyApplied", requestId
    ),
    command: transitionCommandAt(node, "command", requestId),
    stepCount: stepCount.int,
    stepIndex: stepIndex.int,
  )

proc errorName(error: WorkflowPolicyError): string =
  case error
  of wpeNone: "none"
  of wpeNameRequired: "name_required"
  of wpeStepsRequired: "steps_required"
  of wpeInvalidStep: "invalid_step"
  of wpeDuplicateStep: "duplicate_step"
  of wpeScheduleMissing: "schedule_missing"
  of wpeScheduleConflict: "schedule_conflict"
  of wpeInvalidSchedule: "invalid_schedule"
  of wpeReplyRequiresMessage: "reply_requires_message"
  of wpeInvalidTrigger: "invalid_trigger"
  of wpeInvalidAction: "invalid_action"
  of wpeDefinitionDisabled: "definition_disabled"
  of wpeInvalidCondition: "invalid_condition"
  of wpeUnknownVariable: "unknown_variable"
  of wpeTypeMismatch: "type_mismatch"
  of wpeInvalidTemplate: "invalid_template"
  of wpeRunNotRunning: "run_not_running"
  of wpeInvalidStepIndex: "invalid_step_index"
  of wpeStaleRevision: "stale_revision"
  of wpeDuplicateTransition: "duplicate_transition"
  of wpeInvalidTransition: "invalid_transition"
  of wpeTerminalState: "terminal_state"
  of wpeInvalidTransitionId: "invalid_transition_id"

proc actionName(action: WorkflowActionKind): string =
  case action
  of waSendMessage: "send_message"
  of waSendDm: "send_dm"
  of waSetChannelTopic: "set_channel_topic"
  of waAddReaction: "add_reaction"
  of waCallWebhook: "call_webhook"
  of waRequestApproval: "request_approval"
  of waDelay: "delay"

proc actionJson(action: WorkflowAction): JsonNode =
  var headers = newJObject()
  for key, value in action.headers:
    headers[key] = %value
  %*{
    "kind": actionName(action.kind),
    "text": action.text,
    "channel": action.channel,
    "replyInThread": action.replyInThread,
    "recipient": action.recipient,
    "topic": action.topic,
    "emoji": action.emoji,
    "url": action.url,
    "httpMethod": action.httpMethod,
    "headers": headers,
    "body": action.body,
    "approver": action.approver,
    "message": action.message,
    "timeout": action.timeout,
    "duration": action.duration,
  }

proc directiveName(directive: WorkflowDirective): string =
  case directive
  of wdReject: "reject"
  of wdExecuteEffect: "execute_effect"
  of wdSkipStep: "skip_step"
  of wdCompleteRun: "complete_run"

proc statusName(status: WorkflowRunStatus): string =
  case status
  of wrPending: "pending"
  of wrRunning: "running"
  of wrWaitingApproval: "waiting_approval"
  of wrCompleted: "completed"
  of wrFailed: "failed"
  of wrCancelled: "cancelled"

proc stateJson(state: WorkflowRunState): JsonNode =
  %*{
    "status": statusName(state.status),
    "currentStep": state.currentStep,
    "revision": state.revision,
  }

proc portEffectName(effect: WorkflowPortEffect): string =
  case effect
  of wpfNone: "none"
  of wpfPersistTransition: "persist_transition"

proc executeWorkflowPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "workflow policy decision is required")

  case payload["decision"].getStr()
  of "definition":
    requireShape(payload, ["decision", "definition"], requestId)
    let decision = validateWorkflowDefinition(
      definitionAt(payload["definition"], requestId)
    )
    %*{
      "decision": "definition",
      "valid": decision.valid,
      "error": errorName(decision.error),
      "requiresElevatedAuthority": decision.requiresElevatedAuthority,
    }
  of "condition":
    requireShape(payload, ["decision", "expression", "values"], requestId)
    let decision = evaluateWorkflowCondition(
      stringAt(payload, "expression", requestId),
      valueTableAt(payload["values"], requestId),
    )
    %*{
      "decision": "condition",
      "value": decision.value,
      "error": errorName(decision.error),
    }
  of "plan":
    requireShape(payload, ["decision", "request"], requestId)
    let decision = planWorkflowStep(planRequestAt(payload["request"], requestId))
    %*{
      "decision": "plan",
      "directive": directiveName(decision.directive),
      "error": errorName(decision.error),
      "stepId": decision.stepId,
      "effect": if decision.directive == wdExecuteEffect:
        actionJson(decision.effect)
      else:
        newJNull(),
    }
  of "transition":
    requireShape(payload, ["decision", "request"], requestId)
    let decision = decideWorkflowTransition(
      transitionRequestAt(payload["request"], requestId)
    )
    %*{
      "decision": "transition",
      "allowed": decision.allowed,
      "error": errorName(decision.error),
      "nextState": stateJson(decision.nextState),
      "portEffect": portEffectName(decision.portEffect),
    }
  else:
    invalid(requestId, "unknown workflow policy decision")
