import std/options

from ./membership_policy import MembershipRole, mrAdmin, mrNone, mrOwner

const MaxModerationCommandSkewSeconds* = 120'i64

type
  ModerationReportTarget* = enum
    mrtEvent
    mrtPubkey
    mrtBlob

  ModerationReportType* = enum
    mrtIllegal
    mrtNudity
    mrtMalware
    mrtSpam
    mrtImpersonation
    mrtProfanity
    mrtOther

  ModerationRestrictionCommand* = enum
    mrcBan
    mrcUnban
    mrcTimeout
    mrcUntimeout

  ModerationResolutionStatus* = enum
    mrsResolved
    mrsDismissed

  ModerationResolutionAction* = enum
    mraDelete
    mraKick
    mraBan
    mraTimeout
    mraDismiss
    mraEscalate

  ModerationEnforcementOperation* = enum
    meoAuthenticate
    meoWrite

  ModerationEffect* = enum
    meReject
    meQueueReport
    meApplyBan
    meLiftBan
    meApplyTimeout
    meClearTimeout
    meResolveReport
    meAllow
    meDenyBan
    meDenyTimeout

  ModerationAuthority* = enum
    moaNone
    moaReporter
    moaCommunityOwner
    moaCommunityAdmin

  ModerationAuditAction* = enum
    maaNone
    maaBan
    maaUnban
    maaTimeout
    maaUntimeout
    maaDismissReport
    maaEscalate
    maaResolveDelete
    maaResolveKick
    maaResolveBan
    maaResolveTimeout

  ModerationPolicyError* = enum
    moeNone
    moeResourceMissing
    moeTenantMismatch
    moeFactConflict
    moeActorBanned
    moeNotAuthorized
    moeProtectedTarget
    moeSelfTarget
    moeStaleCommand
    moeExpirationRequired
    moeExpirationElapsed
    moeDuplicate
    moeNotBanned
    moeNotTimedOut
    moeReportClosed
    moeResolutionPair

  ModerationDecision* = object
    effect*: ModerationEffect
    authority*: ModerationAuthority
    auditAction*: ModerationAuditAction
    error*: ModerationPolicyError

  ModerationReportRequest* = object
    requestCommunity*: string
    targetCommunity*: Option[string]
    targetExists*: bool
    reporterIsTarget*: bool
    duplicate*: bool
    targetKind*: ModerationReportTarget
    reportType*: ModerationReportType

  ModerationRestrictionRequest* = object
    command*: ModerationRestrictionCommand
    requestCommunity*: string
    actorRoleCommunity*: Option[string]
    targetRoleCommunity*: Option[string]
    actorRestrictionCommunity*: Option[string]
    targetRestrictionCommunity*: Option[string]
    actorRole*: MembershipRole
    targetRole*: MembershipRole
    actorRestrictionExists*: bool
    actorBanSet*: bool
    actorBanExpiresAt*: Option[int64]
    targetRestrictionExists*: bool
    targetBanSet*: bool
    targetBanExpiresAt*: Option[int64]
    targetMutedUntil*: Option[int64]
    actorIsTarget*: bool
    createdAtSeconds*: int64
    nowSeconds*: int64
    requestedExpiresAt*: Option[int64]

  ModerationResolutionRequest* = object
    requestCommunity*: string
    actorRoleCommunity*: Option[string]
    actorRestrictionCommunity*: Option[string]
    reportCommunity*: Option[string]
    actorRole*: MembershipRole
    actorRestrictionExists*: bool
    actorBanSet*: bool
    actorBanExpiresAt*: Option[int64]
    reportExists*: bool
    reportOpen*: bool
    createdAtSeconds*: int64
    nowSeconds*: int64
    status*: ModerationResolutionStatus
    action*: ModerationResolutionAction

  ModerationEnforcementRequest* = object
    operation*: ModerationEnforcementOperation
    requestCommunity*: string
    principalRestrictionCommunity*: Option[string]
    ownerRestrictionCommunity*: Option[string]
    principalRestrictionExists*: bool
    principalBanSet*: bool
    principalBanExpiresAt*: Option[int64]
    principalMutedUntil*: Option[int64]
    ownerAttested*: bool
    ownerRestrictionExists*: bool
    ownerBanSet*: bool
    ownerBanExpiresAt*: Option[int64]
    nowSeconds*: int64

proc reject(error: ModerationPolicyError): ModerationDecision =
  ModerationDecision(effect: meReject, error: error)

proc resourceScopeError(
    requestCommunity: string;
    resourceCommunity: Option[string];
    resourceExists: bool
): ModerationPolicyError =
  if not resourceExists:
    return if resourceCommunity.isSome: moeFactConflict else: moeResourceMissing
  if resourceCommunity.isNone:
    return moeFactConflict
  if resourceCommunity.get() != requestCommunity:
    return moeTenantMismatch
  moeNone

proc roleScopeError(
    requestCommunity: string;
    roleCommunity: Option[string];
    role: MembershipRole
): ModerationPolicyError =
  if role == mrNone:
    return if roleCommunity.isSome: moeFactConflict else: moeNone
  if roleCommunity.isNone:
    return moeFactConflict
  if roleCommunity.get() != requestCommunity:
    return moeTenantMismatch
  moeNone

proc restrictionScopeError(
    requestCommunity: string;
    restrictionCommunity: Option[string];
    restrictionExists: bool;
    banSet: bool;
    banExpiresAt: Option[int64];
    mutedUntil: Option[int64]
): ModerationPolicyError =
  if not restrictionExists:
    if restrictionCommunity.isSome or banSet or banExpiresAt.isSome or
        mutedUntil.isSome:
      return moeFactConflict
    return moeNone
  if restrictionCommunity.isNone:
    return moeFactConflict
  if restrictionCommunity.get() != requestCommunity:
    return moeTenantMismatch
  if not banSet and banExpiresAt.isSome:
    return moeFactConflict
  moeNone

proc commandIsFresh(createdAt, now: int64): bool =
  if createdAt >= now:
    createdAt - now <= MaxModerationCommandSkewSeconds
  else:
    now - createdAt <= MaxModerationCommandSkewSeconds

proc banIsActive(
    banSet: bool; expiresAt: Option[int64]; now: int64
): bool =
  banSet and (expiresAt.isNone or expiresAt.get() > now)

proc timeoutIsActive(mutedUntil: Option[int64]; now: int64): bool =
  mutedUntil.isSome and mutedUntil.get() > now

proc actorAuthority(role: MembershipRole): ModerationAuthority =
  case role
  of mrOwner: moaCommunityOwner
  of mrAdmin: moaCommunityAdmin
  else: moaNone

proc auditAction(command: ModerationRestrictionCommand): ModerationAuditAction =
  case command
  of mrcBan: maaBan
  of mrcUnban: maaUnban
  of mrcTimeout: maaTimeout
  of mrcUntimeout: maaUntimeout

proc resolutionAuditAction(
    action: ModerationResolutionAction
): ModerationAuditAction =
  case action
  of mraDelete: maaResolveDelete
  of mraKick: maaResolveKick
  of mraBan: maaResolveBan
  of mraTimeout: maaResolveTimeout
  of mraDismiss: maaDismissReport
  of mraEscalate: maaEscalate

proc actorError(
    requestCommunity: string;
    roleCommunity: Option[string];
    actorRole: MembershipRole;
    restrictionCommunity: Option[string];
    restrictionExists: bool;
    banSet: bool;
    banExpiresAt: Option[int64];
    now: int64
): ModerationPolicyError =
  result = roleScopeError(requestCommunity, roleCommunity, actorRole)
  if result != moeNone:
    return
  result = restrictionScopeError(
    requestCommunity,
    restrictionCommunity,
    restrictionExists,
    banSet,
    banExpiresAt,
    none(int64),
  )
  if result != moeNone:
    return
  if banIsActive(banSet, banExpiresAt, now):
    return moeActorBanned
  if actorAuthority(actorRole) == moaNone:
    return moeNotAuthorized

proc decideModerationReport*(
    request: ModerationReportRequest
): ModerationDecision =
  let scoped = resourceScopeError(
    request.requestCommunity, request.targetCommunity, request.targetExists
  )
  if scoped != moeNone:
    return reject(scoped)
  if request.reporterIsTarget:
    return reject(moeSelfTarget)
  if request.duplicate:
    return reject(moeDuplicate)
  ModerationDecision(
    effect: meQueueReport,
    authority: moaReporter,
    auditAction: maaNone,
    error: moeNone,
  )

proc decideModerationRestriction*(
    request: ModerationRestrictionRequest
): ModerationDecision =
  if request.createdAtSeconds < 0 or request.nowSeconds < 0:
    return reject(moeFactConflict)
  if not commandIsFresh(request.createdAtSeconds, request.nowSeconds):
    return reject(moeStaleCommand)

  var scoped = actorError(
    request.requestCommunity,
    request.actorRoleCommunity,
    request.actorRole,
    request.actorRestrictionCommunity,
    request.actorRestrictionExists,
    request.actorBanSet,
    request.actorBanExpiresAt,
    request.nowSeconds,
  )
  if scoped != moeNone:
    return reject(scoped)
  scoped = roleScopeError(
    request.requestCommunity, request.targetRoleCommunity, request.targetRole
  )
  if scoped != moeNone:
    return reject(scoped)
  scoped = restrictionScopeError(
    request.requestCommunity,
    request.targetRestrictionCommunity,
    request.targetRestrictionExists,
    request.targetBanSet,
    request.targetBanExpiresAt,
    request.targetMutedUntil,
  )
  if scoped != moeNone:
    return reject(scoped)
  if request.actorIsTarget:
    return reject(moeSelfTarget)

  let authority = actorAuthority(request.actorRole)
  if authority == moaCommunityAdmin and
      request.command in {mrcBan, mrcTimeout} and
      request.targetRole in {mrOwner, mrAdmin}:
    return reject(moeProtectedTarget)

  let banned = banIsActive(
    request.targetBanSet, request.targetBanExpiresAt, request.nowSeconds
  )
  let timedOut = timeoutIsActive(request.targetMutedUntil, request.nowSeconds)
  var effect: ModerationEffect
  case request.command
  of mrcBan:
    if request.requestedExpiresAt.isSome and
        request.requestedExpiresAt.get() <= request.nowSeconds:
      return reject(moeExpirationElapsed)
    if banned:
      return reject(moeDuplicate)
    effect = meApplyBan
  of mrcUnban:
    if request.requestedExpiresAt.isSome:
      return reject(moeFactConflict)
    if not banned:
      return reject(moeNotBanned)
    effect = meLiftBan
  of mrcTimeout:
    if request.requestedExpiresAt.isNone:
      return reject(moeExpirationRequired)
    if request.requestedExpiresAt.get() <= request.nowSeconds:
      return reject(moeExpirationElapsed)
    if timedOut:
      return reject(moeDuplicate)
    effect = meApplyTimeout
  of mrcUntimeout:
    if request.requestedExpiresAt.isSome:
      return reject(moeFactConflict)
    if not timedOut:
      return reject(moeNotTimedOut)
    effect = meClearTimeout

  ModerationDecision(
    effect: effect,
    authority: authority,
    auditAction: auditAction(request.command),
    error: moeNone,
  )

proc decideModerationResolution*(
    request: ModerationResolutionRequest
): ModerationDecision =
  if request.createdAtSeconds < 0 or request.nowSeconds < 0:
    return reject(moeFactConflict)
  if not commandIsFresh(request.createdAtSeconds, request.nowSeconds):
    return reject(moeStaleCommand)

  var scoped = actorError(
    request.requestCommunity,
    request.actorRoleCommunity,
    request.actorRole,
    request.actorRestrictionCommunity,
    request.actorRestrictionExists,
    request.actorBanSet,
    request.actorBanExpiresAt,
    request.nowSeconds,
  )
  if scoped != moeNone:
    return reject(scoped)
  scoped = resourceScopeError(
    request.requestCommunity, request.reportCommunity, request.reportExists
  )
  if scoped != moeNone:
    return reject(scoped)
  if not request.reportOpen:
    return reject(moeReportClosed)
  if (request.action == mraDismiss) != (request.status == mrsDismissed):
    return reject(moeResolutionPair)

  ModerationDecision(
    effect: meResolveReport,
    authority: actorAuthority(request.actorRole),
    auditAction: resolutionAuditAction(request.action),
    error: moeNone,
  )

proc decideModerationEnforcement*(
    request: ModerationEnforcementRequest
): ModerationDecision =
  if request.nowSeconds < 0:
    return reject(moeFactConflict)

  var scoped = restrictionScopeError(
    request.requestCommunity,
    request.principalRestrictionCommunity,
    request.principalRestrictionExists,
    request.principalBanSet,
    request.principalBanExpiresAt,
    request.principalMutedUntil,
  )
  if scoped != moeNone:
    return reject(scoped)

  if not request.ownerAttested:
    if request.ownerRestrictionCommunity.isSome or
        request.ownerRestrictionExists or request.ownerBanSet or
        request.ownerBanExpiresAt.isSome:
      return reject(moeFactConflict)
  else:
    scoped = restrictionScopeError(
      request.requestCommunity,
      request.ownerRestrictionCommunity,
      request.ownerRestrictionExists,
      request.ownerBanSet,
      request.ownerBanExpiresAt,
      none(int64),
    )
    if scoped != moeNone:
      return reject(scoped)

  if banIsActive(
      request.principalBanSet,
      request.principalBanExpiresAt,
      request.nowSeconds,
    ) or
      (
        request.ownerAttested and
        banIsActive(
          request.ownerBanSet,
          request.ownerBanExpiresAt,
          request.nowSeconds,
        )
      ):
    return ModerationDecision(effect: meDenyBan, error: moeNone)
  if request.operation == meoWrite and
      timeoutIsActive(request.principalMutedUntil, request.nowSeconds):
    return ModerationDecision(effect: meDenyTimeout, error: moeNone)
  ModerationDecision(effect: meAllow, error: moeNone)
