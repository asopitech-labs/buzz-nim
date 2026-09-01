import std/options

const
  MinInviteTtlSeconds* = 60'i64
  MaxInviteTtlSeconds* = 2_592_000'i64
  MaxInviteUses* = 10_000

type
  MembershipRole* = enum
    mrNone
    mrOwner
    mrAdmin
    mrMember
    mrGuest
    mrBot

  RoleCapabilities* = object
    channelActive*: bool
    channelElevated*: bool
    relayMember*: bool
    relayAdmin*: bool
    relayOwner*: bool

  ChannelVisibility* = enum
    cvOpen
    cvPrivate

  MembershipCommand* = enum
    mcJoin
    mcAdd
    mcChangeRole
    mcRemove
    mcLeave

  AgentAddPolicy* = enum
    apAnyone
    apOwnerOnly
    apNobody

  InviteCommand* = enum
    icMint
    icClaim

  InviteState* = enum
    isValid
    isExpired
    isExhausted
    isInvalid

  MembershipAction* = enum
    maReject
    maInsert
    maUpdateRole
    maRemove
    maNoop
    maMint
    maJoin
    maTransfer

  MembershipPolicyError* = enum
    mpeNone
    mpeNotOperator
    mpeNotAuthorized
    mpeNotMember
    mpeInviteRequired
    mpeRoleInvalid
    mpeTargetMissing
    mpeLastOwner
    mpeSelfMutation
    mpeOwnerProtected
    mpeAgentAddDenied
    mpeInviteInvalid
    mpeInviteExpired
    mpeInviteExhausted
    mpePolicyRequired
    mpeMintBounds
    mpeOwnerMissing
    mpeOwnerConflict
    mpeOwnerLimit

  ChannelMembershipRequest* = object
    command*: MembershipCommand
    visibility*: ChannelVisibility
    actorRole*: MembershipRole
    targetRole*: MembershipRole
    requestedRole*: MembershipRole
    actorIsTarget*: bool
    actorOwnsTargetAgent*: bool
    targetIsAgent*: bool
    targetAddPolicy*: AgentAddPolicy
    ownerCount*: int

  RelayMembershipRequest* = object
    command*: MembershipCommand
    actorRole*: MembershipRole
    targetRole*: MembershipRole
    requestedRole*: MembershipRole
    actorIsTarget*: bool

  InvitePolicyRequest* = object
    command*: InviteCommand
    actorRole*: MembershipRole
    inviteState*: InviteState
    ttlSeconds*: int64
    maxUses*: Option[int]
    alreadyMember*: bool
    policyRequired*: bool
    policyAccepted*: bool

  OwnershipTransferRequest* = object
    actorIsOperator*: bool
    ownerPresent*: bool
    expectedOwnerMatches*: bool
    newOwnerIsCurrentOwner*: bool
    newOwnerAtLimit*: bool

  MembershipDecision* = object
    action*: MembershipAction
    error*: MembershipPolicyError
    effectiveRole*: MembershipRole

proc roleCapabilities*(role: MembershipRole): RoleCapabilities =
  case role
  of mrNone:
    RoleCapabilities()
  of mrOwner:
    RoleCapabilities(
      channelActive: true,
      channelElevated: true,
      relayMember: true,
      relayAdmin: true,
      relayOwner: true,
    )
  of mrAdmin:
    RoleCapabilities(
      channelActive: true,
      channelElevated: true,
      relayMember: true,
      relayAdmin: true,
    )
  of mrMember:
    RoleCapabilities(channelActive: true, relayMember: true)
  of mrGuest, mrBot:
    RoleCapabilities(channelActive: true)

proc reject(error: MembershipPolicyError): MembershipDecision =
  MembershipDecision(action: maReject, error: error)

proc requestedOrCurrent(request: ChannelMembershipRequest): MembershipRole =
  if request.requestedRole != mrNone:
    request.requestedRole
  elif request.targetRole != mrNone:
    request.targetRole
  else:
    mrMember

proc decideLeave(request: ChannelMembershipRequest): MembershipDecision =
  if not request.actorIsTarget:
    return reject(mpeSelfMutation)
  if request.targetRole == mrNone:
    return reject(mpeNotMember)
  if request.targetRole == mrOwner and request.ownerCount <= 1:
    return reject(mpeLastOwner)
  MembershipDecision(
    action: maRemove,
    error: mpeNone,
    effectiveRole: request.targetRole,
  )

proc decideChannelMembership*(
    request: ChannelMembershipRequest
): MembershipDecision =
  if request.actorIsTarget and request.actorRole != request.targetRole:
    return reject(mpeRoleInvalid)
  case request.command
  of mcJoin:
    if not request.actorIsTarget:
      return reject(mpeSelfMutation)
    if request.targetRole != mrNone:
      return MembershipDecision(
        action: maNoop,
        error: mpeNone,
        effectiveRole: request.targetRole,
      )
    if request.visibility == cvPrivate:
      return reject(mpeInviteRequired)
    MembershipDecision(action: maInsert, error: mpeNone,
        effectiveRole: mrMember)
  of mcLeave:
    decideLeave(request)
  of mcAdd:
    if request.actorIsTarget:
      return reject(mpeSelfMutation)
    if request.visibility == cvPrivate and
        not roleCapabilities(request.actorRole).channelActive:
      return reject(mpeNotAuthorized)
    if request.targetIsAgent:
      case request.targetAddPolicy
      of apNobody:
        return reject(mpeAgentAddDenied)
      of apOwnerOnly:
        if not request.actorOwnsTargetAgent:
          return reject(mpeAgentAddDenied)
      of apAnyone:
        discard

    let effectiveRole = requestedOrCurrent(request)
    if effectiveRole == mrNone:
      return reject(mpeRoleInvalid)
    if roleCapabilities(effectiveRole).channelElevated and
        not roleCapabilities(request.actorRole).channelElevated:
      return reject(mpeNotAuthorized)
    if request.targetRole != mrNone and effectiveRole != request.targetRole and
        not roleCapabilities(request.actorRole).channelElevated:
      return reject(mpeNotAuthorized)
    if request.targetRole == mrOwner and effectiveRole != mrOwner and
        request.ownerCount <= 1:
      return reject(mpeLastOwner)
    MembershipDecision(
      action:
      if request.targetRole == mrNone: maInsert
        elif effectiveRole == request.targetRole: maNoop
        else: maUpdateRole,
      error: mpeNone,
      effectiveRole: effectiveRole,
    )
  of mcChangeRole:
    if not roleCapabilities(request.actorRole).channelElevated:
      return reject(mpeNotAuthorized)
    if request.targetRole == mrNone:
      return reject(mpeTargetMissing)
    if request.requestedRole == mrNone:
      return reject(mpeRoleInvalid)
    if request.targetRole == mrOwner and request.requestedRole != mrOwner and
        request.ownerCount <= 1:
      return reject(mpeLastOwner)
    MembershipDecision(
      action:
      if request.targetRole == request.requestedRole: maNoop else: maUpdateRole,
      error: mpeNone,
      effectiveRole: request.requestedRole,
    )
  of mcRemove:
    if request.actorIsTarget:
      return decideLeave(request)
    let actor = roleCapabilities(request.actorRole)
    if not actor.channelActive:
      return reject(mpeNotAuthorized)
    if not actor.channelElevated and
        not (request.targetIsAgent and request.actorOwnsTargetAgent):
      return reject(mpeNotAuthorized)
    if request.targetRole == mrNone:
      return reject(mpeTargetMissing)
    if request.targetRole == mrOwner and request.ownerCount <= 1:
      return reject(mpeLastOwner)
    MembershipDecision(
      action: maRemove,
      error: mpeNone,
      effectiveRole: request.targetRole,
    )

proc decideRelayMembership*(
    request: RelayMembershipRequest
): MembershipDecision =
  if not roleCapabilities(request.actorRole).relayAdmin:
    return reject(mpeNotAuthorized)
  if request.actorIsTarget and request.actorRole != request.targetRole:
    return reject(mpeRoleInvalid)

  case request.command
  of mcAdd:
    let requestedRole =
      if request.requestedRole == mrNone: mrMember else: request.requestedRole
    if requestedRole notin {mrAdmin, mrMember}:
      return reject(mpeRoleInvalid)
    if request.actorRole == mrAdmin and requestedRole == mrAdmin:
      return reject(mpeNotAuthorized)
    MembershipDecision(
      action: if request.targetRole == mrNone: maInsert else: maNoop,
      error: mpeNone,
      effectiveRole:
      if request.targetRole == mrNone: requestedRole else: request.targetRole,
    )
  of mcRemove:
    if request.actorIsTarget:
      return reject(mpeSelfMutation)
    if request.targetRole == mrNone:
      return reject(mpeTargetMissing)
    if request.targetRole == mrOwner:
      return reject(mpeOwnerProtected)
    if request.targetRole notin {mrAdmin, mrMember}:
      return reject(mpeRoleInvalid)
    if request.actorRole == mrAdmin and request.targetRole != mrMember:
      return reject(mpeNotAuthorized)
    MembershipDecision(
      action: maRemove,
      error: mpeNone,
      effectiveRole: request.targetRole,
    )
  of mcChangeRole:
    if request.actorRole != mrOwner:
      return reject(mpeNotAuthorized)
    if request.actorIsTarget:
      return reject(mpeSelfMutation)
    if request.requestedRole notin {mrAdmin, mrMember}:
      return reject(mpeRoleInvalid)
    if request.targetRole == mrNone:
      return reject(mpeTargetMissing)
    if request.targetRole == mrOwner:
      return reject(mpeOwnerProtected)
    MembershipDecision(
      action:
      if request.targetRole == request.requestedRole: maNoop else: maUpdateRole,
      error: mpeNone,
      effectiveRole: request.requestedRole,
    )
  of mcJoin, mcLeave:
    reject(mpeRoleInvalid)

proc decideInvite*(request: InvitePolicyRequest): MembershipDecision =
  case request.command
  of icMint:
    if not roleCapabilities(request.actorRole).relayAdmin:
      return reject(mpeNotAuthorized)
    if request.ttlSeconds < MinInviteTtlSeconds or
        request.ttlSeconds > MaxInviteTtlSeconds:
      return reject(mpeMintBounds)
    if request.maxUses.isSome and
        (request.maxUses.get() < 1 or request.maxUses.get() > MaxInviteUses):
      return reject(mpeMintBounds)
    MembershipDecision(action: maMint, error: mpeNone)
  of icClaim:
    if request.policyRequired and not request.policyAccepted:
      return reject(mpePolicyRequired)
    case request.inviteState
    of isInvalid:
      reject(mpeInviteInvalid)
    of isExpired:
      reject(mpeInviteExpired)
    of isExhausted:
      if request.alreadyMember:
        MembershipDecision(action: maNoop, error: mpeNone,
            effectiveRole: mrMember)
      else:
        reject(mpeInviteExhausted)
    of isValid:
      MembershipDecision(
        action: if request.alreadyMember: maNoop else: maJoin,
        error: mpeNone,
        effectiveRole: mrMember,
      )

proc decideOwnershipTransfer*(
    request: OwnershipTransferRequest
): MembershipDecision =
  if not request.actorIsOperator:
    return reject(mpeNotOperator)
  if not request.ownerPresent:
    return reject(mpeOwnerMissing)
  if not request.expectedOwnerMatches:
    return reject(mpeOwnerConflict)
  if request.newOwnerIsCurrentOwner:
    return MembershipDecision(action: maNoop, error: mpeNone,
        effectiveRole: mrOwner)
  if request.newOwnerAtLimit:
    return reject(mpeOwnerLimit)
  MembershipDecision(action: maTransfer, error: mpeNone, effectiveRole: mrOwner)
