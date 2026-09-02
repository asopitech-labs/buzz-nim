const
  RequiredProductCapability* = "nimino-v1"
  RequiredControlProtocolVersion* = 1
  RequiredDataProtocolVersion* = 1

type
  ClusterNodeState* = enum
    cnsOffline
    cnsJoining
    cnsSyncing
    cnsReady
    cnsDraining

  LifecycleCommand* = enum
    lcJoin
    lcStartSync
    lcMarkReady
    lcBeginDrain
    lcMarkOffline

  ClusterLane* = enum
    clNegotiation
    clControl
    clDataSync
    clClientRead
    clClientWrite
    clLease

  LifecycleEffect* = enum
    leReject
    leEnterJoining
    leEnterSyncing
    leEnterReady
    leEnterDraining
    leEnterOffline
    leAllowLane
    leDenyLane

  ClusterLifecycleError* = enum
    cleNone
    cleUnauthenticated
    cleRevoked
    cleIdentityConflict
    cleCapabilityMismatch
    cleControlVersionMismatch
    cleDataVersionMismatch
    cleInvalidTransition
    cleTransitionUncommitted
    cleSyncIncomplete
    cleEpochMismatch
    cleDrainIncomplete
    cleLaneNotAllowed
    cleFactConflict

  LifecycleTransitionRequest* = object
    command*: LifecycleCommand
    currentState*: ClusterNodeState
    authenticated*: bool
    revoked*: bool
    identityUnique*: bool
    productCapability*: string
    controlProtocolVersion*: int
    dataProtocolVersion*: int
    controlDecisionCommitted*: bool
    snapshotInstalled*: bool
    checkpointMatches*: bool
    requiredVoterEpoch*: int64
    observedVoterEpoch*: int64
    activeWork*: int

  ClusterLaneRequest* = object
    state*: ClusterNodeState
    lane*: ClusterLane

  ClusterLifecycleDecision* = object
    effect*: LifecycleEffect
    nextState*: ClusterNodeState
    error*: ClusterLifecycleError

proc reject(
    state: ClusterNodeState; error: ClusterLifecycleError
): ClusterLifecycleDecision =
  ClusterLifecycleDecision(effect: leReject, nextState: state, error: error)

proc expectedState(command: LifecycleCommand): ClusterNodeState =
  case command
  of lcJoin: cnsOffline
  of lcStartSync: cnsJoining
  of lcMarkReady: cnsSyncing
  of lcBeginDrain: cnsReady
  of lcMarkOffline: cnsDraining

proc transition(
    state: ClusterNodeState; effect: LifecycleEffect
): ClusterLifecycleDecision =
  ClusterLifecycleDecision(effect: effect, nextState: state, error: cleNone)

proc admissionError(
    request: LifecycleTransitionRequest
): ClusterLifecycleError =
  if not request.authenticated:
    return cleUnauthenticated
  if request.revoked:
    return cleRevoked
  if not request.identityUnique:
    return cleIdentityConflict
  if request.productCapability != RequiredProductCapability:
    return cleCapabilityMismatch
  if request.controlProtocolVersion != RequiredControlProtocolVersion:
    return cleControlVersionMismatch
  if request.dataProtocolVersion != RequiredDataProtocolVersion:
    return cleDataVersionMismatch
  cleNone

proc decideLifecycleTransition*(
    request: LifecycleTransitionRequest
): ClusterLifecycleDecision =
  if request.controlProtocolVersion < 0 or request.dataProtocolVersion < 0 or
      request.requiredVoterEpoch < 0 or request.observedVoterEpoch < 0 or
      request.activeWork < 0:
    return reject(request.currentState, cleFactConflict)
  if request.currentState != expectedState(request.command):
    return reject(request.currentState, cleInvalidTransition)
  if request.command in {lcJoin, lcStartSync, lcMarkReady}:
    let admission = admissionError(request)
    if admission != cleNone:
      return reject(request.currentState, admission)

  case request.command
  of lcJoin:
    transition(cnsJoining, leEnterJoining)
  of lcStartSync:
    if not request.controlDecisionCommitted:
      return reject(request.currentState, cleTransitionUncommitted)
    transition(cnsSyncing, leEnterSyncing)
  of lcMarkReady:
    if not request.controlDecisionCommitted:
      return reject(request.currentState, cleTransitionUncommitted)
    if not request.snapshotInstalled or not request.checkpointMatches:
      return reject(request.currentState, cleSyncIncomplete)
    if request.requiredVoterEpoch == 0 or request.observedVoterEpoch == 0:
      return reject(request.currentState, cleFactConflict)
    if request.requiredVoterEpoch != request.observedVoterEpoch:
      return reject(request.currentState, cleEpochMismatch)
    transition(cnsReady, leEnterReady)
  of lcBeginDrain:
    if not request.controlDecisionCommitted:
      return reject(request.currentState, cleTransitionUncommitted)
    transition(cnsDraining, leEnterDraining)
  of lcMarkOffline:
    if not request.controlDecisionCommitted:
      return reject(request.currentState, cleTransitionUncommitted)
    if request.activeWork != 0 or not request.checkpointMatches:
      return reject(request.currentState, cleDrainIncomplete)
    transition(cnsOffline, leEnterOffline)

proc decideClusterLane*(request: ClusterLaneRequest): ClusterLifecycleDecision =
  let allowed = case request.state
    of cnsOffline: false
    of cnsJoining: request.lane == clNegotiation
    of cnsSyncing, cnsDraining:
      request.lane in {clNegotiation, clControl, clDataSync}
    of cnsReady: true
  if allowed:
    return transition(request.state, leAllowLane)
  ClusterLifecycleDecision(
    effect: leDenyLane,
    nextState: request.state,
    error: cleLaneNotAllowed,
  )
