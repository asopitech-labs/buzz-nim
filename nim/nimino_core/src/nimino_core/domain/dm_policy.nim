import std/options

const
  MinDmParticipants* = 2
  MaxDmParticipants* = 9

type
  DmCommand* = enum
    dcOpen
    dcAdd
    dcHide

  DmAction* = enum
    daReject
    daCreate
    daReuse
    daUnhide
    daHide
    daNoop

  DmAccessOperation* = enum
    daoRead
    daoWrite
    daoVisibility

  DmPolicyError* = enum
    dpeNone
    dpeResourceMissing
    dpeTenantMismatch
    dpeNotDm
    dpeNotParticipant
    dpeActorMissing
    dpeParticipantCount
    dpeNoNewParticipant
    dpeFactConflict
    dpeViewerMismatch

  DmMutationRequest* = object
    command*: DmCommand
    requestCommunity*: string
    sourceCommunity*: Option[string]
    destinationCommunity*: Option[string]
    sourceExists*: bool
    sourceIsDm*: bool
    actorIsSourceParticipant*: bool
    sourceActorHidden*: bool
    actorIncluded*: bool
    participantCount*: int
    newParticipantCount*: int
    destinationExists*: bool
    destinationIsDm*: bool
    destinationActorHidden*: bool

  DmMutationDecision* = object
    action*: DmAction
    error*: DmPolicyError

  DmAccessRequest* = object
    operation*: DmAccessOperation
    requestCommunity*: string
    resourceCommunity*: Option[string]
    resourceExists*: bool
    channelIsDm*: bool
    actorIsParticipant*: bool
    actorIsViewer*: bool

  DmAccessDecision* = object
    allowed*: bool
    error*: DmPolicyError

proc reject(error: DmPolicyError): DmMutationDecision =
  DmMutationDecision(action: daReject, error: error)

proc scopeError(
    requestCommunity: string;
    resourceCommunity: Option[string];
    resourceExists: bool
): DmPolicyError =
  if not resourceExists:
    return if resourceCommunity.isSome: dpeFactConflict else: dpeResourceMissing
  if resourceCommunity.isNone:
    return dpeResourceMissing
  if resourceCommunity.get() != requestCommunity:
    return dpeTenantMismatch
  dpeNone

proc destinationDecision(request: DmMutationRequest): DmMutationDecision =
  if not request.destinationExists:
    if request.destinationCommunity.isSome or request.destinationIsDm or
        request.destinationActorHidden:
      return reject(dpeFactConflict)
    return DmMutationDecision(action: daCreate, error: dpeNone)

  let scoped = scopeError(
    request.requestCommunity,
    request.destinationCommunity,
    request.destinationExists,
  )
  if scoped != dpeNone:
    return reject(scoped)
  if not request.destinationIsDm:
    return reject(dpeNotDm)
  DmMutationDecision(
    action: if request.destinationActorHidden: daUnhide else: daReuse,
    error: dpeNone,
  )

proc decideDmMutation*(request: DmMutationRequest): DmMutationDecision =
  case request.command
  of dcOpen:
    if request.sourceExists or request.sourceCommunity.isSome or
        request.sourceIsDm or request.actorIsSourceParticipant or
        request.sourceActorHidden:
      return reject(dpeFactConflict)
    if not request.actorIncluded:
      return reject(dpeActorMissing)
    if request.participantCount < MinDmParticipants or
        request.participantCount > MaxDmParticipants:
      return reject(dpeParticipantCount)
    destinationDecision(request)
  of dcAdd:
    let scoped = scopeError(
      request.requestCommunity,
      request.sourceCommunity,
      request.sourceExists,
    )
    if scoped != dpeNone:
      return reject(scoped)
    if not request.sourceIsDm:
      return reject(dpeNotDm)
    if not request.actorIsSourceParticipant:
      return reject(dpeNotParticipant)
    if not request.actorIncluded:
      return reject(dpeActorMissing)
    if request.newParticipantCount < 1:
      return reject(dpeNoNewParticipant)
    if request.participantCount <= MinDmParticipants or
        request.participantCount > MaxDmParticipants:
      return reject(dpeParticipantCount)
    destinationDecision(request)
  of dcHide:
    let scoped = scopeError(
      request.requestCommunity,
      request.sourceCommunity,
      request.sourceExists,
    )
    if scoped != dpeNone:
      return reject(scoped)
    if not request.sourceIsDm:
      return reject(dpeNotDm)
    if not request.actorIsSourceParticipant:
      return reject(dpeNotParticipant)
    DmMutationDecision(
      action: if request.sourceActorHidden: daNoop else: daHide,
      error: dpeNone,
    )

proc decideDmAccess*(request: DmAccessRequest): DmAccessDecision =
  let scoped = scopeError(
    request.requestCommunity,
    request.resourceCommunity,
    request.resourceExists,
  )
  if scoped != dpeNone:
    return DmAccessDecision(error: scoped)

  case request.operation
  of daoRead, daoWrite:
    if not request.channelIsDm:
      return DmAccessDecision(error: dpeNotDm)
    if not request.actorIsParticipant:
      return DmAccessDecision(error: dpeNotParticipant)
  of daoVisibility:
    if not request.actorIsViewer:
      return DmAccessDecision(error: dpeViewerMismatch)
  DmAccessDecision(allowed: true, error: dpeNone)
