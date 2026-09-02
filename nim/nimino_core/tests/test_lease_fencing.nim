import std/[options, unittest]

import nimino_core/domain/lease_fencing

proc authority(
    leader = "node-a"; term = 1'u64; epoch = 1'u64; quorum = true
): LeaseAuthority =
  LeaseAuthority(
    leaderId: leader,
    term: term,
    voterEpoch: epoch,
    quorumAvailable: quorum,
  )

proc commitFact(
    index: uint64; leader = "node-a"; term = 1'u64; epoch = 1'u64;
    clock = "boot-a"; now = 10'u64
): CommittedLeaseFact =
  CommittedLeaseFact(
    committed: true,
    controlIndex: index,
    leaderId: leader,
    term: term,
    voterEpoch: epoch,
    clockEpoch: clock,
    nowTick: now,
  )

proc servingFact(
    leader = "node-a"; term = 1'u64; epoch = 1'u64; quorum = true;
    clock = "boot-a"; now = 11'u64
): ServingLeaseFact =
  ServingLeaseFact(
    quorumAvailable: quorum,
    leaderId: leader,
    term: term,
    voterEpoch: epoch,
    clockEpoch: clock,
    nowTick: now,
  )

proc grant(
    state: LeaseState; transitionId: string; eligible: seq[string];
    controlIndex: uint64; leader = "node-a"; term = 1'u64; epoch = 1'u64;
    clock = "boot-a"; now = 10'u64; duration = 20'u64;
    mode = lamLive
): LeaseDecision =
  let proposed = planLeaseGrant(
    state,
    authority(leader, term, epoch),
    transitionId,
    eligible,
    duration,
  )
  doAssert proposed.error == lfeNone
  applyCommittedLease(
    state,
    proposed.command.get(),
    commitFact(controlIndex, leader, term, epoch, clock, now),
    mode,
  )

suite "Nimino lease, fencing, ownership, and routing":
  test "all singleton consumers receive typed fence and owner rejections":
    let initial = initLeaseState("workflow:daily")
    let granted = grant(initial, "grant-1", @["node-a"], 1)
    check granted.error == lfeNone

    let facts = servingFact()
    check authorizeSingletonEffect(
      granted.state,
      SingletonEffectAttempt(
        resourceId: "workflow:daily", ownerId: "node-a", fenceToken: 0
      ),
      facts,
    ).error == lfeStaleFence
    check authorizeSingletonEffect(
      granted.state,
      SingletonEffectAttempt(
        resourceId: "workflow:daily", ownerId: "node-a", fenceToken: 2
      ),
      facts,
    ).error == lfeFutureFence
    check authorizeSingletonEffect(
      granted.state,
      SingletonEffectAttempt(
        resourceId: "workflow:daily", ownerId: "node-b", fenceToken: 1
      ),
      facts,
    ).error == lfeOwnerMismatch
    check authorizeSingletonEffect(
      granted.state,
      SingletonEffectAttempt(
        resourceId: "workflow:daily", ownerId: "node-a", fenceToken: 1
      ),
      facts,
    ).allowed

  test "partition and lease expiry stop routing and side effects":
    let granted = grant(
      initLeaseState("deletion:tenant-a"), "grant-1", @["node-a"], 1
    )
    let partitioned = servingFact(quorum = false)
    check routeSingleton(granted.state, partitioned).error == lfeQuorumUnavailable
    check authorizeSingletonEffect(
      granted.state,
      SingletonEffectAttempt(
        resourceId: "deletion:tenant-a", ownerId: "node-a", fenceToken: 1
      ),
      partitioned,
    ).error == lfeQuorumUnavailable

    let expired = servingFact(now = 30)
    check routeSingleton(granted.state, expired).error == lfeLeaseExpired
    check not routeSingleton(granted.state, expired).allowed

  test "failover owner and fencing token are deterministic":
    let first = grant(
      initLeaseState("agent:singleton"),
      "grant-1",
      @["node-c", "node-a", "node-b"],
      1,
    )
    check first.state.activeLease.get().ownerId == "node-a"
    check first.state.lastFenceToken == 1

    let secondPlanA = planLeaseGrant(
      first.state,
      authority("node-b", 2, 1),
      "grant-2",
      @["node-c", "node-b"],
      20,
    )
    let secondPlanB = planLeaseGrant(
      first.state,
      authority("node-b", 2, 1),
      "grant-2",
      @["node-b", "node-c", "node-b"],
      20,
    )
    check secondPlanA == secondPlanB
    check secondPlanA.command.get().ownerId == "node-b"
    check secondPlanA.command.get().fenceToken == 2

    let failedOver = applyCommittedLease(
      first.state,
      secondPlanA.command.get(),
      commitFact(2, "node-b", 2, 1, "boot-a", 31),
      lamLive,
    )
    check failedOver.error == lfeNone
    check routeSingleton(
      failedOver.state,
      servingFact("node-b", 2, 1, true, "boot-a", 32),
    ).ownerId == "node-b"

  test "committed replay is idempotent and recovery never revives a lease":
    let initial = initLeaseState("projection:search")
    let plan = planLeaseGrant(
      initial,
      authority(),
      "grant-1",
      @["node-a"],
      20,
    )
    let command = plan.command.get()
    let applied = applyCommittedLease(
      initial, command, commitFact(1), lamLive
    )
    let duplicate = applyCommittedLease(
      applied.state, command, commitFact(1), lamLive
    )
    check duplicate.error == lfeNone
    check duplicate.effect == leReplay
    check duplicate.state == applied.state

    let recovered = applyCommittedLease(
      initial,
      command,
      commitFact(1, clock = "boot-after-restart"),
      lamRecovery,
    )
    check recovered.error == lfeNone
    check recovered.state.activeLease.isNone
    check recovered.state.lastFenceToken == 1
    check routeSingleton(
      recovered.state,
      servingFact(clock = "boot-after-restart"),
    ).error == lfeNoActiveLease

    let next = planLeaseGrant(
      recovered.state,
      authority(term = 2),
      "grant-2",
      @["node-a"],
      20,
    )
    check next.command.get().fenceToken == 2

  test "uncommitted and stale authority facts fail closed":
    let initial = initLeaseState("media:gc")
    let command = planLeaseGrant(
      initial, authority(), "grant-1", @["node-a"], 20
    ).command.get()
    var notCommitted = commitFact(1)
    notCommitted.committed = false
    check applyCommittedLease(
      initial, command, notCommitted, lamLive
    ).error == lfeControlNotCommitted

    let granted = applyCommittedLease(
      initial, command, commitFact(1), lamLive
    )
    check routeSingleton(
      granted.state, servingFact(term = 2)
    ).error == lfeAuthorityStale
    check routeSingleton(
      granted.state, servingFact(epoch = 2)
    ).error == lfeAuthorityStale
