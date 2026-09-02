## Strict JSON adapter for cluster admission and lifecycle policy.

import std/json

import ../domain/cluster_lifecycle
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError(
    "INVALID_REQUEST", message, requestId, "domain.cluster.lifecycle"
  )

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "cluster lifecycle payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "cluster lifecycle payload is missing " & key)

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
    invalid(requestId, key & " must be an integer")
  result = node[key].getBiggestInt().int64
  if result < 0 or result > int32.high.int64:
    invalid(requestId, key & " must be between 0 and 2147483647")

proc stateAt(node: JsonNode; key, requestId: string): ClusterNodeState =
  case stringAt(node, key, requestId)
  of "offline": cnsOffline
  of "joining": cnsJoining
  of "syncing": cnsSyncing
  of "ready": cnsReady
  of "draining": cnsDraining
  else: invalid(requestId, key & " is not a cluster node state")

proc commandAt(node: JsonNode; key, requestId: string): LifecycleCommand =
  case stringAt(node, key, requestId)
  of "join": lcJoin
  of "start_sync": lcStartSync
  of "mark_ready": lcMarkReady
  of "begin_drain": lcBeginDrain
  of "mark_offline": lcMarkOffline
  else: invalid(requestId, key & " is not a lifecycle command")

proc laneAt(node: JsonNode; key, requestId: string): ClusterLane =
  case stringAt(node, key, requestId)
  of "negotiation": clNegotiation
  of "control": clControl
  of "data_sync": clDataSync
  of "client_read": clClientRead
  of "client_write": clClientWrite
  of "lease": clLease
  else: invalid(requestId, key & " is not a cluster lane")

proc transitionRequestAt(
    node: JsonNode; requestId: string
): LifecycleTransitionRequest =
  requireShape(
    node,
    [
      "command",
      "currentState",
      "authenticated",
      "revoked",
      "identityUnique",
      "productCapability",
      "controlProtocolVersion",
      "dataProtocolVersion",
      "controlDecisionCommitted",
      "snapshotInstalled",
      "checkpointMatches",
      "requiredVoterEpoch",
      "observedVoterEpoch",
      "activeWork",
    ],
    requestId,
  )
  LifecycleTransitionRequest(
    command: commandAt(node, "command", requestId),
    currentState: stateAt(node, "currentState", requestId),
    authenticated: boolAt(node, "authenticated", requestId),
    revoked: boolAt(node, "revoked", requestId),
    identityUnique: boolAt(node, "identityUnique", requestId),
    productCapability: stringAt(node, "productCapability", requestId),
    controlProtocolVersion:
      nonNegativeIntAt(node, "controlProtocolVersion", requestId).int,
    dataProtocolVersion:
      nonNegativeIntAt(node, "dataProtocolVersion", requestId).int,
    controlDecisionCommitted: boolAt(
      node, "controlDecisionCommitted", requestId
    ),
    snapshotInstalled: boolAt(node, "snapshotInstalled", requestId),
    checkpointMatches: boolAt(node, "checkpointMatches", requestId),
    requiredVoterEpoch: nonNegativeIntAt(
      node, "requiredVoterEpoch", requestId
    ),
    observedVoterEpoch: nonNegativeIntAt(
      node, "observedVoterEpoch", requestId
    ),
    activeWork: nonNegativeIntAt(node, "activeWork", requestId).int,
  )

proc stateName(state: ClusterNodeState): string =
  case state
  of cnsOffline: "offline"
  of cnsJoining: "joining"
  of cnsSyncing: "syncing"
  of cnsReady: "ready"
  of cnsDraining: "draining"

proc effectName(effect: LifecycleEffect): string =
  case effect
  of leReject: "reject"
  of leEnterJoining: "enter_joining"
  of leEnterSyncing: "enter_syncing"
  of leEnterReady: "enter_ready"
  of leEnterDraining: "enter_draining"
  of leEnterOffline: "enter_offline"
  of leAllowLane: "allow_lane"
  of leDenyLane: "deny_lane"

proc errorName(error: ClusterLifecycleError): string =
  case error
  of cleNone: "none"
  of cleUnauthenticated: "unauthenticated"
  of cleRevoked: "revoked"
  of cleIdentityConflict: "identity_conflict"
  of cleCapabilityMismatch: "capability_mismatch"
  of cleControlVersionMismatch: "control_version_mismatch"
  of cleDataVersionMismatch: "data_version_mismatch"
  of cleInvalidTransition: "invalid_transition"
  of cleTransitionUncommitted: "transition_uncommitted"
  of cleSyncIncomplete: "sync_incomplete"
  of cleEpochMismatch: "epoch_mismatch"
  of cleDrainIncomplete: "drain_incomplete"
  of cleLaneNotAllowed: "lane_not_allowed"
  of cleFactConflict: "fact_conflict"

proc resultNode(
    decision: string; value: ClusterLifecycleDecision
): JsonNode =
  %*{
    "decision": decision,
    "effect": effectName(value.effect),
    "nextState": stateName(value.nextState),
    "error": errorName(value.error),
  }

proc executeClusterLifecycle*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "cluster lifecycle decision is required")
  requireShape(payload, ["decision", "request"], requestId)

  case payload["decision"].getStr()
  of "transition":
    resultNode(
      "transition",
      decideLifecycleTransition(
        transitionRequestAt(payload["request"], requestId)
      ),
    )
  of "lane":
    let request = payload["request"]
    requireShape(request, ["state", "lane"], requestId)
    resultNode(
      "lane",
      decideClusterLane(
        ClusterLaneRequest(
          state: stateAt(request, "state", requestId),
          lane: laneAt(request, "lane", requestId),
        )
      ),
    )
  else:
    invalid(requestId, "unknown cluster lifecycle decision")
