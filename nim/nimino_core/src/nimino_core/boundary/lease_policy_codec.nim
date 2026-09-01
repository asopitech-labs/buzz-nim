## Strict JSON adapter for lease, fencing, and singleton-routing policy.

import std/[json, options]

import ../domain/lease_fencing
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError("INVALID_REQUEST", message, requestId, "domain.lease.policy")

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "lease policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "lease policy payload is missing " & key)

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

proc stringsAt(node: JsonNode; key, requestId: string): seq[string] =
  if node[key].kind != JArray:
    invalid(requestId, key & " must be an array")
  for item in node[key].items:
    if item.kind != JString:
      invalid(requestId, key & " must contain strings")
    result.add(item.getStr())

proc authorityAt(node: JsonNode; requestId: string): LeaseAuthority =
  requireShape(node, ["leaderId", "term", "voterEpoch", "quorumAvailable"], requestId)
  LeaseAuthority(
    leaderId: stringAt(node, "leaderId", requestId),
    term: uint64At(node, "term", requestId),
    voterEpoch: uint64At(node, "voterEpoch", requestId),
    quorumAvailable: boolAt(node, "quorumAvailable", requestId),
  )

proc commandAt(node: JsonNode; requestId: string): LeaseCommand =
  requireShape(
    node,
    [
      "resourceId", "transitionId", "ownerId", "eligibleOwners",
      "expectedPreviousFence", "fenceToken", "durationTicks", "leaderId",
      "term", "voterEpoch",
    ],
    requestId,
  )
  LeaseCommand(
    resourceId: stringAt(node, "resourceId", requestId),
    transitionId: stringAt(node, "transitionId", requestId),
    ownerId: stringAt(node, "ownerId", requestId),
    eligibleOwners: stringsAt(node, "eligibleOwners", requestId),
    expectedPreviousFence: uint64At(node, "expectedPreviousFence", requestId),
    fenceToken: uint64At(node, "fenceToken", requestId),
    durationTicks: uint64At(node, "durationTicks", requestId),
    leaderId: stringAt(node, "leaderId", requestId),
    term: uint64At(node, "term", requestId),
    voterEpoch: uint64At(node, "voterEpoch", requestId),
  )

proc activeAt(node: JsonNode; requestId: string): ActiveLease =
  requireShape(
    node,
    [
      "resourceId", "ownerId", "fenceToken", "leaderId", "term",
      "voterEpoch", "clockEpoch", "activatedAtTick", "expiresAtTick",
    ],
    requestId,
  )
  ActiveLease(
    resourceId: stringAt(node, "resourceId", requestId),
    ownerId: stringAt(node, "ownerId", requestId),
    fenceToken: uint64At(node, "fenceToken", requestId),
    leaderId: stringAt(node, "leaderId", requestId),
    term: uint64At(node, "term", requestId),
    voterEpoch: uint64At(node, "voterEpoch", requestId),
    clockEpoch: stringAt(node, "clockEpoch", requestId),
    activatedAtTick: uint64At(node, "activatedAtTick", requestId),
    expiresAtTick: uint64At(node, "expiresAtTick", requestId),
  )

proc stateAt*(node: JsonNode; requestId: string): LeaseState =
  requireShape(
    node,
    [
      "valid", "resourceId", "lastFenceToken", "lastControlIndex",
      "lastCommand", "activeLease",
    ],
    requestId,
  )
  result = LeaseState(
    valid: boolAt(node, "valid", requestId),
    resourceId: stringAt(node, "resourceId", requestId),
    lastFenceToken: uint64At(node, "lastFenceToken", requestId),
    lastControlIndex: uint64At(node, "lastControlIndex", requestId),
  )
  result.lastCommand = if node["lastCommand"].kind == JNull:
      none(LeaseCommand)
    else:
      some(commandAt(node["lastCommand"], requestId))
  result.activeLease = if node["activeLease"].kind == JNull:
      none(ActiveLease)
    else:
      some(activeAt(node["activeLease"], requestId))

proc committedFactAt(node: JsonNode; requestId: string): CommittedLeaseFact =
  requireShape(
    node,
    [
      "committed", "controlIndex", "leaderId", "term", "voterEpoch",
      "clockEpoch", "nowTick",
    ],
    requestId,
  )
  CommittedLeaseFact(
    committed: boolAt(node, "committed", requestId),
    controlIndex: uint64At(node, "controlIndex", requestId),
    leaderId: stringAt(node, "leaderId", requestId),
    term: uint64At(node, "term", requestId),
    voterEpoch: uint64At(node, "voterEpoch", requestId),
    clockEpoch: stringAt(node, "clockEpoch", requestId),
    nowTick: uint64At(node, "nowTick", requestId),
  )

proc servingFactAt*(node: JsonNode; requestId: string): ServingLeaseFact =
  requireShape(
    node,
    [
      "quorumAvailable", "leaderId", "term", "voterEpoch", "clockEpoch",
      "nowTick",
    ],
    requestId,
  )
  ServingLeaseFact(
    quorumAvailable: boolAt(node, "quorumAvailable", requestId),
    leaderId: stringAt(node, "leaderId", requestId),
    term: uint64At(node, "term", requestId),
    voterEpoch: uint64At(node, "voterEpoch", requestId),
    clockEpoch: stringAt(node, "clockEpoch", requestId),
    nowTick: uint64At(node, "nowTick", requestId),
  )

proc attemptAt(node: JsonNode; requestId: string): SingletonEffectAttempt =
  requireShape(node, ["resourceId", "ownerId", "fenceToken"], requestId)
  SingletonEffectAttempt(
    resourceId: stringAt(node, "resourceId", requestId),
    ownerId: stringAt(node, "ownerId", requestId),
    fenceToken: uint64At(node, "fenceToken", requestId),
  )

proc stringsNode(values: seq[string]): JsonNode =
  result = newJArray()
  for value in values:
    result.add(%value)

proc effectName(value: LeaseEffect): string =
  case value
  of leReject: "reject"
  of lePropose: "propose"
  of leActivate: "activate"
  of leReplay: "replay"
  of leRoute: "route"
  of leAuthorize: "authorize"

proc leaseErrorName*(value: LeaseFenceError): string =
  case value
  of lfeNone: "none"
  of lfeInvalidState: "invalid_state"
  of lfeTransitionRequired: "transition_required"
  of lfeInvalidEligibleOwners: "invalid_eligible_owners"
  of lfeLeaseDurationInvalid: "lease_duration_invalid"
  of lfeQuorumUnavailable: "quorum_unavailable"
  of lfeAuthorityInvalid: "authority_invalid"
  of lfeControlNotCommitted: "control_not_committed"
  of lfeAuthorityStale: "authority_stale"
  of lfeControlReplay: "control_replay"
  of lfeStaleFence: "stale_fence"
  of lfeFutureFence: "future_fence"
  of lfeOwnerMismatch: "owner_mismatch"
  of lfeResourceMismatch: "resource_mismatch"
  of lfeNoActiveLease: "no_active_lease"
  of lfeClockEpochMismatch: "clock_epoch_mismatch"
  of lfeClockRegression: "clock_regression"
  of lfeLeaseExpired: "lease_expired"
  of lfeReplayConflict: "replay_conflict"
  of lfeTickOverflow: "tick_overflow"

proc commandNode(value: LeaseCommand): JsonNode =
  %*{
    "resourceId": value.resourceId,
    "transitionId": value.transitionId,
    "ownerId": value.ownerId,
    "eligibleOwners": stringsNode(value.eligibleOwners),
    "expectedPreviousFence": value.expectedPreviousFence,
    "fenceToken": value.fenceToken,
    "durationTicks": value.durationTicks,
    "leaderId": value.leaderId,
    "term": value.term,
    "voterEpoch": value.voterEpoch,
  }

proc activeNode(value: ActiveLease): JsonNode =
  %*{
    "resourceId": value.resourceId,
    "ownerId": value.ownerId,
    "fenceToken": value.fenceToken,
    "leaderId": value.leaderId,
    "term": value.term,
    "voterEpoch": value.voterEpoch,
    "clockEpoch": value.clockEpoch,
    "activatedAtTick": value.activatedAtTick,
    "expiresAtTick": value.expiresAtTick,
  }

proc stateNode(value: LeaseState): JsonNode =
  result = %*{
    "valid": value.valid,
    "resourceId": value.resourceId,
    "lastFenceToken": value.lastFenceToken,
    "lastControlIndex": value.lastControlIndex,
  }
  result["lastCommand"] = if value.lastCommand.isSome:
      commandNode(value.lastCommand.get())
    else:
      newJNull()
  result["activeLease"] = if value.activeLease.isSome:
      activeNode(value.activeLease.get())
    else:
      newJNull()

proc executeLeasePolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "lease policy decision is required")
  case payload["decision"].getStr()
  of "plan_grant":
    requireShape(
      payload,
      [
        "decision", "state", "authority", "transitionId", "eligibleOwners",
        "durationTicks",
      ],
      requestId,
    )
    let plan = planLeaseGrant(
      stateAt(payload["state"], requestId),
      authorityAt(payload["authority"], requestId),
      stringAt(payload, "transitionId", requestId),
      stringsAt(payload, "eligibleOwners", requestId),
      uint64At(payload, "durationTicks", requestId),
    )
    result = %*{
      "decision": "plan_grant",
      "result": {
        "effect": effectName(plan.effect),
        "error": leaseErrorName(plan.error),
      },
    }
    result["result"]["beforeState"] = stateNode(plan.beforeState)
    result["result"]["command"] = if plan.command.isSome:
        commandNode(plan.command.get())
      else:
        newJNull()
  of "apply_committed":
    requireShape(payload, ["decision", "state", "command", "fact", "mode"], requestId)
    let mode = case stringAt(payload, "mode", requestId)
      of "live": lamLive
      of "recovery": lamRecovery
      else: invalid(requestId, "mode is not a lease apply mode")
    let decision = applyCommittedLease(
      stateAt(payload["state"], requestId),
      commandAt(payload["command"], requestId),
      committedFactAt(payload["fact"], requestId),
      mode,
    )
    result = %*{
      "decision": "apply_committed",
      "result": {
        "effect": effectName(decision.effect),
        "error": leaseErrorName(decision.error),
      },
    }
    result["result"]["state"] = stateNode(decision.state)
  of "route":
    requireShape(payload, ["decision", "state", "fact"], requestId)
    let route = routeSingleton(
      stateAt(payload["state"], requestId),
      servingFactAt(payload["fact"], requestId),
    )
    result = %*{
      "decision": "route",
      "result": {
        "allowed": route.allowed,
        "error": leaseErrorName(route.error),
        "ownerId": route.ownerId,
        "fenceToken": route.fenceToken,
      },
    }
  of "authorize":
    requireShape(payload, ["decision", "state", "attempt", "fact"], requestId)
    let decision = authorizeSingletonEffect(
      stateAt(payload["state"], requestId),
      attemptAt(payload["attempt"], requestId),
      servingFactAt(payload["fact"], requestId),
    )
    result = %*{
      "decision": "authorize",
      "result": {
        "allowed": decision.allowed,
        "error": leaseErrorName(decision.error),
      },
    }
  else:
    invalid(requestId, "unknown lease policy decision")
