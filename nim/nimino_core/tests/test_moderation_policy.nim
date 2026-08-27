import std/[options, unittest]

from nimino_core/domain/membership_policy import
  MembershipRole, mrAdmin, mrMember, mrNone, mrOwner
import nimino_core/domain/moderation_policy

const
  CommunityA = "018f5e5a-9b7d-7c01-a7bb-46fbe46d0001"
  CommunityB = "018f5e5a-9b7d-7c01-a7bb-46fbe46d0002"
  Now = 1_800_000_000'i64

proc restriction(
    command: ModerationRestrictionCommand;
    actorRole = mrOwner;
    targetRole = mrMember;
    actorIsTarget = false;
    targetBanSet = false;
    targetBanExpiresAt = none(int64);
    targetMutedUntil = none(int64);
    requestedExpiresAt = none(int64)
): ModerationRestrictionRequest =
  ModerationRestrictionRequest(
    command: command,
    requestCommunity: CommunityA,
    actorRoleCommunity: some(CommunityA),
    targetRoleCommunity: some(CommunityA),
    actorRestrictionCommunity: none(string),
    targetRestrictionCommunity: some(CommunityA),
    actorRole: actorRole,
    targetRole: targetRole,
    actorRestrictionExists: false,
    actorBanSet: false,
    actorBanExpiresAt: none(int64),
    targetRestrictionExists: true,
    targetBanSet: targetBanSet,
    targetBanExpiresAt: targetBanExpiresAt,
    targetMutedUntil: targetMutedUntil,
    actorIsTarget: actorIsTarget,
    createdAtSeconds: Now,
    nowSeconds: Now,
    requestedExpiresAt: requestedExpiresAt,
  )

suite "Nimino moderation policy":
  test "reports require an exact target tenant and reject self or duplicate reports":
    let accepted = ModerationReportRequest(
      requestCommunity: CommunityA,
      targetCommunity: some(CommunityA),
      targetExists: true,
      reporterIsTarget: false,
      duplicate: false,
      targetKind: mrtEvent,
      reportType: mrtSpam,
    )
    check decideModerationReport(accepted).effect == meQueueReport

    var rejected = accepted
    rejected.targetCommunity = some(CommunityB)
    check decideModerationReport(rejected).error == moeTenantMismatch
    rejected = accepted
    rejected.reporterIsTarget = true
    check decideModerationReport(rejected).error == moeSelfTarget
    rejected = accepted
    rejected.duplicate = true
    check decideModerationReport(rejected).error == moeDuplicate

  test "owner and admin restriction authority protects privileged targets":
    check decideModerationRestriction(restriction(mrcBan)).effect == meApplyBan
    check decideModerationRestriction(
      restriction(mrcBan, actorRole = mrAdmin, targetRole = mrOwner)
    ).error == moeProtectedTarget
    check decideModerationRestriction(
      restriction(mrcBan, actorRole = mrMember)
    ).error == moeNotAuthorized
    check decideModerationRestriction(
      restriction(mrcBan, actorIsTarget = true)
    ).error == moeSelfTarget

  test "restriction transitions reject stale, duplicate, and elapsed expiry facts":
    var request = restriction(mrcTimeout, requestedExpiresAt = some(Now + 300))
    check decideModerationRestriction(request).effect == meApplyTimeout

    request.targetMutedUntil = some(Now + 60)
    check decideModerationRestriction(request).error == moeDuplicate
    request = restriction(mrcTimeout, requestedExpiresAt = some(Now - 1))
    check decideModerationRestriction(request).error == moeExpirationElapsed
    request = restriction(mrcBan)
    request.createdAtSeconds = Now - MaxModerationCommandSkewSeconds - 1
    check decideModerationRestriction(request).error == moeStaleCommand

  test "reversals require an active restriction after expiry evaluation":
    check decideModerationRestriction(
      restriction(mrcUnban, targetBanSet = true)
    ).effect == meLiftBan
    check decideModerationRestriction(
      restriction(
        mrcUnban,
        targetBanSet = true,
        targetBanExpiresAt = some(Now - 1),
      )
    ).error == moeNotBanned
    check decideModerationRestriction(
      restriction(mrcUntimeout, targetMutedUntil = some(Now + 60))
    ).effect == meClearTimeout

  test "report resolution is owner or admin only and closes an open scoped report":
    let accepted = ModerationResolutionRequest(
      requestCommunity: CommunityA,
      actorRoleCommunity: some(CommunityA),
      actorRestrictionCommunity: none(string),
      reportCommunity: some(CommunityA),
      actorRole: mrAdmin,
      actorRestrictionExists: false,
      actorBanSet: false,
      actorBanExpiresAt: none(int64),
      reportExists: true,
      reportOpen: true,
      createdAtSeconds: Now,
      nowSeconds: Now,
      status: mrsDismissed,
      action: mraDismiss,
    )
    let decision = decideModerationResolution(accepted)
    check decision.effect == meResolveReport
    check decision.auditAction == maaDismissReport

    var rejected = accepted
    rejected.reportOpen = false
    check decideModerationResolution(rejected).error == moeReportClosed
    rejected = accepted
    rejected.status = mrsResolved
    check decideModerationResolution(rejected).error == moeResolutionPair

  test "enforcement evaluates principal and attested-owner expiry in Nim":
    let clear = ModerationEnforcementRequest(
      operation: meoAuthenticate,
      requestCommunity: CommunityA,
      principalRestrictionCommunity: none(string),
      ownerRestrictionCommunity: none(string),
      principalRestrictionExists: false,
      principalBanSet: false,
      principalBanExpiresAt: none(int64),
      principalMutedUntil: none(int64),
      ownerAttested: false,
      ownerRestrictionExists: false,
      ownerBanSet: false,
      ownerBanExpiresAt: none(int64),
      nowSeconds: Now,
    )
    check decideModerationEnforcement(clear).effect == meAllow

    var restricted = clear
    restricted.principalRestrictionCommunity = some(CommunityA)
    restricted.principalRestrictionExists = true
    restricted.principalBanSet = true
    check decideModerationEnforcement(restricted).effect == meDenyBan
    restricted.principalBanExpiresAt = some(Now - 1)
    check decideModerationEnforcement(restricted).effect == meAllow

    restricted = clear
    restricted.operation = meoWrite
    restricted.principalRestrictionCommunity = some(CommunityA)
    restricted.principalRestrictionExists = true
    restricted.principalMutedUntil = some(Now + 60)
    check decideModerationEnforcement(restricted).effect == meDenyTimeout

    restricted = clear
    restricted.ownerAttested = true
    restricted.ownerRestrictionCommunity = some(CommunityA)
    restricted.ownerRestrictionExists = true
    restricted.ownerBanSet = true
    check decideModerationEnforcement(restricted).effect == meDenyBan
