import std/unittest

import nimino_core/domain/repair_policy

const
  Canonical = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  Projection = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  Objects = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
  Corrupt = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"

proc fact(
    nodeId: string; canonical = Canonical; projection = Projection;
    objects = Objects; readable = true; unknown = 0'u32
): ReplicaHealthFact =
  ReplicaHealthFact(
    communityId: "community-a",
    nodeId: nodeId,
    readable: readable,
    checkpoint: 3_005,
    canonicalDigest: canonical,
    projectionDigest: projection,
    objectDigest: objects,
    unknownEffects: unknown,
  )

proc isolated(): RepairState =
  settleRepairPlan(
    planRepairVerification(
      initRepairState("community-a", "node-c"),
      @[fact("node-a"), fact("node-b"), fact("node-c", canonical = Corrupt)],
    ),
    true,
  ).state

proc request(authorized = true): RepairRequest =
  RepairRequest(
    communityId: "community-a",
    targetNodeId: "node-c",
    sourceNodeId: "node-a",
    repairId: "repair-1",
    operatorAuthorized: authorized,
    operatorId: "operator-a",
    reason: "replace quarantined replica from quorum source",
  )

suite "Nimino data verify and repair policy":
  test "healthy replicas verify and replay idempotently":
    let initial = initRepairState("community-a", "node-a")
    let verified = settleRepairPlan(
      planRepairVerification(initial, @[fact("node-a")]), true
    )
    check verified.effect == reVerified
    check verified.state.phase == rpComplete
    check planRepairVerification(
      verified.state, @[fact("node-a")]
    ).effect == reReplay

  test "quorum isolates corruption and manual repair is idempotent":
    let initial = initRepairState("community-a", "node-c")
    let facts =
      @[fact("node-a"), fact("node-b"), fact("node-c", canonical = Corrupt)]
    let failedIsolation = settleRepairPlan(
      planRepairVerification(initial, facts), false
    )
    check failedIsolation.error == rpePersistenceFailure
    check failedIsolation.state == initial
    let quarantined = isolated()
    check quarantined.phase == rpQuarantined
    check quarantined.sourceNodeId == "node-a"
    check planRepairVerification(
      quarantined,
      @[fact("node-a"), fact("node-b"), fact("node-c", canonical = Corrupt)],
    ).effect == reReplay
    check planManualRepair(
      quarantined, request(authorized = false)
    ).error == rpeOperatorUnauthorized

    let repairPlan = planManualRepair(quarantined, request())
    check repairPlan.portEffect == rpoCommitRepairMarker
    let repair = settleRepairPlan(repairPlan, true)
    check repair.effect == reBeginRepair
    check repair.replaceCanonical
    check repair.rebuildProjections
    check repair.repairObjects

    let applied = settleRepairAdapters(repair.state, true)
    check applied.effect == reVerifyRepair
    let verified = settleRepairPlan(
      planRepairVerification(
        applied.state, @[fact("node-a"), fact("node-b"), fact("node-c")]
      ),
      true,
    )
    check verified.state.phase == rpComplete
    check planRepairVerification(
      verified.state, @[fact("node-a"), fact("node-b"), fact("node-c")]
    ).effect == reReplay

  test "disk full and interrupted batches never publish repaired state":
    let quarantined = isolated()
    let plan = planManualRepair(quarantined, request())
    let markerFailure = settleRepairPlan(plan, false)
    check markerFailure.error == rpePersistenceFailure
    check markerFailure.state == quarantined

    let repairing = settleRepairPlan(plan, true).state
    let diskFull = settleRepairAdapters(repairing, false)
    check diskFull.error == rpeAdapterFailure
    check diskFull.state.phase == rpQuarantined
    let killed = recoverInterruptedRepair(repairing)
    check killed.effect == reRepairFailed
    check killed.state.phase == rpQuarantined
    check killed.state.canonicalDigest == Canonical

  test "corrupt chunks and unknown effects fail closed":
    let noQuorum = planRepairVerification(
      initRepairState("community-a", "node-c"),
      @[fact("node-a"), fact("node-b", canonical = Corrupt), fact("node-c",
          projection = Corrupt)],
    )
    check noQuorum.error == rpeNoQuorum

    let blocked = settleRepairPlan(
      planRepairVerification(
        initRepairState("community-a", "node-a"),
        @[fact("node-a", unknown = 1)],
      ),
      true,
    )
    check blocked.effect == reManualEffectReconcile
    check blocked.state.phase == rpBlockedEffects
