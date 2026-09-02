## Strict JSON adapter for cluster presence and typing policy.

import std/[json, options]

import ../domain/ephemeral_policy
import ./protocol

const MaxPruneBatch = 256

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError("INVALID_REQUEST", message, requestId, "domain.ephemeral.policy")

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "ephemeral policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "ephemeral policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc boolAt(node: JsonNode; key, requestId: string): bool =
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean")
  node[key].getBool()

proc uint64At(node: JsonNode; key, requestId: string): uint64 =
  if node[key].kind != JInt or node[key].getBiggestInt() < 0:
    invalid(requestId, key & " must be a non-negative integer")
  uint64(node[key].getBiggestInt())

proc kindAt(node: JsonNode; key, requestId: string): EphemeralKind =
  case stringAt(node, key, requestId)
  of "presence": ekPresence
  of "typing": ekTyping
  else: invalid(requestId, "kind is invalid")

proc stateAt(node: JsonNode; requestId: string): EphemeralState =
  requireShape(
    node,
    [
      "scope", "kind", "subject", "context", "value", "active", "observedAtMs",
      "expiresAtMs", "originNodeId", "transitionId",
    ],
    requestId,
  )
  EphemeralState(
    scope: stringAt(node, "scope", requestId),
    kind: kindAt(node, "kind", requestId),
    subject: stringAt(node, "subject", requestId),
    context: stringAt(node, "context", requestId),
    value: stringAt(node, "value", requestId),
    active: boolAt(node, "active", requestId),
    observedAtMs: uint64At(node, "observedAtMs", requestId),
    expiresAtMs: uint64At(node, "expiresAtMs", requestId),
    originNodeId: stringAt(node, "originNodeId", requestId),
    transitionId: stringAt(node, "transitionId", requestId),
  )

proc commandAt(node: JsonNode; requestId: string): EphemeralCommand =
  requireShape(
    node,
    [
      "scope", "kind", "subject", "context", "value", "active", "observedAtMs",
      "ttlSecs", "originNodeId", "transitionId",
    ],
    requestId,
  )
  EphemeralCommand(
    scope: stringAt(node, "scope", requestId),
    kind: kindAt(node, "kind", requestId),
    subject: stringAt(node, "subject", requestId),
    context: stringAt(node, "context", requestId),
    value: stringAt(node, "value", requestId),
    active: boolAt(node, "active", requestId),
    observedAtMs: uint64At(node, "observedAtMs", requestId),
    ttlSecs: uint64At(node, "ttlSecs", requestId),
    originNodeId: stringAt(node, "originNodeId", requestId),
    transitionId: stringAt(node, "transitionId", requestId),
  )

proc kindName(value: EphemeralKind): string =
  case value
  of ekPresence: "presence"
  of ekTyping: "typing"

proc stateNode(value: EphemeralState): JsonNode =
  %*{
    "scope": value.scope,
    "kind": kindName(value.kind),
    "subject": value.subject,
    "context": value.context,
    "value": value.value,
    "active": value.active,
    "observedAtMs": value.observedAtMs,
    "expiresAtMs": value.expiresAtMs,
    "originNodeId": value.originNodeId,
    "transitionId": value.transitionId,
  }

proc errorName(value: EphemeralPolicyError): string =
  case value
  of epeNone: "none"
  of epeInvalidState: "invalid_state"
  of epeInvalidScope: "invalid_scope"
  of epeInvalidSubject: "invalid_subject"
  of epeInvalidContext: "invalid_context"
  of epeInvalidValue: "invalid_value"
  of epeInvalidTime: "invalid_time"
  of epeTtlOutOfRange: "ttl_out_of_range"
  of epeInvalidOrigin: "invalid_origin"
  of epeInvalidTransition: "invalid_transition"

proc effectName(value: EphemeralEffect): string =
  case value
  of eeApply: "apply"
  of eeReplay: "replay"
  of eeStale: "stale"
  of eeExpired: "expired"
  of eeReject: "reject"

proc executeEphemeralPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "ephemeral policy decision is required")
  case payload["decision"].getStr()
  of "apply":
    requireShape(payload, ["decision", "state", "command", "nowMs"], requestId)
    let state = if payload["state"].kind == JNull:
        none(EphemeralState)
      else:
        some(stateAt(payload["state"], requestId))
    let decision = applyEphemeral(
      state,
      commandAt(payload["command"], requestId),
      uint64At(payload, "nowMs", requestId),
    )
    result = %*{
      "decision": "apply",
      "result": {
        "effect": effectName(decision.effect), "error": errorName(decision.error)
      },
    }
    result["result"]["state"] = if decision.state.isSome:
        stateNode(decision.state.get())
      else:
        newJNull()
  of "prune":
    requireShape(payload, ["decision", "states", "beforeMs"], requestId)
    if payload["states"].kind != JArray or payload["states"].len > MaxPruneBatch:
      invalid(requestId, "states must be a bounded array")
    var states: seq[EphemeralState]
    for item in payload["states"].items:
      states.add(stateAt(item, requestId))
    let decision = pruneEphemeral(states, uint64At(payload, "beforeMs", requestId))
    result = %*{
      "decision": "prune",
      "result": {"error": errorName(decision.error), "retained": []},
    }
    for state in decision.retained:
      result["result"]["retained"].add(stateNode(state))
  else:
    invalid(requestId, "unknown ephemeral policy decision")
