import std/unittest

import nimino_core/domain/cluster_lifecycle

proc request(
    command: LifecycleCommand; state: ClusterNodeState
): LifecycleTransitionRequest =
  LifecycleTransitionRequest(
    command: command,
    currentState: state,
    authenticated: true,
    revoked: false,
    identityUnique: true,
    productCapability: RequiredProductCapability,
    controlProtocolVersion: RequiredControlProtocolVersion,
    dataProtocolVersion: RequiredDataProtocolVersion,
    controlDecisionCommitted: true,
    snapshotInstalled: true,
    checkpointMatches: true,
    requiredVoterEpoch: 1,
    observedVoterEpoch: 1,
    activeWork: 0,
  )

suite "Nimino cluster lifecycle policy":
  test "join rejects unauthenticated revoked duplicate and incompatible nodes":
    var candidate = request(lcJoin, cnsOffline)
    candidate.authenticated = false
    check decideLifecycleTransition(candidate).error == cleUnauthenticated

    candidate = request(lcJoin, cnsOffline)
    candidate.revoked = true
    check decideLifecycleTransition(candidate).error == cleRevoked

    candidate = request(lcJoin, cnsOffline)
    candidate.identityUnique = false
    check decideLifecycleTransition(candidate).error == cleIdentityConflict

    candidate = request(lcJoin, cnsOffline)
    candidate.productCapability = "other-v1"
    check decideLifecycleTransition(candidate).error == cleCapabilityMismatch

    candidate = request(lcJoin, cnsOffline)
    candidate.controlProtocolVersion += 1
    check decideLifecycleTransition(candidate).error == cleControlVersionMismatch

    candidate = request(lcJoin, cnsOffline)
    candidate.dataProtocolVersion += 1
    check decideLifecycleTransition(candidate).error == cleDataVersionMismatch

  test "state transitions cannot skip and readiness requires synchronized facts":
    check decideLifecycleTransition(
      request(lcMarkReady, cnsOffline)
    ).error == cleInvalidTransition

    var readiness = request(lcMarkReady, cnsSyncing)
    readiness.snapshotInstalled = false
    check decideLifecycleTransition(readiness).error == cleSyncIncomplete

    readiness = request(lcMarkReady, cnsSyncing)
    readiness.observedVoterEpoch = 2
    check decideLifecycleTransition(readiness).error == cleEpochMismatch

    readiness = request(lcMarkReady, cnsSyncing)
    readiness.revoked = true
    check decideLifecycleTransition(readiness).error == cleRevoked

  test "join drain and rejoin follow every committed state":
    var state = cnsOffline
    let steps: array[6, tuple[
      command: LifecycleCommand; expected: ClusterNodeState
    ]] = [
      (lcJoin, cnsJoining),
      (lcStartSync, cnsSyncing),
      (lcMarkReady, cnsReady),
      (lcBeginDrain, cnsDraining),
      (lcMarkOffline, cnsOffline),
      (lcJoin, cnsJoining),
    ]
    for step in steps:
      let decision = decideLifecycleTransition(request(step.command, state))
      check decision.error == cleNone
      check decision.nextState == step.expected
      state = decision.nextState

  test "uncommitted transitions and active drain work fail closed":
    var transition = request(lcStartSync, cnsJoining)
    transition.controlDecisionCommitted = false
    check decideLifecycleTransition(transition).error == cleTransitionUncommitted

    transition = request(lcMarkOffline, cnsDraining)
    transition.activeWork = 1
    check decideLifecycleTransition(transition).error == cleDrainIncomplete

  test "pre-ready and draining nodes cannot serve client or lease lanes":
    for state in [cnsJoining, cnsSyncing, cnsDraining, cnsOffline]:
      for lane in [clClientRead, clClientWrite, clLease]:
        let decision = decideClusterLane(ClusterLaneRequest(state: state, lane: lane))
        check decision.effect == leDenyLane
        check decision.error == cleLaneNotAllowed

    for lane in ClusterLane:
      check decideClusterLane(
        ClusterLaneRequest(state: cnsReady, lane: lane)
      ).effect == leAllowLane
