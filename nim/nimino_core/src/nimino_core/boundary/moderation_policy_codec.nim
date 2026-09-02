## Strict JSON adapter for the typed moderation policy operation.

import std/[json, options]

from ../domain/membership_policy import
  MembershipRole, mrAdmin, mrBot, mrGuest, mrMember, mrNone, mrOwner
import ../domain/moderation_policy
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError(
    "INVALID_REQUEST", message, requestId, "domain.moderation.policy"
  )

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "moderation policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "moderation policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc boolAt(node: JsonNode; key, requestId: string): bool =
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean")
  node[key].getBool()

proc secondsAt(node: JsonNode; key, requestId: string): int64 =
  if node[key].kind != JInt:
    invalid(requestId, key & " must be an integer")
  let value = node[key].getBiggestInt()
  if value < 0:
    invalid(requestId, key & " must be a non-negative 64-bit integer")
  value.int64

proc optionalSecondsAt(
    node: JsonNode; key, requestId: string
): Option[int64] =
  if node[key].kind == JNull:
    none(int64)
  else:
    some(secondsAt(node, key, requestId))

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

proc roleAt(node: JsonNode; key, requestId: string): MembershipRole =
  case stringAt(node, key, requestId)
  of "none": mrNone
  of "owner": mrOwner
  of "admin": mrAdmin
  of "member": mrMember
  of "guest": mrGuest
  of "bot": mrBot
  else: invalid(requestId, key & " is not a membership role")

proc targetAt(node: JsonNode; key, requestId: string): ModerationReportTarget =
  case stringAt(node, key, requestId)
  of "event": mrtEvent
  of "pubkey": mrtPubkey
  of "blob": mrtBlob
  else: invalid(requestId, key & " is not a report target")

proc reportTypeAt(node: JsonNode; key, requestId: string): ModerationReportType =
  case stringAt(node, key, requestId)
  of "illegal": mrtIllegal
  of "nudity": mrtNudity
  of "malware": mrtMalware
  of "spam": mrtSpam
  of "impersonation": mrtImpersonation
  of "profanity": mrtProfanity
  of "other": mrtOther
  else: invalid(requestId, key & " is not a report type")

proc restrictionCommandAt(
    node: JsonNode; key, requestId: string
): ModerationRestrictionCommand =
  case stringAt(node, key, requestId)
  of "ban": mrcBan
  of "unban": mrcUnban
  of "timeout": mrcTimeout
  of "untimeout": mrcUntimeout
  else: invalid(requestId, key & " is not a restriction command")

proc resolutionStatusAt(
    node: JsonNode; key, requestId: string
): ModerationResolutionStatus =
  case stringAt(node, key, requestId)
  of "resolved": mrsResolved
  of "dismissed": mrsDismissed
  else: invalid(requestId, key & " is not a resolution status")

proc resolutionActionAt(
    node: JsonNode; key, requestId: string
): ModerationResolutionAction =
  case stringAt(node, key, requestId)
  of "delete": mraDelete
  of "kick": mraKick
  of "ban": mraBan
  of "timeout": mraTimeout
  of "dismiss": mraDismiss
  of "escalate": mraEscalate
  else: invalid(requestId, key & " is not a resolution action")

proc enforcementOperationAt(
    node: JsonNode; key, requestId: string
): ModerationEnforcementOperation =
  case stringAt(node, key, requestId)
  of "authenticate": meoAuthenticate
  of "write": meoWrite
  else: invalid(requestId, key & " is not an enforcement operation")

proc reportRequestAt(node: JsonNode; requestId: string): ModerationReportRequest =
  requireShape(
    node,
    [
      "requestCommunity",
      "targetCommunity",
      "targetExists",
      "reporterIsTarget",
      "duplicate",
      "targetKind",
      "reportType",
    ],
    requestId,
  )
  ModerationReportRequest(
    requestCommunity: uuidAt(node, "requestCommunity", requestId),
    targetCommunity: optionalUuidAt(node, "targetCommunity", requestId),
    targetExists: boolAt(node, "targetExists", requestId),
    reporterIsTarget: boolAt(node, "reporterIsTarget", requestId),
    duplicate: boolAt(node, "duplicate", requestId),
    targetKind: targetAt(node, "targetKind", requestId),
    reportType: reportTypeAt(node, "reportType", requestId),
  )

proc restrictionRequestAt(
    node: JsonNode; requestId: string
): ModerationRestrictionRequest =
  requireShape(
    node,
    [
      "command",
      "requestCommunity",
      "actorRoleCommunity",
      "targetRoleCommunity",
      "actorRestrictionCommunity",
      "targetRestrictionCommunity",
      "actorRole",
      "targetRole",
      "actorRestrictionExists",
      "actorBanSet",
      "actorBanExpiresAt",
      "targetRestrictionExists",
      "targetBanSet",
      "targetBanExpiresAt",
      "targetMutedUntil",
      "actorIsTarget",
      "createdAtSeconds",
      "nowSeconds",
      "requestedExpiresAt",
    ],
    requestId,
  )
  ModerationRestrictionRequest(
    command: restrictionCommandAt(node, "command", requestId),
    requestCommunity: uuidAt(node, "requestCommunity", requestId),
    actorRoleCommunity: optionalUuidAt(node, "actorRoleCommunity", requestId),
    targetRoleCommunity: optionalUuidAt(node, "targetRoleCommunity", requestId),
    actorRestrictionCommunity: optionalUuidAt(
      node, "actorRestrictionCommunity", requestId
    ),
    targetRestrictionCommunity: optionalUuidAt(
      node, "targetRestrictionCommunity", requestId
    ),
    actorRole: roleAt(node, "actorRole", requestId),
    targetRole: roleAt(node, "targetRole", requestId),
    actorRestrictionExists: boolAt(node, "actorRestrictionExists", requestId),
    actorBanSet: boolAt(node, "actorBanSet", requestId),
    actorBanExpiresAt: optionalSecondsAt(node, "actorBanExpiresAt", requestId),
    targetRestrictionExists: boolAt(
      node, "targetRestrictionExists", requestId
    ),
    targetBanSet: boolAt(node, "targetBanSet", requestId),
    targetBanExpiresAt: optionalSecondsAt(
      node, "targetBanExpiresAt", requestId
    ),
    targetMutedUntil: optionalSecondsAt(node, "targetMutedUntil", requestId),
    actorIsTarget: boolAt(node, "actorIsTarget", requestId),
    createdAtSeconds: secondsAt(node, "createdAtSeconds", requestId),
    nowSeconds: secondsAt(node, "nowSeconds", requestId),
    requestedExpiresAt: optionalSecondsAt(
      node, "requestedExpiresAt", requestId
    ),
  )

proc resolutionRequestAt(
    node: JsonNode; requestId: string
): ModerationResolutionRequest =
  requireShape(
    node,
    [
      "requestCommunity",
      "actorRoleCommunity",
      "actorRestrictionCommunity",
      "reportCommunity",
      "actorRole",
      "actorRestrictionExists",
      "actorBanSet",
      "actorBanExpiresAt",
      "reportExists",
      "reportOpen",
      "createdAtSeconds",
      "nowSeconds",
      "status",
      "action",
    ],
    requestId,
  )
  ModerationResolutionRequest(
    requestCommunity: uuidAt(node, "requestCommunity", requestId),
    actorRoleCommunity: optionalUuidAt(node, "actorRoleCommunity", requestId),
    actorRestrictionCommunity: optionalUuidAt(
      node, "actorRestrictionCommunity", requestId
    ),
    reportCommunity: optionalUuidAt(node, "reportCommunity", requestId),
    actorRole: roleAt(node, "actorRole", requestId),
    actorRestrictionExists: boolAt(node, "actorRestrictionExists", requestId),
    actorBanSet: boolAt(node, "actorBanSet", requestId),
    actorBanExpiresAt: optionalSecondsAt(node, "actorBanExpiresAt", requestId),
    reportExists: boolAt(node, "reportExists", requestId),
    reportOpen: boolAt(node, "reportOpen", requestId),
    createdAtSeconds: secondsAt(node, "createdAtSeconds", requestId),
    nowSeconds: secondsAt(node, "nowSeconds", requestId),
    status: resolutionStatusAt(node, "status", requestId),
    action: resolutionActionAt(node, "action", requestId),
  )

proc enforcementRequestAt(
    node: JsonNode; requestId: string
): ModerationEnforcementRequest =
  requireShape(
    node,
    [
      "operation",
      "requestCommunity",
      "principalRestrictionCommunity",
      "ownerRestrictionCommunity",
      "principalRestrictionExists",
      "principalBanSet",
      "principalBanExpiresAt",
      "principalMutedUntil",
      "ownerAttested",
      "ownerRestrictionExists",
      "ownerBanSet",
      "ownerBanExpiresAt",
      "nowSeconds",
    ],
    requestId,
  )
  ModerationEnforcementRequest(
    operation: enforcementOperationAt(node, "operation", requestId),
    requestCommunity: uuidAt(node, "requestCommunity", requestId),
    principalRestrictionCommunity: optionalUuidAt(
      node, "principalRestrictionCommunity", requestId
    ),
    ownerRestrictionCommunity: optionalUuidAt(
      node, "ownerRestrictionCommunity", requestId
    ),
    principalRestrictionExists: boolAt(
      node, "principalRestrictionExists", requestId
    ),
    principalBanSet: boolAt(node, "principalBanSet", requestId),
    principalBanExpiresAt: optionalSecondsAt(
      node, "principalBanExpiresAt", requestId
    ),
    principalMutedUntil: optionalSecondsAt(
      node, "principalMutedUntil", requestId
    ),
    ownerAttested: boolAt(node, "ownerAttested", requestId),
    ownerRestrictionExists: boolAt(node, "ownerRestrictionExists", requestId),
    ownerBanSet: boolAt(node, "ownerBanSet", requestId),
    ownerBanExpiresAt: optionalSecondsAt(node, "ownerBanExpiresAt", requestId),
    nowSeconds: secondsAt(node, "nowSeconds", requestId),
  )

proc effectName(effect: ModerationEffect): string =
  case effect
  of meReject: "reject"
  of meQueueReport: "queue_report"
  of meApplyBan: "apply_ban"
  of meLiftBan: "lift_ban"
  of meApplyTimeout: "apply_timeout"
  of meClearTimeout: "clear_timeout"
  of meResolveReport: "resolve_report"
  of meAllow: "allow"
  of meDenyBan: "deny_ban"
  of meDenyTimeout: "deny_timeout"

proc authorityName(authority: ModerationAuthority): string =
  case authority
  of moaNone: "none"
  of moaReporter: "reporter"
  of moaCommunityOwner: "community_owner"
  of moaCommunityAdmin: "community_admin"

proc auditName(action: ModerationAuditAction): string =
  case action
  of maaNone: "none"
  of maaBan: "ban"
  of maaUnban: "unban"
  of maaTimeout: "timeout"
  of maaUntimeout: "untimeout"
  of maaDismissReport: "dismiss_report"
  of maaEscalate: "escalate"
  of maaResolveDelete: "resolve_delete"
  of maaResolveKick: "resolve_kick"
  of maaResolveBan: "resolve_ban"
  of maaResolveTimeout: "resolve_timeout"

proc errorName(error: ModerationPolicyError): string =
  case error
  of moeNone: "none"
  of moeResourceMissing: "resource_missing"
  of moeTenantMismatch: "tenant_mismatch"
  of moeFactConflict: "fact_conflict"
  of moeActorBanned: "actor_banned"
  of moeNotAuthorized: "not_authorized"
  of moeProtectedTarget: "protected_target"
  of moeSelfTarget: "self_target"
  of moeStaleCommand: "stale_command"
  of moeExpirationRequired: "expiration_required"
  of moeExpirationElapsed: "expiration_elapsed"
  of moeDuplicate: "duplicate"
  of moeNotBanned: "not_banned"
  of moeNotTimedOut: "not_timed_out"
  of moeReportClosed: "report_closed"
  of moeResolutionPair: "resolution_pair"

proc resultNode(
    decision: string; decisionResult: ModerationDecision
): JsonNode =
  %*{
    "decision": decision,
    "effect": effectName(decisionResult.effect),
    "authority": authorityName(decisionResult.authority),
    "auditAction": auditName(decisionResult.auditAction),
    "error": errorName(decisionResult.error),
  }

proc executeModerationPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "moderation policy decision is required")
  requireShape(payload, ["decision", "request"], requestId)

  case payload["decision"].getStr()
  of "report":
    resultNode(
      "report", decideModerationReport(reportRequestAt(payload["request"], requestId))
    )
  of "restriction":
    resultNode(
      "restriction",
      decideModerationRestriction(
        restrictionRequestAt(payload["request"], requestId)
      ),
    )
  of "resolution":
    resultNode(
      "resolution",
      decideModerationResolution(resolutionRequestAt(payload["request"], requestId)),
    )
  of "enforcement":
    resultNode(
      "enforcement",
      decideModerationEnforcement(
        enforcementRequestAt(payload["request"], requestId)
      ),
    )
  else:
    invalid(requestId, "unknown moderation policy decision")
