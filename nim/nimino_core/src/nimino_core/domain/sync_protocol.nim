## Bounded pull-based anti-entropy decisions owned by Nimino.
##
## Rust computes and verifies SHA-256, performs exact-checkpoint store commits,
## and transports frames over Chirps. This module owns session scope, range
## order, backpressure, cancellation, and durable checkpoint advancement.

import std/[options, strutils]

const
  SyncProtocolName* = "nimino.sync"
  SyncProtocolVersion* = 2'u16
  MaxSyncRecords* = 1_000'u16
  MaxSyncEncodedBytes* = 1_048_576'u32

type
  SyncPhase* = enum
    spIdle
    spWaitingBatch
    spApplying
    spComplete
    spCancelled

  SyncEffect* = enum
    syReject
    syNoop
    syRequestRange
    syRequestSnapshot
    syApplyBatch
    syAcknowledgeDuplicate
    syComplete
    syCancel

  SyncError* = enum
    seNone
    seInvalidState
    seProtocolMismatch
    seScopeMismatch
    seDigestInvalid
    seDigestMismatch
    seRemoteBehind
    sePhaseInvalid
    seBackpressure
    seBatchBounds
    seSequenceGap
    sePeerTimeout
    seCancelled
    seStoreFailure
    seStoreCheckpointMismatch
    seStaleSettlement
    seReasonRequired
    seTickOverflow
    seRevisionOverflow

  SyncEnvelope* = object
    protocol*: string
    version*: uint16
    sessionId*: string
    communityId*: string
    senderNodeId*: string
    receiverNodeId*: string

  DigestFrame* = object
    envelope*: SyncEnvelope
    checkpoint*: uint64
    prefixDigest*: string

  RangeRequestFrame* = object
    envelope*: SyncEnvelope
    afterCheckpoint*: uint64
    limitRecords*: uint16
    maxEncodedBytes*: uint32

  SyncRecord* = object
    sequence*: uint64
    recordType*: string
    key*: string
    deleted*: bool
    payload*: string
    contentDigest*: string

  RangeBatchFrame* = object
    envelope*: SyncEnvelope
    batchId*: string
    baseCheckpoint*: uint64
    baseDigest*: string
    throughCheckpoint*: uint64
    resultDigest*: string
    encodedBytes*: uint32
    digestVerified*: bool
    records*: seq[SyncRecord]

  CancelFrame* = object
    envelope*: SyncEnvelope
    reason*: string

  SyncState* = object
    valid*: bool
    revision*: uint64
    phase*: SyncPhase
    sessionId*: string
    communityId*: string
    localNodeId*: string
    remoteNodeId*: string
    checkpoint*: uint64
    checkpointDigest*: string
    remoteCheckpoint*: uint64
    remoteDigest*: string
    maxRecords*: uint16
    maxEncodedBytes*: uint32
    timeoutTicks*: uint64
    deadlineTick*: uint64
    pendingBatchId*: string

  SyncDecision* = object
    effect*: SyncEffect
    error*: SyncError
    state*: SyncState

  RangeBatchPlan* = object
    effect*: SyncEffect
    error*: SyncError
    beforeState*: SyncState
    inflightState*: SyncState
    nextState*: SyncState
    expectedCheckpoint*: uint64
    throughCheckpoint*: uint64
    records*: seq[SyncRecord]

  RangeReadPlan* = object
    allowed*: bool
    error*: SyncError
    communityId*: string
    afterCheckpoint*: uint64
    limitRecords*: uint16
    maxEncodedBytes*: uint32

proc isSha256(value: string): bool =
  value.len == 64 and value.allCharsInSet({'0' .. '9', 'a' .. 'f'})

proc validState(state: SyncState): bool =
  if not state.valid or state.sessionId.len == 0 or state.communityId.len == 0 or
      state.localNodeId.len == 0 or state.remoteNodeId.len == 0 or
      state.localNodeId == state.remoteNodeId or not isSha256(state.checkpointDigest) or
      state.maxRecords == 0 or state.maxRecords > MaxSyncRecords or
      state.maxEncodedBytes == 0 or
      state.maxEncodedBytes > MaxSyncEncodedBytes or state.timeoutTicks == 0:
    return false
  case state.phase
  of spIdle:
    state.remoteCheckpoint == 0 and state.remoteDigest.len == 0 and
      state.deadlineTick == 0 and state.pendingBatchId.len == 0
  of spWaitingBatch:
    state.remoteCheckpoint > state.checkpoint and isSha256(state.remoteDigest) and
      state.deadlineTick > 0 and state.pendingBatchId.len == 0
  of spApplying:
    state.remoteCheckpoint > state.checkpoint and isSha256(state.remoteDigest) and
      state.deadlineTick > 0 and state.pendingBatchId.len > 0
  of spComplete:
    state.remoteCheckpoint == state.checkpoint and
      state.remoteDigest == state.checkpointDigest and
      state.deadlineTick == 0 and state.pendingBatchId.len == 0
  of spCancelled:
    state.pendingBatchId.len == 0

proc initSyncState*(
    sessionId, communityId, localNodeId, remoteNodeId: string;
    checkpoint: uint64; checkpointDigest: string; maxRecords: uint16;
    maxEncodedBytes: uint32; timeoutTicks: uint64
): SyncState =
  result = SyncState(
    valid: true,
    phase: spIdle,
    sessionId: sessionId,
    communityId: communityId,
    localNodeId: localNodeId,
    remoteNodeId: remoteNodeId,
    checkpoint: checkpoint,
    checkpointDigest: checkpointDigest,
    maxRecords: maxRecords,
    maxEncodedBytes: maxEncodedBytes,
    timeoutTicks: timeoutTicks,
  )
  result.valid = validState(result)

proc envelopeError(state: SyncState; envelope: SyncEnvelope): SyncError =
  if envelope.protocol != SyncProtocolName or envelope.version != SyncProtocolVersion:
    return seProtocolMismatch
  if envelope.sessionId != state.sessionId or
      envelope.communityId != state.communityId or
      envelope.senderNodeId != state.remoteNodeId or
      envelope.receiverNodeId != state.localNodeId:
    return seScopeMismatch
  seNone

proc reject(state: SyncState; error: SyncError): SyncDecision =
  SyncDecision(effect: syReject, error: error, state: state)

proc deadline(nowTick, timeoutTicks: uint64): Option[uint64] =
  if timeoutTicks > high(uint64) - nowTick:
    none(uint64)
  else:
    some(nowTick + timeoutTicks)

proc acceptRemoteDigest*(
    state: SyncState; frame: DigestFrame; nowTick: uint64
): SyncDecision =
  if not validState(state):
    return reject(state, seInvalidState)
  if state.phase == spCancelled:
    return reject(state, seCancelled)
  if state.phase != spIdle:
    return reject(state, sePhaseInvalid)
  if state.revision == high(uint64):
    return reject(state, seRevisionOverflow)
  let scopeError = envelopeError(state, frame.envelope)
  if scopeError != seNone:
    return reject(state, scopeError)
  if not isSha256(frame.prefixDigest):
    return reject(state, seDigestInvalid)
  if frame.prefixDigest == state.checkpointDigest:
    var complete = state
    complete.revision += 1
    complete.phase = spComplete
    complete.remoteCheckpoint = frame.checkpoint
    complete.remoteDigest = frame.prefixDigest
    return SyncDecision(effect: syComplete, error: seNone, state: complete)
  if frame.checkpoint < state.checkpoint:
    return reject(state, seRemoteBehind)
  if frame.checkpoint == state.checkpoint and
      state.localNodeId > state.remoteNodeId:
    return SyncDecision(effect: syNoop, error: seNone, state: state)

  let nextDeadline = deadline(nowTick, state.timeoutTicks)
  if nextDeadline.isNone:
    return reject(state, seTickOverflow)
  var waiting = state
  waiting.revision += 1
  waiting.phase = spWaitingBatch
  waiting.remoteCheckpoint = frame.checkpoint
  waiting.remoteDigest = frame.prefixDigest
  waiting.deadlineTick = nextDeadline.get()
  SyncDecision(effect: syRequestSnapshot, error: seNone, state: waiting)

proc nextRangeRequest*(state: SyncState): Option[RangeRequestFrame] =
  if not validState(state) or state.phase != spWaitingBatch:
    return none(RangeRequestFrame)
  let remaining = state.remoteCheckpoint - state.checkpoint
  let limit = min(remaining, uint64(state.maxRecords)).uint16
  some(
    RangeRequestFrame(
      envelope: SyncEnvelope(
        protocol: SyncProtocolName,
        version: SyncProtocolVersion,
        sessionId: state.sessionId,
        communityId: state.communityId,
        senderNodeId: state.localNodeId,
        receiverNodeId: state.remoteNodeId,
      ),
      afterCheckpoint: state.checkpoint,
      limitRecords: limit,
      maxEncodedBytes: state.maxEncodedBytes,
    )
  )

proc planRangeRead*(
    frame: RangeRequestFrame; sessionId, communityId, sourceNodeId,
    targetNodeId: string; sourceCheckpoint: uint64
): RangeReadPlan =
  if frame.envelope.protocol != SyncProtocolName or
      frame.envelope.version != SyncProtocolVersion:
    return RangeReadPlan(error: seProtocolMismatch)
  if sessionId.len == 0 or communityId.len == 0 or sourceNodeId.len == 0 or
      targetNodeId.len == 0 or sourceNodeId == targetNodeId or
      frame.envelope.sessionId != sessionId or
      frame.envelope.communityId != communityId or
      frame.envelope.senderNodeId != targetNodeId or
      frame.envelope.receiverNodeId != sourceNodeId:
    return RangeReadPlan(error: seScopeMismatch)
  if frame.limitRecords == 0 or frame.limitRecords > MaxSyncRecords or
      frame.maxEncodedBytes == 0 or
      frame.maxEncodedBytes > MaxSyncEncodedBytes or
      frame.afterCheckpoint >= sourceCheckpoint:
    return RangeReadPlan(error: seBatchBounds)
  RangeReadPlan(
    allowed: true,
    error: seNone,
    communityId: communityId,
    afterCheckpoint: frame.afterCheckpoint,
    limitRecords: frame.limitRecords,
    maxEncodedBytes: frame.maxEncodedBytes,
  )

proc rejectPlan(state: SyncState; error: SyncError): RangeBatchPlan =
  RangeBatchPlan(
    effect: syReject,
    error: error,
    beforeState: state,
    inflightState: state,
    nextState: state,
  )

proc validBatchShape(state: SyncState; frame: RangeBatchFrame): SyncError =
  if frame.batchId.len == 0 or frame.records.len == 0 or
      frame.records.len > int(state.maxRecords) or frame.encodedBytes == 0 or
      frame.encodedBytes > state.maxEncodedBytes or
      frame.throughCheckpoint <= frame.baseCheckpoint or
      not isSha256(frame.baseDigest) or not isSha256(frame.resultDigest):
    return seBatchBounds
  var payloadBytes = 0'u64
  var expected = frame.baseCheckpoint
  for record in frame.records:
    if expected == high(uint64) or record.sequence != expected + 1:
      return seSequenceGap
    if record.recordType.len == 0 or record.key.len == 0 or
        record.payload.len > int(state.maxEncodedBytes) or
        not isSha256(record.contentDigest):
      return seBatchBounds
    expected = record.sequence
    payloadBytes += uint64(record.payload.len)
  if expected != frame.throughCheckpoint or
      uint64(frame.records.len) != frame.throughCheckpoint - frame.baseCheckpoint or
      payloadBytes > uint64(frame.encodedBytes):
    return seBatchBounds
  if not frame.digestVerified:
    return seDigestMismatch
  seNone

proc planRangeBatch*(
    state: SyncState; frame: RangeBatchFrame; nowTick: uint64
): RangeBatchPlan =
  if not validState(state):
    return rejectPlan(state, seInvalidState)
  if state.phase == spCancelled:
    return rejectPlan(state, seCancelled)
  if state.phase == spApplying:
    return rejectPlan(state, seBackpressure)
  if state.phase notin {spWaitingBatch, spComplete}:
    return rejectPlan(state, sePhaseInvalid)
  if state.phase == spWaitingBatch and state.revision > high(uint64) - 2:
    return rejectPlan(state, seRevisionOverflow)
  if state.phase == spWaitingBatch and nowTick >= state.deadlineTick:
    return rejectPlan(state, sePeerTimeout)
  let scopeError = envelopeError(state, frame.envelope)
  if scopeError != seNone:
    return rejectPlan(state, scopeError)
  let shapeError = validBatchShape(state, frame)
  if shapeError != seNone:
    return rejectPlan(state, shapeError)

  if frame.throughCheckpoint == state.checkpoint and
      frame.resultDigest == state.checkpointDigest and
      frame.baseCheckpoint < state.checkpoint:
    return RangeBatchPlan(
      effect: syAcknowledgeDuplicate,
      error: seNone,
      beforeState: state,
      inflightState: state,
      nextState: state,
      expectedCheckpoint: state.checkpoint,
      throughCheckpoint: state.checkpoint,
    )
  if state.phase == spComplete:
    return rejectPlan(state, sePhaseInvalid)
  if frame.baseCheckpoint != state.checkpoint or
      frame.baseDigest != state.checkpointDigest:
    return rejectPlan(state, seDigestMismatch)
  if frame.throughCheckpoint > state.remoteCheckpoint:
    return rejectPlan(state, seBatchBounds)
  if frame.throughCheckpoint == state.remoteCheckpoint and
      frame.resultDigest != state.remoteDigest:
    return rejectPlan(state, seDigestMismatch)

  let nextDeadline = deadline(nowTick, state.timeoutTicks)
  if nextDeadline.isNone:
    return rejectPlan(state, seTickOverflow)
  var inflight = state
  inflight.revision += 1
  inflight.phase = spApplying
  inflight.pendingBatchId = frame.batchId

  var next = inflight
  next.revision += 1
  next.checkpoint = frame.throughCheckpoint
  next.checkpointDigest = frame.resultDigest
  next.pendingBatchId = ""
  if next.checkpoint == next.remoteCheckpoint:
    next.phase = spComplete
    next.deadlineTick = 0
  else:
    next.phase = spWaitingBatch
    next.deadlineTick = nextDeadline.get()
  RangeBatchPlan(
    effect: syApplyBatch,
    error: seNone,
    beforeState: state,
    inflightState: inflight,
    nextState: next,
    expectedCheckpoint: state.checkpoint,
    throughCheckpoint: frame.throughCheckpoint,
    records: frame.records,
  )

proc settleRangeBatch*(
    plan: RangeBatchPlan; currentState: SyncState; storeSucceeded: bool;
    committedCheckpoint: uint64
): SyncDecision =
  if plan.error != seNone:
    return reject(currentState, plan.error)
  if plan.effect == syAcknowledgeDuplicate:
    return SyncDecision(
      effect: syAcknowledgeDuplicate,
      error: seNone,
      state: currentState,
    )
  if plan.effect != syApplyBatch:
    return reject(currentState, sePhaseInvalid)
  if currentState != plan.inflightState:
    if currentState.phase == spCancelled:
      return reject(currentState, seCancelled)
    return reject(currentState, seStaleSettlement)
  if not storeSucceeded:
    var retry = plan.beforeState
    retry.revision = currentState.revision + 1
    return reject(retry, seStoreFailure)
  if committedCheckpoint != plan.throughCheckpoint:
    var cancelled = currentState
    cancelled.revision += 1
    cancelled.phase = spCancelled
    cancelled.pendingBatchId = ""
    return reject(cancelled, seStoreCheckpointMismatch)
  let effect = if plan.nextState.phase == spComplete: syComplete else: syRequestRange
  SyncDecision(effect: effect, error: seNone, state: plan.nextState)

proc stopSync*(state: SyncState; reason: string): SyncDecision =
  if not validState(state):
    return reject(state, seInvalidState)
  if reason.len == 0:
    return reject(state, seReasonRequired)
  if state.phase == spCancelled:
    return SyncDecision(effect: syCancel, error: seNone, state: state)
  if state.revision == high(uint64):
    return reject(state, seRevisionOverflow)
  var cancelled = state
  cancelled.revision += 1
  cancelled.phase = spCancelled
  cancelled.pendingBatchId = ""
  SyncDecision(effect: syCancel, error: seNone, state: cancelled)

proc cancelSync*(state: SyncState; frame: CancelFrame): SyncDecision =
  if not validState(state):
    return reject(state, seInvalidState)
  let scopeError = envelopeError(state, frame.envelope)
  if scopeError != seNone:
    return reject(state, scopeError)
  stopSync(state, frame.reason)

proc checkSyncDeadline*(state: SyncState; nowTick: uint64): SyncDecision =
  if not validState(state):
    return reject(state, seInvalidState)
  if state.phase notin {spWaitingBatch, spApplying} or nowTick < state.deadlineTick:
    return SyncDecision(effect: syNoop, error: seNone, state: state)
  if state.revision == high(uint64):
    return reject(state, seRevisionOverflow)
  var cancelled = state
  cancelled.revision += 1
  cancelled.phase = spCancelled
  cancelled.pendingBatchId = ""
  SyncDecision(effect: syCancel, error: sePeerTimeout, state: cancelled)
