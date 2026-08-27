## Strict JSON adapter for the typed direct-message policy operation.

import std/[json, options]

import ../domain/dm_policy
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError("INVALID_REQUEST", message, requestId, "domain.dm.policy")

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "DM policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "DM policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc boolAt(node: JsonNode; key, requestId: string): bool =
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean")
  node[key].getBool()

proc countAt(node: JsonNode; key, requestId: string): int =
  if node[key].kind != JInt:
    invalid(requestId, key & " must be an integer")
  result = node[key].getInt()
  if result < 0 or result > int(high(int32)):
    invalid(requestId, key & " must be a non-negative 32-bit integer")

proc uuidAt(node: JsonNode; key, requestId: string): string =
  result = stringAt(node, key, requestId)
  if result.len != 36:
    invalid(requestId, key & " must be a canonical UUID")
  for index, ch in result:
    if index in [8, 13, 18, 23]:
      if ch != '-':
        invalid(requestId, key & " must be a canonical UUID")
    elif ch notin {'0' .. '9', 'a' .. 'f'}:
      invalid(requestId, key & " must be a canonical UUID")

proc optionalUuidAt(node: JsonNode; key, requestId: string): Option[string] =
  if node[key].kind == JNull:
    none(string)
  else:
    some(uuidAt(node, key, requestId))

proc commandAt(node: JsonNode; key, requestId: string): DmCommand =
  case stringAt(node, key, requestId)
  of "open": dcOpen
  of "add": dcAdd
  of "hide": dcHide
  else: invalid(requestId, key & " is not a DM command")

proc accessAt(node: JsonNode; key, requestId: string): DmAccessOperation =
  case stringAt(node, key, requestId)
  of "read": daoRead
  of "write": daoWrite
  of "visibility": daoVisibility
  else: invalid(requestId, key & " is not a DM access operation")

proc mutationRequestAt(node: JsonNode; requestId: string): DmMutationRequest =
  requireShape(
    node,
    [
      "command",
      "requestCommunity",
      "sourceCommunity",
      "destinationCommunity",
      "sourceExists",
      "sourceIsDm",
      "actorIsSourceParticipant",
      "sourceActorHidden",
      "actorIncluded",
      "participantCount",
      "newParticipantCount",
      "destinationExists",
      "destinationIsDm",
      "destinationActorHidden",
    ],
    requestId,
  )
  DmMutationRequest(
    command: commandAt(node, "command", requestId),
    requestCommunity: uuidAt(node, "requestCommunity", requestId),
    sourceCommunity: optionalUuidAt(node, "sourceCommunity", requestId),
    destinationCommunity: optionalUuidAt(
      node, "destinationCommunity", requestId
    ),
    sourceExists: boolAt(node, "sourceExists", requestId),
    sourceIsDm: boolAt(node, "sourceIsDm", requestId),
    actorIsSourceParticipant: boolAt(
      node, "actorIsSourceParticipant", requestId
    ),
    sourceActorHidden: boolAt(node, "sourceActorHidden", requestId),
    actorIncluded: boolAt(node, "actorIncluded", requestId),
    participantCount: countAt(node, "participantCount", requestId),
    newParticipantCount: countAt(node, "newParticipantCount", requestId),
    destinationExists: boolAt(node, "destinationExists", requestId),
    destinationIsDm: boolAt(node, "destinationIsDm", requestId),
    destinationActorHidden: boolAt(
      node, "destinationActorHidden", requestId
    ),
  )

proc accessRequestAt(node: JsonNode; requestId: string): DmAccessRequest =
  requireShape(
    node,
    [
      "operation",
      "requestCommunity",
      "resourceCommunity",
      "resourceExists",
      "channelIsDm",
      "actorIsParticipant",
      "actorIsViewer",
    ],
    requestId,
  )
  DmAccessRequest(
    operation: accessAt(node, "operation", requestId),
    requestCommunity: uuidAt(node, "requestCommunity", requestId),
    resourceCommunity: optionalUuidAt(node, "resourceCommunity", requestId),
    resourceExists: boolAt(node, "resourceExists", requestId),
    channelIsDm: boolAt(node, "channelIsDm", requestId),
    actorIsParticipant: boolAt(node, "actorIsParticipant", requestId),
    actorIsViewer: boolAt(node, "actorIsViewer", requestId),
  )

proc actionName(action: DmAction): string =
  case action
  of daReject: "reject"
  of daCreate: "create"
  of daReuse: "reuse"
  of daUnhide: "unhide"
  of daHide: "hide"
  of daNoop: "noop"

proc errorName(error: DmPolicyError): string =
  case error
  of dpeNone: "none"
  of dpeResourceMissing: "resource_missing"
  of dpeTenantMismatch: "tenant_mismatch"
  of dpeNotDm: "not_dm"
  of dpeNotParticipant: "not_participant"
  of dpeActorMissing: "actor_missing"
  of dpeParticipantCount: "participant_count"
  of dpeNoNewParticipant: "no_new_participant"
  of dpeFactConflict: "fact_conflict"
  of dpeViewerMismatch: "viewer_mismatch"

proc executeDmPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "DM policy decision is required")
  requireShape(payload, ["decision", "request"], requestId)

  case payload["decision"].getStr()
  of "mutation":
    let decision = decideDmMutation(mutationRequestAt(payload["request"], requestId))
    %*{
      "decision": "mutation",
      "action": actionName(decision.action),
      "error": errorName(decision.error),
    }
  of "access":
    let decision = decideDmAccess(accessRequestAt(payload["request"], requestId))
    %*{
      "decision": "access",
      "allowed": decision.allowed,
      "error": errorName(decision.error),
    }
  else:
    invalid(requestId, "unknown DM policy decision")
