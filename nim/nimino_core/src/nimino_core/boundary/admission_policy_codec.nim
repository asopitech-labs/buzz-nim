## Strict JSON adapter for cluster-wide request admission policy.

import std/[json, options]

import ../domain/admission_policy
import ./protocol

const MaxPruneBatch = 256

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError("INVALID_REQUEST", message, requestId, "domain.admission.policy")

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "admission policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "admission policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc uint64At(node: JsonNode; key, requestId: string): uint64 =
  if node[key].kind != JInt or node[key].getBiggestInt() < 0:
    invalid(requestId, key & " must be a non-negative integer")
  uint64(node[key].getBiggestInt())

proc stateAt(node: JsonNode; requestId: string): ReplayClaimState =
  requireShape(
    node, ["scope", "eventId", "expiresAtMs", "lastControlIndex"], requestId
  )
  ReplayClaimState(
    scope: stringAt(node, "scope", requestId),
    eventId: stringAt(node, "eventId", requestId),
    expiresAtMs: uint64At(node, "expiresAtMs", requestId),
    lastControlIndex: uint64At(node, "lastControlIndex", requestId),
  )

proc commandAt(node: JsonNode; requestId: string): ReplayClaimCommand =
  requireShape(node, ["scope", "eventId", "observedAtMs", "ttlSecs"], requestId)
  ReplayClaimCommand(
    scope: stringAt(node, "scope", requestId),
    eventId: stringAt(node, "eventId", requestId),
    observedAtMs: uint64At(node, "observedAtMs", requestId),
    ttlSecs: uint64At(node, "ttlSecs", requestId),
  )

proc rateStateAt(node: JsonNode; requestId: string): RateLimitState =
  requireShape(
    node,
    [
      "namespace", "key", "windowStartedAtMs", "windowSecs", "limit", "count",
      "lastControlIndex",
    ],
    requestId,
  )
  RateLimitState(
    namespace: stringAt(node, "namespace", requestId),
    key: stringAt(node, "key", requestId),
    windowStartedAtMs: uint64At(node, "windowStartedAtMs", requestId),
    windowSecs: uint64At(node, "windowSecs", requestId),
    limit: uint64At(node, "limit", requestId),
    count: uint64At(node, "count", requestId),
    lastControlIndex: uint64At(node, "lastControlIndex", requestId),
  )

proc rateCommandAt(node: JsonNode; requestId: string): RateLimitCommand =
  requireShape(
    node, ["namespace", "key", "observedAtMs", "windowSecs", "limit"], requestId
  )
  RateLimitCommand(
    namespace: stringAt(node, "namespace", requestId),
    key: stringAt(node, "key", requestId),
    observedAtMs: uint64At(node, "observedAtMs", requestId),
    windowSecs: uint64At(node, "windowSecs", requestId),
    limit: uint64At(node, "limit", requestId),
  )

proc invalidationKindAt(
    node: JsonNode; key, requestId: string
): AuthorizationInvalidationKind =
  case stringAt(node, key, requestId)
  of "ban": aikBan
  of "membership": aikMembership
  of "visibility": aikVisibility
  of "community": aikCommunity
  else: invalid(requestId, "invalidation kind is invalid")

proc invalidationStateAt(
    node: JsonNode; requestId: string
): AuthorizationInvalidationState =
  requireShape(
    node, ["scope", "kind", "subject", "channelId", "factId", "revision"], requestId
  )
  AuthorizationInvalidationState(
    scope: stringAt(node, "scope", requestId),
    kind: invalidationKindAt(node, "kind", requestId),
    subject: stringAt(node, "subject", requestId),
    channelId: stringAt(node, "channelId", requestId),
    factId: stringAt(node, "factId", requestId),
    revision: uint64At(node, "revision", requestId),
  )

proc invalidationCommandAt(
    node: JsonNode; requestId: string
): AuthorizationInvalidationCommand =
  requireShape(
    node, ["scope", "kind", "subject", "channelId", "factId"], requestId
  )
  AuthorizationInvalidationCommand(
    scope: stringAt(node, "scope", requestId),
    kind: invalidationKindAt(node, "kind", requestId),
    subject: stringAt(node, "subject", requestId),
    channelId: stringAt(node, "channelId", requestId),
    factId: stringAt(node, "factId", requestId),
  )

proc stateNode(value: ReplayClaimState): JsonNode =
  %*{
    "scope": value.scope,
    "eventId": value.eventId,
    "expiresAtMs": value.expiresAtMs,
    "lastControlIndex": value.lastControlIndex,
  }

proc rateStateNode(value: RateLimitState): JsonNode =
  %*{
    "namespace": value.namespace,
    "key": value.key,
    "windowStartedAtMs": value.windowStartedAtMs,
    "windowSecs": value.windowSecs,
    "limit": value.limit,
    "count": value.count,
    "lastControlIndex": value.lastControlIndex,
  }

proc invalidationKindName(value: AuthorizationInvalidationKind): string =
  case value
  of aikBan: "ban"
  of aikMembership: "membership"
  of aikVisibility: "visibility"
  of aikCommunity: "community"

proc invalidationStateNode(value: AuthorizationInvalidationState): JsonNode =
  %*{
    "scope": value.scope,
    "kind": invalidationKindName(value.kind),
    "subject": value.subject,
    "channelId": value.channelId,
    "factId": value.factId,
    "revision": value.revision,
  }

proc invalidationErrorName(value: AuthorizationInvalidationError): string =
  case value
  of aieNone: "none"
  of aieInvalidState: "invalid_state"
  of aieInvalidScope: "invalid_scope"
  of aieInvalidSubject: "invalid_subject"
  of aieInvalidChannel: "invalid_channel"
  of aieInvalidFact: "invalid_fact"
  of aieInvalidRevision: "invalid_revision"

proc invalidationEffectName(value: AuthorizationInvalidationEffect): string =
  case value
  of aieApply: "apply"
  of aieReplay: "replay"
  of aieStale: "stale"
  of aieReject: "reject"

proc errorName(value: AdmissionPolicyError): string =
  case value
  of apeNone: "none"
  of apeInvalidState: "invalid_state"
  of apeInvalidScope: "invalid_scope"
  of apeInvalidEventId: "invalid_event_id"
  of apeInvalidTime: "invalid_time"
  of apeTtlOutOfRange: "ttl_out_of_range"
  of apeControlReplay: "control_replay"
  of apeInvalidNamespace: "invalid_namespace"
  of apeInvalidKey: "invalid_key"
  of apeInvalidWindow: "invalid_window"
  of apePolicyConflict: "policy_conflict"
  of apeClockRegression: "clock_regression"
  of apeCounterOverflow: "counter_overflow"
  of apeInvalidBatch: "invalid_batch"

proc rateDecisionNode(value: RateLimitDecision): JsonNode =
  result = %*{
    "allowed": value.allowed,
    "current": value.current,
    "limit": value.limit,
    "resetInSecs": value.resetInSecs,
    "error": errorName(value.error),
  }
  result["state"] = if value.state.isSome:
      rateStateNode(value.state.get())
    else:
      newJNull()

proc executeAdmissionPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "admission policy decision is required")
  case payload["decision"].getStr()
  of "apply_replay_claim":
    requireShape(payload, ["decision", "state", "command", "controlIndex"], requestId)
    let state = if payload["state"].kind == JNull:
        none(ReplayClaimState)
      else:
        some(stateAt(payload["state"], requestId))
    let decision = applyReplayClaim(
      state,
      commandAt(payload["command"], requestId),
      uint64At(payload, "controlIndex", requestId),
    )
    result = %*{
      "decision": "apply_replay_claim",
      "result": {"allowed": decision.allowed, "error": errorName(decision.error)},
    }
    result["result"]["state"] = if decision.state.isSome:
        stateNode(decision.state.get())
      else:
        newJNull()
  of "prune_replay":
    requireShape(payload, ["decision", "states", "beforeMs"], requestId)
    if payload["states"].kind != JArray or payload["states"].len > MaxPruneBatch:
      invalid(requestId, "states must be a bounded array")
    var states: seq[ReplayClaimState]
    for item in payload["states"].items:
      states.add(stateAt(item, requestId))
    let decision = pruneReplayClaims(
      states, uint64At(payload, "beforeMs", requestId)
    )
    result = %*{
      "decision": "prune_replay",
      "result": {"error": errorName(decision.error), "retained": []},
    }
    for state in decision.retained:
      result["result"]["retained"].add(stateNode(state))
  of "apply_rate_limit":
    requireShape(payload, ["decision", "state", "command", "controlIndex"], requestId)
    let state = if payload["state"].kind == JNull:
        none(RateLimitState)
      else:
        some(rateStateAt(payload["state"], requestId))
    let decision = applyRateLimit(
      state,
      rateCommandAt(payload["command"], requestId),
      uint64At(payload, "controlIndex", requestId),
    )
    result = %*{
      "decision": "apply_rate_limit",
      "result": rateDecisionNode(decision),
    }
  of "apply_rate_limit_batch":
    requireShape(payload, ["decision", "states", "commands", "controlIndex"], requestId)
    if payload["states"].kind != JArray or payload["states"].len > MaxRateBatch:
      invalid(requestId, "states must be a bounded array")
    if payload["commands"].kind != JArray or payload["commands"].len notin 1 .. MaxRateBatch:
      invalid(requestId, "commands must be a non-empty bounded array")
    var states: seq[RateLimitState]
    var commands: seq[RateLimitCommand]
    for item in payload["states"].items:
      states.add(rateStateAt(item, requestId))
    for item in payload["commands"].items:
      commands.add(rateCommandAt(item, requestId))
    let decision = applyRateLimitBatch(
      states, commands, uint64At(payload, "controlIndex", requestId)
    )
    result = %*{
      "decision": "apply_rate_limit_batch",
      "result": {"error": errorName(decision.error), "results": []},
    }
    for item in decision.results:
      result["result"]["results"].add(rateDecisionNode(item))
  of "apply_authorization_invalidation":
    requireShape(payload, ["decision", "state", "command", "revision"], requestId)
    let state = if payload["state"].kind == JNull:
        none(AuthorizationInvalidationState)
      else:
        some(invalidationStateAt(payload["state"], requestId))
    let decision = applyAuthorizationInvalidation(
      state,
      invalidationCommandAt(payload["command"], requestId),
      uint64At(payload, "revision", requestId),
    )
    result = %*{
      "decision": "apply_authorization_invalidation",
      "result": {
        "effect": invalidationEffectName(decision.effect),
        "error": invalidationErrorName(decision.error),
      },
    }
    result["result"]["state"] = if decision.state.isSome:
        invalidationStateNode(decision.state.get())
      else:
        newJNull()
  else:
    invalid(requestId, "unknown admission policy decision")
