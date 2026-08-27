import std/[options, strutils, unittest]

import nimino_core/domain/[effect_ledger, lease_fencing]

const Digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

proc authority(quorum = true): LeaseAuthority =
  LeaseAuthority(
    leaderId: "node-a", term: 1, voterEpoch: 1, quorumAvailable: quorum
  )

proc serving(quorum = true; now = 11'u64): ServingLeaseFact =
  ServingLeaseFact(
    quorumAvailable: quorum,
    leaderId: "node-a",
    term: 1,
    voterEpoch: 1,
    clockEpoch: "boot-a",
    nowTick: now,
  )

proc lease(
    owner = "node-a"; transition = "grant-1"; fence = 1'u64;
    mode = lamLive
): LeaseState =
  let initial = initLeaseState("workflow:daily")
  let plan = planLeaseGrant(
    initial, authority(), transition, @[owner], 20
  )
  doAssert plan.error == lfeNone
  let command = plan.command.get()
  doAssert command.fenceToken == fence
  result = applyCommittedLease(
    initial,
    command,
    CommittedLeaseFact(
      committed: true,
      controlIndex: fence,
      leaderId: "node-a",
      term: 1,
      voterEpoch: 1,
      clockEpoch: "boot-a",
      nowTick: 10,
    ),
    mode,
  ).state

proc initial(): EffectLedgerState =
  initEffectLedger(
    "community-a",
    "daily",
    "run-1",
    "notify",
    "nimino:community-a:run-1:notify",
    Digest,
    "workflow:daily",
  )

proc claimed(
    state: EffectLedgerState; currentLease: LeaseState
): EffectLedgerState =
  settleEffectLedger(
    planEffectClaim(state, "node-a", currentLease.lastFenceToken, currentLease, serving()),
    true,
  ).state

proc executing(
    state: EffectLedgerState; currentLease: LeaseState
): EffectLedgerState =
  settleEffectLedger(
    planEffectExecution(
      state, "node-a", currentLease.lastFenceToken, currentLease, serving()
    ),
    true,
  ).state

proc receipt(outcome = eroSucceeded; id = "provider-123"): EffectReceipt =
  EffectReceipt(outcome: outcome, receiptId: id, resultDigest: Digest)

suite "Nimino workflow effect ledger":
  test "persisted identities are bounded at the trust boundary":
    check initial().valid
    check not initEffectLedger(
      "community-a", "daily", "run-1", "notify", repeat('k', 257), Digest,
      "workflow:daily",
    ).valid
    var corrupt = initial()
    corrupt.status = elsClaimed
    corrupt.attempt = 1
    corrupt.ownerNodeId = repeat('n', 257)
    corrupt.fenceToken = 1
    check planEffectRecovery(
      corrupt, initLeaseState("workflow:daily"), serving()
    ).error == eleInvalidState

  test "quorum lease gates claim and durable execution marker":
    let currentLease = lease()
    let noQuorum = planEffectClaim(
      initial(), "node-a", 1, currentLease, serving(quorum = false)
    )
    check noQuorum.error == eleLeaseRejected
    check noQuorum.leaseError == lfeQuorumUnavailable

    let claimPlan = planEffectClaim(
      initial(), "node-a", 1, currentLease, serving()
    )
    check claimPlan.portEffect == elpCommitCanonical
    let claim = settleEffectLedger(claimPlan, true)
    check claim.state.status == elsClaimed
    check claim.state.attempt == 1

    let expired = planEffectExecution(
      claim.state, "node-a", 1, currentLease, serving(now = 30)
    )
    check expired.error == eleLeaseRejected
    check expired.leaseError == lfeLeaseExpired

    let executePlan = planEffectExecution(
      claim.state, "node-a", 1, currentLease, serving()
    )
    let failedMarker = settleEffectLedger(executePlan, false)
    check failedMarker.effect == efeReject
    check failedMarker.state == claim.state
    let execute = settleEffectLedger(executePlan, true)
    check execute.effect == efeExecuteExternal
    check execute.state.status == elsExecuting

  test "crash before marker is reclaimable but any marked attempt is unknown":
    let currentLease = lease()
    let claim = claimed(initial(), currentLease)
    let partitionedRecovery = planEffectRecovery(
      claim, currentLease, serving(quorum = false)
    )
    check partitionedRecovery.error == eleLeaseRejected
    check partitionedRecovery.leaseError == lfeQuorumUnavailable
    let recoveredClaim = settleEffectLedger(
      planEffectRecovery(claim, lease(mode = lamRecovery), serving()), true
    )
    check recoveredClaim.effect == efeClaimRecovered
    check recoveredClaim.state.status == elsPending
    check recoveredClaim.state.idempotencyKey == claim.idempotencyKey

    let execute = executing(claim, currentLease)
    let recoveredExecution = settleEffectLedger(
      planEffectRecovery(execute, initLeaseState("workflow:daily"), serving()),
      true,
    )
    check recoveredExecution.effect == efeUnknown
    check recoveredExecution.state.status == elsUnknown
    check planEffectClaim(
      recoveredExecution.state, "node-a", 1, currentLease, serving()
    ).error == eleManualReconcileRequired
    check planEffectExecution(
      recoveredExecution.state, "node-a", 1, currentLease, serving()
    ).error == eleManualReconcileRequired

  test "duplicate claims and receipts never duplicate external execution":
    let currentLease = lease()
    let claim = claimed(initial(), currentLease)
    check planEffectClaim(
      claim, "node-a", 1, currentLease, serving()
    ).effect == efeReplay
    check planEffectClaim(
      claim, "node-b", 1, currentLease, serving()
    ).error == eleClaimConflict

    let execute = executing(claim, currentLease)
    let stored = settleEffectLedger(
      planEffectReceipt(execute, "node-a", 1, receipt()), true
    ).state
    check stored.status == elsSucceeded
    check stored.idempotencyKey == "nimino:community-a:run-1:notify"
    check stored.receipt.get().receiptId == "provider-123"
    check planEffectReceipt(
      stored, "node-a", 1, receipt()
    ).effect == efeReplay
    check planEffectReceipt(
      stored, "node-a", 1, receipt(id = "different")
    ).error == eleTerminalConflict

  test "unknown requires authorized manual reconciliation":
    let currentLease = lease()
    let unknown = settleEffectLedger(
      planEffectRecovery(executing(claimed(initial(), currentLease), currentLease),
        initLeaseState("workflow:daily"), serving()),
      true,
    ).state
    var reconcile = EffectReconcileRequest(
      operatorAuthorized: false,
      operatorId: "operator-a",
      reason: "provider lookup",
      command: ercMarkSucceeded,
      receipt: some(receipt()),
    )
    check planEffectReconcile(unknown, reconcile).error == eleReconcileUnauthorized
    reconcile.operatorAuthorized = true
    let resolved = settleEffectLedger(
      planEffectReconcile(unknown, reconcile), true
    )
    check resolved.effect == efeReconciled
    check resolved.state.status == elsSucceeded
    check resolved.state.reconciledBy == "operator-a"

    reconcile.command = ercRetry
    reconcile.receipt = none(EffectReceipt)
    let retried = settleEffectLedger(
      planEffectReconcile(unknown, reconcile), true
    )
    check retried.effect == efeManualRetry
    check retried.state.status == elsPending
    check retried.state.attempt == 1
    check retried.state.idempotencyKey == unknown.idempotencyKey
