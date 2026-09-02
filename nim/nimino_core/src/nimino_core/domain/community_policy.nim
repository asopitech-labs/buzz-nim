import std/options

type
  CommunityState* = enum
    csMissing
    csActive
    csArchived
    csDeleting
    csDeleted

  CommunityCommand* = enum
    ccCreate
    ccArchive
    ccUnarchive

  CommunityAction* = enum
    caReject
    caCreate
    caArchive
    caUnarchive
    caNoop

  CommunityPolicyError* = enum
    cpeNone
    cpeNotOperator
    cpeNotOwner
    cpeOwnerRequired
    cpeOwnerLimit
    cpeHostReserved
    cpeProtectedCommunity
    cpeInvalidState
    cpeResourceMissing
    cpeTenantMismatch

  CommunityLifecycleRequest* = object
    command*: CommunityCommand
    state*: CommunityState
    actorIsOperator*: bool
    actorIsOwner*: bool
    ownerProvided*: bool
    ownerAtLimit*: bool
    protectedDeployment*: bool

  CommunityLifecycleDecision* = object
    action*: CommunityAction
    error*: CommunityPolicyError

  CommunityScopeRequest* = object
    requestCommunity*: string
    resourceCommunity*: Option[string]

  CommunityScopeDecision* = object
    allowed*: bool
    error*: CommunityPolicyError

proc reject(error: CommunityPolicyError): CommunityLifecycleDecision =
  CommunityLifecycleDecision(action: caReject, error: error)

proc decideLifecycle*(request: CommunityLifecycleRequest): CommunityLifecycleDecision =
  if not request.actorIsOperator:
    return reject(cpeNotOperator)
  if request.protectedDeployment:
    return reject(cpeProtectedCommunity)

  case request.command
  of ccCreate:
    if not request.ownerProvided:
      return reject(cpeOwnerRequired)
    if request.state != csMissing:
      return reject(cpeHostReserved)
    if request.ownerAtLimit:
      return reject(cpeOwnerLimit)
    CommunityLifecycleDecision(action: caCreate, error: cpeNone)
  of ccArchive, ccUnarchive:
    if not request.actorIsOwner:
      return reject(cpeNotOwner)
    case request.state
    of csActive:
      CommunityLifecycleDecision(
        action: if request.command == ccArchive: caArchive else: caNoop,
        error: cpeNone,
      )
    of csArchived:
      CommunityLifecycleDecision(
        action: if request.command == ccUnarchive: caUnarchive else: caNoop,
        error: cpeNone,
      )
    of csMissing, csDeleting, csDeleted:
      reject(cpeInvalidState)

proc decideScope*(request: CommunityScopeRequest): CommunityScopeDecision =
  if request.resourceCommunity.isNone:
    return CommunityScopeDecision(error: cpeResourceMissing)
  if request.requestCommunity != request.resourceCommunity.get():
    return CommunityScopeDecision(error: cpeTenantMismatch)
  CommunityScopeDecision(allowed: true, error: cpeNone)
