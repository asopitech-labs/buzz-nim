## Strict JSON adapter for the typed membership-policy boundary operation.

import std/[json, options]

import ../domain/membership_policy
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError(
    "INVALID_REQUEST", message, requestId, "domain.membership.policy"
  )

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "membership policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "membership policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc boolAt(node: JsonNode; key, requestId: string): bool =
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean")
  node[key].getBool()

proc intAt(node: JsonNode; key, requestId: string): int =
  if node[key].kind != JInt:
    invalid(requestId, key & " must be an integer")
  node[key].getInt()

proc roleAt(node: JsonNode; key, requestId: string): MembershipRole =
  case stringAt(node, key, requestId)
  of "none": mrNone
  of "owner": mrOwner
  of "admin": mrAdmin
  of "member": mrMember
  of "guest": mrGuest
  of "bot": mrBot
  else: invalid(requestId, key & " is not a membership role")

proc visibilityAt(node: JsonNode; key, requestId: string): ChannelVisibility =
  case stringAt(node, key, requestId)
  of "open": cvOpen
  of "private": cvPrivate
  else: invalid(requestId, key & " is not a channel visibility")

proc membershipCommandAt(
    node: JsonNode; key, requestId: string
): MembershipCommand =
  case stringAt(node, key, requestId)
  of "join": mcJoin
  of "add": mcAdd
  of "change_role": mcChangeRole
  of "remove": mcRemove
  of "leave": mcLeave
  else: invalid(requestId, key & " is not a membership command")

proc addPolicyAt(node: JsonNode; key, requestId: string): AgentAddPolicy =
  case stringAt(node, key, requestId)
  of "anyone": apAnyone
  of "owner_only": apOwnerOnly
  of "nobody": apNobody
  else: invalid(requestId, key & " is not an agent add policy")

proc inviteCommandAt(node: JsonNode; key, requestId: string): InviteCommand =
  case stringAt(node, key, requestId)
  of "mint": icMint
  of "claim": icClaim
  else: invalid(requestId, key & " is not an invite command")

proc inviteStateAt(node: JsonNode; key, requestId: string): InviteState =
  case stringAt(node, key, requestId)
  of "valid": isValid
  of "expired": isExpired
  of "exhausted": isExhausted
  of "invalid": isInvalid
  else: invalid(requestId, key & " is not an invite state")

proc channelRequestAt(
    node: JsonNode; requestId: string
): ChannelMembershipRequest =
  requireShape(
    node,
    [
      "command",
      "visibility",
      "actorRole",
      "targetRole",
      "requestedRole",
      "actorIsTarget",
      "actorOwnsTargetAgent",
      "targetIsAgent",
      "targetAddPolicy",
      "ownerCount",
    ],
    requestId,
  )
  let ownerCount = intAt(node, "ownerCount", requestId)
  if ownerCount < 0 or ownerCount > int(high(int32)):
    invalid(requestId, "ownerCount must be a non-negative 32-bit integer")
  ChannelMembershipRequest(
    command: membershipCommandAt(node, "command", requestId),
    visibility: visibilityAt(node, "visibility", requestId),
    actorRole: roleAt(node, "actorRole", requestId),
    targetRole: roleAt(node, "targetRole", requestId),
    requestedRole: roleAt(node, "requestedRole", requestId),
    actorIsTarget: boolAt(node, "actorIsTarget", requestId),
    actorOwnsTargetAgent: boolAt(node, "actorOwnsTargetAgent", requestId),
    targetIsAgent: boolAt(node, "targetIsAgent", requestId),
    targetAddPolicy: addPolicyAt(node, "targetAddPolicy", requestId),
    ownerCount: ownerCount,
  )

proc relayRequestAt(node: JsonNode; requestId: string): RelayMembershipRequest =
  requireShape(
    node,
    ["command", "actorRole", "targetRole", "requestedRole", "actorIsTarget"],
    requestId,
  )
  RelayMembershipRequest(
    command: membershipCommandAt(node, "command", requestId),
    actorRole: roleAt(node, "actorRole", requestId),
    targetRole: roleAt(node, "targetRole", requestId),
    requestedRole: roleAt(node, "requestedRole", requestId),
    actorIsTarget: boolAt(node, "actorIsTarget", requestId),
  )

proc inviteRequestAt(node: JsonNode; requestId: string): InvitePolicyRequest =
  requireShape(
    node,
    [
      "command",
      "actorRole",
      "inviteState",
      "ttlSeconds",
      "maxUses",
      "alreadyMember",
      "policyRequired",
      "policyAccepted",
    ],
    requestId,
  )
  var maxUses = none(int)
  if node["maxUses"].kind != JNull:
    let value = intAt(node, "maxUses", requestId)
    if value < int(low(int32)) or value > int(high(int32)):
      invalid(requestId, "maxUses must be a 32-bit integer or null")
    maxUses = some(value)
  InvitePolicyRequest(
    command: inviteCommandAt(node, "command", requestId),
    actorRole: roleAt(node, "actorRole", requestId),
    inviteState: inviteStateAt(node, "inviteState", requestId),
    ttlSeconds: intAt(node, "ttlSeconds", requestId).int64,
    maxUses: maxUses,
    alreadyMember: boolAt(node, "alreadyMember", requestId),
    policyRequired: boolAt(node, "policyRequired", requestId),
    policyAccepted: boolAt(node, "policyAccepted", requestId),
  )

proc ownershipRequestAt(
    node: JsonNode; requestId: string
): OwnershipTransferRequest =
  requireShape(
    node,
    [
      "actorIsOperator",
      "ownerPresent",
      "expectedOwnerMatches",
      "newOwnerIsCurrentOwner",
      "newOwnerAtLimit",
    ],
    requestId,
  )
  OwnershipTransferRequest(
    actorIsOperator: boolAt(node, "actorIsOperator", requestId),
    ownerPresent: boolAt(node, "ownerPresent", requestId),
    expectedOwnerMatches: boolAt(node, "expectedOwnerMatches", requestId),
    newOwnerIsCurrentOwner: boolAt(node, "newOwnerIsCurrentOwner", requestId),
    newOwnerAtLimit: boolAt(node, "newOwnerAtLimit", requestId),
  )

proc roleName(role: MembershipRole): string =
  case role
  of mrNone: "none"
  of mrOwner: "owner"
  of mrAdmin: "admin"
  of mrMember: "member"
  of mrGuest: "guest"
  of mrBot: "bot"

proc actionName(action: MembershipAction): string =
  case action
  of maReject: "reject"
  of maInsert: "insert"
  of maUpdateRole: "update_role"
  of maRemove: "remove"
  of maNoop: "noop"
  of maMint: "mint"
  of maJoin: "join"
  of maTransfer: "transfer"

proc errorName(error: MembershipPolicyError): string =
  case error
  of mpeNone: "none"
  of mpeNotOperator: "not_operator"
  of mpeNotAuthorized: "not_authorized"
  of mpeNotMember: "not_member"
  of mpeInviteRequired: "invite_required"
  of mpeRoleInvalid: "role_invalid"
  of mpeTargetMissing: "target_missing"
  of mpeLastOwner: "last_owner"
  of mpeSelfMutation: "self_mutation"
  of mpeOwnerProtected: "owner_protected"
  of mpeAgentAddDenied: "agent_add_denied"
  of mpeInviteInvalid: "invite_invalid"
  of mpeInviteExpired: "invite_expired"
  of mpeInviteExhausted: "invite_exhausted"
  of mpePolicyRequired: "policy_required"
  of mpeMintBounds: "mint_bounds"
  of mpeOwnerMissing: "owner_missing"
  of mpeOwnerConflict: "owner_conflict"
  of mpeOwnerLimit: "owner_limit"

proc resultNode(decision: string; decisionResult: MembershipDecision): JsonNode =
  %*{
    "decision": decision,
    "action": actionName(decisionResult.action),
    "error": errorName(decisionResult.error),
    "effectiveRole": roleName(decisionResult.effectiveRole),
  }

proc executeMembershipPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "membership policy decision is required")

  requireShape(payload, ["decision", "request"], requestId)
  case payload["decision"].getStr()
  of "channel":
    resultNode(
      "channel", decideChannelMembership(channelRequestAt(payload["request"], requestId))
    )
  of "relay":
    resultNode(
      "relay", decideRelayMembership(relayRequestAt(payload["request"], requestId))
    )
  of "invite":
    resultNode("invite", decideInvite(inviteRequestAt(payload["request"], requestId)))
  of "ownership_transfer":
    resultNode(
      "ownership_transfer",
      decideOwnershipTransfer(ownershipRequestAt(payload["request"],
          requestId)),
    )
  else:
    invalid(requestId, "unknown membership policy decision")
