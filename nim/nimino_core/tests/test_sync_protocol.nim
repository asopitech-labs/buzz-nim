import std/[options, unittest]

import nimino_core/domain/sync_protocol

const
  D0 = "0000000000000000000000000000000000000000000000000000000000000000"
  D2 = "2222222222222222222222222222222222222222222222222222222222222222"
  D4 = "4444444444444444444444444444444444444444444444444444444444444444"

proc envelope(sender, receiver: string; community = "community-a"): SyncEnvelope =
  SyncEnvelope(
    protocol: SyncProtocolName,
    version: SyncProtocolVersion,
    sessionId: "session-1",
    communityId: community,
    senderNodeId: sender,
    receiverNodeId: receiver,
  )

proc digestFrame(
    receiver: string; checkpoint = 4'u64; digest = D4;
    community = "community-a"
): DigestFrame =
  DigestFrame(
    envelope: envelope("node-a", receiver, community),
    checkpoint: checkpoint,
    prefixDigest: digest,
  )

proc record(sequence: uint64): SyncRecord =
  SyncRecord(
    sequence: sequence,
    recordType: "event",
    key: "event-" & $sequence,
    payload: "{}",
    contentDigest: D2,
  )

proc batch(
    receiver: string; first, last: uint64; baseDigest, resultDigest: string;
    community = "community-a"; verified = true
): RangeBatchFrame =
  var records: seq[SyncRecord]
  for sequence in first .. last:
    records.add(record(sequence))
  RangeBatchFrame(
    envelope: envelope("node-a", receiver, community),
    batchId: "batch-" & $first & "-" & $last,
    baseCheckpoint: first - 1,
    baseDigest: baseDigest,
    throughCheckpoint: last,
    resultDigest: resultDigest,
    encodedBytes: uint32(records.len * 2),
    digestVerified: verified,
    records: records,
  )

proc initial(node: string; checkpoint = 0'u64; digest = D0): SyncState =
  initSyncState(
    "session-1",
    "community-a",
    node,
    "node-a",
    checkpoint,
    digest,
    2,
    128,
    10,
  )

proc applyBatch(
    state: SyncState; frame: RangeBatchFrame; nowTick: uint64
): SyncDecision =
  let plan = planRangeBatch(state, frame, nowTick)
  doAssert plan.error == seNone
  settleRangeBatch(
    plan,
    plan.inflightState,
    true,
    frame.throughCheckpoint,
  )

suite "Nimino bounded anti-entropy sync":
  test "one source bootstraps two peers in bounded ranges":
    var nodeB = acceptRemoteDigest(initial("node-b"), digestFrame("node-b"), 0).state
    var nodeC = acceptRemoteDigest(initial("node-c"), digestFrame("node-c"), 0).state

    for target in ["node-b", "node-c"]:
      let state = if target == "node-b": nodeB else: nodeC
      let request = nextRangeRequest(state).get()
      check request.afterCheckpoint == 0
      check request.limitRecords == 2
      check request.maxEncodedBytes == 128
      check planRangeRead(
        request,
        "session-1",
        "community-a",
        "node-a",
        target,
        4,
      ).allowed

    nodeB = applyBatch(nodeB, batch("node-b", 1, 2, D0, D2), 1).state
    nodeC = applyBatch(nodeC, batch("node-c", 1, 2, D0, D2), 1).state
    check nextRangeRequest(nodeB).get().afterCheckpoint == 2
    check nextRangeRequest(nodeC).get().afterCheckpoint == 2

    nodeB = applyBatch(nodeB, batch("node-b", 3, 4, D2, D4), 2).state
    nodeC = applyBatch(nodeC, batch("node-c", 3, 4, D2, D4), 2).state
    check nodeB.phase == spComplete
    check nodeC.phase == spComplete
    check nodeB.checkpoint == 4
    check nodeC.checkpointDigest == D4

  test "durable canonical checkpoint resumes after restart":
    var state = acceptRemoteDigest(initial("node-b"), digestFrame("node-b"), 0).state
    state = applyBatch(state, batch("node-b", 1, 2, D0, D2), 1).state

    let restarted = initial("node-b", state.checkpoint, state.checkpointDigest)
    let resumed = acceptRemoteDigest(restarted, digestFrame("node-b"), 20)
    check resumed.error == seNone
    check resumed.state.phase == spWaitingBatch
    check nextRangeRequest(resumed.state).get().afterCheckpoint == 2

  test "slow peers time out and cancellation wins an apply race":
    let waiting = acceptRemoteDigest(initial("node-b"), digestFrame("node-b"), 5).state
    check checkSyncDeadline(waiting, 14).state == waiting
    let timedOut = checkSyncDeadline(waiting, 15)
    check timedOut.error == sePeerTimeout
    check timedOut.state.phase == spCancelled

    let plan = planRangeBatch(
      acceptRemoteDigest(initial("node-b"), digestFrame("node-b"), 0).state,
      batch("node-b", 1, 2, D0, D2),
      1,
    )
    let cancelled = stopSync(plan.inflightState, "operator stop")
    check cancelled.state.phase == spCancelled
    let lateStore = settleRangeBatch(
      plan, cancelled.state, true, 2
    )
    check lateStore.error == seCancelled
    check lateStore.state.phase == spCancelled

  test "one in-flight batch enforces backpressure and duplicates are idempotent":
    let waiting = acceptRemoteDigest(initial("node-b"), digestFrame("node-b"), 0).state
    let firstFrame = batch("node-b", 1, 2, D0, D2)
    let firstPlan = planRangeBatch(waiting, firstFrame, 1)
    check firstPlan.inflightState.phase == spApplying
    check planRangeBatch(
      firstPlan.inflightState,
      batch("node-b", 3, 4, D2, D4),
      2,
    ).error == seBackpressure

    let applied = settleRangeBatch(
      firstPlan, firstPlan.inflightState, true, 2
    ).state
    let duplicate = planRangeBatch(applied, firstFrame, 3)
    check duplicate.error == seNone
    check duplicate.effect == syAcknowledgeDuplicate
    check duplicate.inflightState == applied

    let completed = applyBatch(
      applied, batch("node-b", 3, 4, D2, D4), 4
    ).state
    let finalDuplicate = planRangeBatch(
      completed, batch("node-b", 3, 4, D2, D4), 5
    )
    check finalDuplicate.error == seNone
    check finalDuplicate.effect == syAcknowledgeDuplicate

  test "community isolation and batch integrity fail closed":
    let state = initial("node-b")
    check acceptRemoteDigest(
      state, digestFrame("node-b", community = "community-b"), 0
    ).error == seScopeMismatch

    let waiting = acceptRemoteDigest(state, digestFrame("node-b"), 0).state
    check planRangeBatch(
      waiting,
      batch("node-b", 1, 2, D0, D2, community = "community-b"),
      1,
    ).error == seScopeMismatch
    check cancelSync(
      waiting,
      CancelFrame(
        envelope: envelope("node-a", "node-b", "community-b"),
        reason: "wrong tenant",
      ),
    ).error == seScopeMismatch

    check planRangeBatch(
      waiting, batch("node-b", 1, 2, D0, D2, verified = false), 1
    ).error == seDigestMismatch
    var gap = batch("node-b", 1, 2, D0, D2)
    gap.records[1].sequence = 3
    check planRangeBatch(waiting, gap, 1).error == seSequenceGap

    var oversized = nextRangeRequest(waiting).get()
    oversized.limitRecords = MaxSyncRecords + 1
    check planRangeRead(
      oversized,
      "session-1",
      "community-a",
      "node-a",
      "node-b",
      4,
    ).error == seBatchBounds
