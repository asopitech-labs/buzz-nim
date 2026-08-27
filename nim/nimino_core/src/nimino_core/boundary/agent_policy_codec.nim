## Strict JSON adapter for the typed agent/persona policy operation.

import std/[json, options]

import ../domain/agent_policy
import ./protocol

const
  MaxAgentStringBytes = 4096
  MaxAgentListItems = 256

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError("INVALID_REQUEST", message, requestId, "domain.agent.policy")

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "agent policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "agent policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  result = node[key].getStr()
  if result.len > MaxAgentStringBytes:
    invalid(requestId, key & " is too long")

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

proc optionalStringAt(
    node: JsonNode; key, requestId: string
): Option[string] =
  if node[key].kind == JNull:
    return none(string)
  some(stringAt(node, key, requestId))

proc optionalBoolAt(node: JsonNode; key, requestId: string): Option[bool] =
  if node[key].kind == JNull:
    return none(bool)
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean or null")
  some(node[key].getBool())

proc optionalFloatAt(
    node: JsonNode; key, requestId: string
): Option[float64] =
  case node[key].kind
  of JNull:
    none(float64)
  of JInt, JFloat:
    some(node[key].getFloat())
  else:
    invalid(requestId, key & " must be a number or null")

proc optionalIntAt(node: JsonNode; key, requestId: string): Option[int64] =
  if node[key].kind == JNull:
    return none(int64)
  some(nonNegativeIntAt(node, key, requestId))

proc stringListAt(node: JsonNode; key, requestId: string): seq[string] =
  if node[key].kind != JArray or node[key].len > MaxAgentListItems:
    invalid(requestId, key & " must be a bounded array")
  for value in node[key]:
    if value.kind != JString or value.getStr().len > MaxAgentStringBytes:
      invalid(requestId, key & " values must be bounded strings")
    result.add(value.getStr())

proc optionalStringListAt(
    node: JsonNode; key, requestId: string
): Option[seq[string]] =
  if node[key].kind == JNull:
    return none(seq[string])
  some(stringListAt(node, key, requestId))

proc triggersAt(
    node: JsonNode; key, requestId: string
): Option[PersonaTriggers] =
  if node[key].kind == JNull:
    return none(PersonaTriggers)
  let value = node[key]
  requireShape(value, ["mentions", "keywords", "allMessages"], requestId)
  some(PersonaTriggers(
    mentions: optionalBoolAt(value, "mentions", requestId),
    keywords: optionalStringListAt(value, "keywords", requestId),
    allMessages: optionalBoolAt(value, "allMessages", requestId),
  ))

proc behaviorAt(node: JsonNode; requestId: string): PersonaBehavior =
  requireShape(
    node,
    [
      "model", "temperature", "maxContextTokens", "subscribe", "triggers",
      "threadReplies", "broadcastReplies",
    ],
    requestId,
  )
  PersonaBehavior(
    model: optionalStringAt(node, "model", requestId),
    temperature: optionalFloatAt(node, "temperature", requestId),
    maxContextTokens: optionalIntAt(node, "maxContextTokens", requestId),
    subscribe: optionalStringListAt(node, "subscribe", requestId),
    triggers: triggersAt(node, "triggers", requestId),
    threadReplies: optionalBoolAt(node, "threadReplies", requestId),
    broadcastReplies: optionalBoolAt(node, "broadcastReplies", requestId),
  )

proc eventAt(node: JsonNode; requestId: string): AgentEventFacts =
  requireShape(
    node, ["content", "author", "kind", "channelId", "timestamp", "mentioned"],
    requestId,
  )
  AgentEventFacts(
    content: stringAt(node, "content", requestId),
    author: stringAt(node, "author", requestId),
    kind: nonNegativeIntAt(node, "kind", requestId),
    channelId: stringAt(node, "channelId", requestId),
    timestamp: nonNegativeIntAt(node, "timestamp", requestId),
    mentioned: boolAt(node, "mentioned", requestId),
  )

proc intListAt(node: JsonNode; key, requestId: string): seq[int64] =
  if node[key].kind != JArray or node[key].len > MaxAgentListItems:
    invalid(requestId, key & " must be a bounded array")
  for value in node[key]:
    if value.kind != JInt or value.getBiggestInt() < 0:
      invalid(requestId, key & " values must be non-negative integers")
    result.add(value.getBiggestInt().int64)

proc ruleAt(node: JsonNode; requestId: string): AgentTriggerRule =
  requireShape(
    node,
    [
      "name", "allChannels", "channels", "kinds", "requireMention", "filter",
      "promptTag",
    ],
    requestId,
  )
  AgentTriggerRule(
    name: stringAt(node, "name", requestId),
    allChannels: boolAt(node, "allChannels", requestId),
    channels: stringListAt(node, "channels", requestId),
    kinds: intListAt(node, "kinds", requestId),
    requireMention: boolAt(node, "requireMention", requestId),
    filter: stringAt(node, "filter", requestId),
    promptTag: stringAt(node, "promptTag", requestId),
  )

proc phaseAt(node: JsonNode; key, requestId: string): AgentPhase =
  case stringAt(node, key, requestId)
  of "absent": apAbsent
  of "starting": apStarting
  of "ready": apReady
  of "running": apRunning
  of "cancelling": apCancelling
  of "restart_wait": apRestartWait
  of "stopped": apStopped
  else: invalid(requestId, key & " is not an agent phase")

proc commandAt(
    node: JsonNode; key, requestId: string
): AgentLifecycleCommand =
  case stringAt(node, key, requestId)
  of "start": alcStart
  of "started": alcStarted
  of "start_failed": alcStartFailed
  of "begin_turn": alcBeginTurn
  of "cancel": alcCancel
  of "turn_finished": alcTurnFinished
  of "cancel_timeout": alcCancelTimeout
  of "process_exited": alcProcessExited
  of "retry": alcRetry
  of "shutdown": alcShutdown
  else: invalid(requestId, key & " is not an agent lifecycle command")

proc stateAt(node: JsonNode; requestId: string): AgentLifecycleState =
  requireShape(node, ["phase", "attempt", "retryAtMs", "turnId"], requestId)
  AgentLifecycleState(
    phase: phaseAt(node, "phase", requestId),
    attempt: nonNegativeIntAt(node, "attempt", requestId),
    retryAtMs: nonNegativeIntAt(node, "retryAtMs", requestId),
    turnId: stringAt(node, "turnId", requestId),
  )

proc lifecycleAt(node: JsonNode; requestId: string): AgentLifecycleRequest =
  requireShape(
    node,
    [
      "state", "command", "commandAttempt", "commandTurnId", "pendingWork",
      "nowMs",
    ],
    requestId,
  )
  AgentLifecycleRequest(
    state: stateAt(node["state"], requestId),
    command: commandAt(node, "command", requestId),
    commandAttempt: nonNegativeIntAt(node, "commandAttempt", requestId),
    commandTurnId: stringAt(node, "commandTurnId", requestId),
    pendingWork: boolAt(node, "pendingWork", requestId),
    nowMs: nonNegativeIntAt(node, "nowMs", requestId),
  )

proc optionNode(value: Option[string]): JsonNode =
  if value.isSome: %value.get() else: newJNull()

proc optionNode(value: Option[float64]): JsonNode =
  if value.isSome: %value.get() else: newJNull()

proc optionNode(value: Option[int64]): JsonNode =
  if value.isSome: %value.get() else: newJNull()

proc optionNode(value: Option[seq[string]]): JsonNode =
  if value.isSome: %value.get() else: newJNull()

proc triggersNode(value: Option[ResolvedPersonaTriggers]): JsonNode =
  if value.isNone:
    return newJNull()
  let triggers = value.get()
  %*{
    "mentions": triggers.mentions,
    "keywords": triggers.keywords,
    "allMessages": triggers.allMessages,
  }

proc behaviorResult(value: ResolvedPersonaBehavior): JsonNode =
  %*{
    "decision": "persona",
    "behavior": {
      "model": optionNode(value.model),
      "temperature": optionNode(value.temperature),
      "maxContextTokens": optionNode(value.maxContextTokens),
      "subscribe": optionNode(value.subscribe),
      "triggers": triggersNode(value.triggers),
      "threadReplies": value.threadReplies,
      "broadcastReplies": value.broadcastReplies,
    },
  }

proc errorName(value: AgentTriggerError): string =
  case value
  of ateNone: "none"
  of ateInvalidRule: "invalid_rule"
  of ateInvalidFilter: "invalid_filter"

proc triggerResult(value: AgentTriggerDecision): JsonNode =
  %*{
    "decision": "trigger",
    "matched": value.matched,
    "ruleIndex": value.ruleIndex,
    "promptTag": value.promptTag,
    "error": errorName(value.error),
  }

proc phaseName(value: AgentPhase): string =
  case value
  of apAbsent: "absent"
  of apStarting: "starting"
  of apReady: "ready"
  of apRunning: "running"
  of apCancelling: "cancelling"
  of apRestartWait: "restart_wait"
  of apStopped: "stopped"

proc lifecycleErrorName(value: AgentLifecycleError): string =
  case value
  of aleNone: "none"
  of aleInvalidTransition: "invalid_transition"
  of aleStaleAttempt: "stale_attempt"
  of aleInvalidTurn: "invalid_turn"

proc actionName(value: AgentLifecycleAction): string =
  case value
  of alaReject: "reject"
  of alaNoop: "noop"
  of alaSpawn: "spawn"
  of alaAcceptStart: "accept_start"
  of alaBeginTurn: "begin_turn"
  of alaSendCancel: "send_cancel"
  of alaReturnReady: "return_ready"
  of alaReapAndWait: "reap_and_wait"
  of alaStop: "stop"

proc stateNode(value: AgentLifecycleState): JsonNode =
  %*{
    "phase": phaseName(value.phase),
    "attempt": value.attempt,
    "retryAtMs": value.retryAtMs,
    "turnId": value.turnId,
  }

proc lifecycleResult(value: AgentLifecycleDecision): JsonNode =
  %*{
    "decision": "lifecycle",
    "allowed": value.allowed,
    "error": lifecycleErrorName(value.error),
    "action": actionName(value.action),
    "nextState": stateNode(value.nextState),
  }

proc executeAgentPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "agent policy decision is required")
  case payload["decision"].getStr()
  of "persona":
    requireShape(payload, ["decision", "persona", "defaults"], requestId)
    behaviorResult(resolvePersonaBehavior(
      behaviorAt(payload["persona"], requestId),
      behaviorAt(payload["defaults"], requestId),
    ))
  of "trigger":
    requireShape(payload, ["decision", "event", "rules"], requestId)
    if payload["rules"].kind != JArray or
        payload["rules"].len > MaxAgentListItems:
      invalid(requestId, "rules must be a bounded array")
    var rules: seq[AgentTriggerRule]
    for rule in payload["rules"]:
      rules.add(ruleAt(rule, requestId))
    triggerResult(decideAgentTrigger(eventAt(payload["event"], requestId), rules))
  of "lifecycle":
    requireShape(payload, ["decision", "request"], requestId)
    lifecycleResult(decideAgentLifecycle(
      lifecycleAt(payload["request"], requestId)
    ))
  else:
    invalid(requestId, "unknown agent policy decision")
