## Strict JSON adapter for the typed community-policy boundary operation.

import std/[json, options]

import ../domain/community_policy
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError(
    "INVALID_REQUEST", message, requestId, "domain.community.policy"
  )

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "community policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "community policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc boolAt(node: JsonNode; key, requestId: string): bool =
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean")
  node[key].getBool()

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

proc stateAt(node: JsonNode; key, requestId: string): CommunityState =
  case stringAt(node, key, requestId)
  of "missing": csMissing
  of "active": csActive
  of "archived": csArchived
  of "deleting": csDeleting
  of "deleted": csDeleted
  else: invalid(requestId, key & " is not a community state")

proc commandAt(node: JsonNode; key, requestId: string): CommunityCommand =
  case stringAt(node, key, requestId)
  of "create": ccCreate
  of "archive": ccArchive
  of "unarchive": ccUnarchive
  else: invalid(requestId, key & " is not a community command")

proc lifecycleRequestAt(
    node: JsonNode; requestId: string
): CommunityLifecycleRequest =
  requireShape(
    node,
    [
      "command",
      "state",
      "actorIsOperator",
      "actorIsOwner",
      "ownerProvided",
      "ownerAtLimit",
      "protectedDeployment",
    ],
    requestId,
  )
  CommunityLifecycleRequest(
    command: commandAt(node, "command", requestId),
    state: stateAt(node, "state", requestId),
    actorIsOperator: boolAt(node, "actorIsOperator", requestId),
    actorIsOwner: boolAt(node, "actorIsOwner", requestId),
    ownerProvided: boolAt(node, "ownerProvided", requestId),
    ownerAtLimit: boolAt(node, "ownerAtLimit", requestId),
    protectedDeployment: boolAt(node, "protectedDeployment", requestId),
  )

proc scopeRequestAt(node: JsonNode; requestId: string): CommunityScopeRequest =
  requireShape(node, ["requestCommunity", "resourceCommunity"], requestId)
  var resource = none(string)
  if node["resourceCommunity"].kind != JNull:
    resource = some(uuidAt(node, "resourceCommunity", requestId))
  CommunityScopeRequest(
    requestCommunity: uuidAt(node, "requestCommunity", requestId),
    resourceCommunity: resource,
  )

proc actionName(action: CommunityAction): string =
  case action
  of caReject: "reject"
  of caCreate: "create"
  of caArchive: "archive"
  of caUnarchive: "unarchive"
  of caNoop: "noop"

proc errorName(error: CommunityPolicyError): string =
  case error
  of cpeNone: "none"
  of cpeNotOperator: "not_operator"
  of cpeNotOwner: "not_owner"
  of cpeOwnerRequired: "owner_required"
  of cpeOwnerLimit: "owner_limit"
  of cpeHostReserved: "host_reserved"
  of cpeProtectedCommunity: "protected_community"
  of cpeInvalidState: "invalid_state"
  of cpeResourceMissing: "resource_missing"
  of cpeTenantMismatch: "tenant_mismatch"

proc executeCommunityPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "community policy decision is required")

  case payload["decision"].getStr()
  of "lifecycle":
    requireShape(payload, ["decision", "request"], requestId)
    let decision = decideLifecycle(lifecycleRequestAt(payload["request"], requestId))
    %*{
      "decision": "lifecycle",
      "action": actionName(decision.action),
      "error": errorName(decision.error),
    }
  of "scope":
    requireShape(payload, ["decision", "request"], requestId)
    let decision = decideScope(scopeRequestAt(payload["request"], requestId))
    %*{
      "decision": "scope",
      "allowed": decision.allowed,
      "error": errorName(decision.error),
    }
  else:
    invalid(requestId, "unknown community policy decision")
