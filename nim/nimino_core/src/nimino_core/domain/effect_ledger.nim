## Replicated workflow effect claim, execution, receipt, and reconciliation policy.
##
## The adapter persists every planned state through the canonical store before
## settling it. External I/O is allowed only after an execution marker is
## durable and the #52 quorum lease is live.

import std/options

import nimino_core/domain/lease_fencing

const
  MaxEffectIdentityBytes* = 256
  MaxEffectIdempotencyKeyBytes* = 256
  MaxEffectReceiptIdBytes* = 512
  MaxEffectReconcileReasonBytes* = 1_024

type
  EffectLedgerStatus* = enum
    elsPending
    elsClaimed
    elsExecuting
    elsSucceeded
    elsFailed
    elsUnknown

  EffectReceiptOutcome* = enum
    eroSucceeded
    eroFailed

  EffectReceipt* = object
    outcome*: EffectReceiptOutcome
    receiptId*: string
    resultDigest*: string

  EffectLedgerState* = object
    valid*: bool
    communityId*: string
    workflowId*: string
    runId*: string
    stepId*: string
    idempotencyKey*: string
    effectDigest*: string
    leaseResourceId*: string
    revision*: uint64
    attempt*: uint32
    status*: EffectLedgerStatus
    ownerNodeId*: string
    fenceToken*: uint64
    receipt*: Option[EffectReceipt]
    reconciledBy*: string
    reconcileReason*: string

  EffectLedgerError* = enum
    eleNone
    eleInvalidState
    eleInvalidIdentity
    eleInvalidReceipt
    eleLeaseRejected
    eleOwnerMismatch
    eleFenceMismatch
    eleClaimConflict
    eleManualReconcileRequired
    eleReconcileUnauthorized
    eleReconcileReasonRequired
    eleTerminalConflict
    elePersistenceFailure
    eleRevisionOverflow
    eleAttemptOverflow

  EffectLedgerPortEffect* = enum
    elpNone
    elpCommitCanonical

  EffectLedgerEffect* = enum
    efeReject
    efeReplay
    efeClaimed
    efeExecuteExternal
    efeReceiptRecorded
    efeClaimRecovered
    efeUnknown
    efeManualRetry
    efeReconciled

  EffectLedgerPlan* = object
    effect*: EffectLedgerEffect
    error*: EffectLedgerError
    leaseError*: LeaseFenceError
    portEffect*: EffectLedgerPortEffect
    beforeState*: EffectLedgerState
    nextState*: EffectLedgerState

  EffectLedgerDecision* = object
    effect*: EffectLedgerEffect
    error*: EffectLedgerError
    leaseError*: LeaseFenceError
    state*: EffectLedgerState

  EffectReconcileCommand* = enum
    ercMarkSucceeded
    ercMarkFailed
    ercRetry

  EffectReconcileRequest* = object
    operatorAuthorized*: bool
    operatorId*: string
    reason*: string
    command*: EffectReconcileCommand
    receipt*: Option[EffectReceipt]

proc isSha256(value: string): bool =
  if value.len != 64:
    return false
  for character in value:
    if character notin {'0' .. '9', 'a' .. 'f'}:
      return false
  true

proc validReceipt(receipt: EffectReceipt): bool =
  receipt.receiptId.len in 1 .. MaxEffectReceiptIdBytes and
    isSha256(receipt.resultDigest)

proc validState(state: EffectLedgerState): bool =
  if not state.valid or state.communityId.len notin 1 .. MaxEffectIdentityBytes or
      state.workflowId.len notin 1 .. MaxEffectIdentityBytes or
      state.runId.len notin 1 .. MaxEffectIdentityBytes or
      state.stepId.len notin 1 .. MaxEffectIdentityBytes or
      state.leaseResourceId.len notin 1 .. MaxEffectIdentityBytes or
      state.ownerNodeId.len > MaxEffectIdentityBytes or
      state.idempotencyKey.len == 0 or
      state.idempotencyKey.len > MaxEffectIdempotencyKeyBytes or
      not isSha256(state.effectDigest):
    return false
  if state.reconciledBy.len > MaxEffectIdentityBytes or
      (state.reconciledBy.len == 0) != (state.reconcileReason.len == 0) or
      state.reconcileReason.len > MaxEffectReconcileReasonBytes:
    return false
  case state.status
  of elsPending:
    state.ownerNodeId.len == 0 and state.fenceToken == 0 and state.receipt.isNone
  of elsClaimed, elsExecuting, elsUnknown:
    state.attempt > 0 and state.ownerNodeId.len > 0 and state.fenceToken > 0 and
      state.receipt.isNone
  of elsSucceeded:
    state.attempt > 0 and state.ownerNodeId.len > 0 and state.fenceToken > 0 and
      state.receipt.isSome and validReceipt(state.receipt.get()) and
      state.receipt.get().outcome == eroSucceeded
  of elsFailed:
    state.attempt > 0 and state.ownerNodeId.len > 0 and state.fenceToken > 0 and
      state.receipt.isSome and validReceipt(state.receipt.get()) and
      state.receipt.get().outcome == eroFailed

proc initEffectLedger*(
    communityId, workflowId, runId, stepId, idempotencyKey, effectDigest,
    leaseResourceId: string
): EffectLedgerState =
  result = EffectLedgerState(
    valid: true,
    communityId: communityId,
    workflowId: workflowId,
    runId: runId,
    stepId: stepId,
    idempotencyKey: idempotencyKey,
    effectDigest: effectDigest,
    leaseResourceId: leaseResourceId,
    status: elsPending,
  )
  result.valid = validState(result)

proc reject(
    state: EffectLedgerState; error: EffectLedgerError;
    leaseError = lfeNone
): EffectLedgerPlan =
  EffectLedgerPlan(
    effect: efeReject,
    error: error,
    leaseError: leaseError,
    beforeState: state,
    nextState: state,
  )

proc replay(state: EffectLedgerState): EffectLedgerPlan =
  EffectLedgerPlan(
    effect: efeReplay,
    error: eleNone,
    leaseError: lfeNone,
    portEffect: elpNone,
    beforeState: state,
    nextState: state,
  )

proc persist(
    state, next: EffectLedgerState; effect: EffectLedgerEffect;
    leaseError = lfeNone
): EffectLedgerPlan =
  EffectLedgerPlan(
    effect: effect,
    error: eleNone,
    leaseError: leaseError,
    portEffect: elpCommitCanonical,
    beforeState: state,
    nextState: next,
  )

proc incrementRevision(
    state: EffectLedgerState; next: var EffectLedgerState
): EffectLedgerError =
  if state.revision == high(uint64):
    return eleRevisionOverflow
  next.revision = state.revision + 1
  eleNone

proc authorize(
    state: EffectLedgerState; ownerNodeId: string; fenceToken: uint64;
    leaseState: LeaseState; fact: ServingLeaseFact
): SingletonEffectDecision =
  authorizeSingletonEffect(
    leaseState,
    SingletonEffectAttempt(
      resourceId: state.leaseResourceId,
      ownerId: ownerNodeId,
      fenceToken: fenceToken,
    ),
    fact,
  )

proc planEffectClaim*(
    state: EffectLedgerState; ownerNodeId: string; fenceToken: uint64;
    leaseState: LeaseState; fact: ServingLeaseFact
): EffectLedgerPlan =
  if not validState(state):
    return reject(state, eleInvalidState)
  if state.status == elsUnknown:
    return reject(state, eleManualReconcileRequired)
  if state.status == elsClaimed:
    if state.ownerNodeId == ownerNodeId and state.fenceToken == fenceToken:
      return replay(state)
    return reject(state, eleClaimConflict)
  if state.status != elsPending:
    return reject(state, eleInvalidState)
  if ownerNodeId.len == 0:
    return reject(state, eleInvalidIdentity)
  let lease = authorize(state, ownerNodeId, fenceToken, leaseState, fact)
  if not lease.allowed:
    return reject(state, eleLeaseRejected, lease.error)
  if state.attempt == high(uint32):
    return reject(state, eleAttemptOverflow)
  var next = state
  let revisionError = incrementRevision(state, next)
  if revisionError != eleNone:
    return reject(state, revisionError)
  next.status = elsClaimed
  next.ownerNodeId = ownerNodeId
  next.fenceToken = fenceToken
  next.attempt = state.attempt + 1
  persist(state, next, efeClaimed)

proc planEffectExecution*(
    state: EffectLedgerState; ownerNodeId: string; fenceToken: uint64;
    leaseState: LeaseState; fact: ServingLeaseFact
): EffectLedgerPlan =
  if not validState(state) or state.status != elsClaimed:
    return reject(
      state,
      if state.status == elsUnknown: eleManualReconcileRequired else: eleInvalidState,
    )
  if state.ownerNodeId != ownerNodeId:
    return reject(state, eleOwnerMismatch)
  if state.fenceToken != fenceToken:
    return reject(state, eleFenceMismatch)
  let lease = authorize(state, ownerNodeId, fenceToken, leaseState, fact)
  if not lease.allowed:
    return reject(state, eleLeaseRejected, lease.error)
  var next = state
  let revisionError = incrementRevision(state, next)
  if revisionError != eleNone:
    return reject(state, revisionError)
  next.status = elsExecuting
  persist(state, next, efeExecuteExternal)

proc planEffectReceipt*(
    state: EffectLedgerState; ownerNodeId: string; fenceToken: uint64;
    receipt: EffectReceipt
): EffectLedgerPlan =
  if not validState(state):
    return reject(state, eleInvalidState)
  if state.status in {elsSucceeded, elsFailed}:
    if state.ownerNodeId == ownerNodeId and state.fenceToken == fenceToken and
        state.receipt == some(receipt):
      return replay(state)
    return reject(state, eleTerminalConflict)
  if state.status == elsUnknown:
    return reject(state, eleManualReconcileRequired)
  if state.status != elsExecuting:
    return reject(state, eleInvalidState)
  if state.ownerNodeId != ownerNodeId:
    return reject(state, eleOwnerMismatch)
  if state.fenceToken != fenceToken:
    return reject(state, eleFenceMismatch)
  if not validReceipt(receipt):
    return reject(state, eleInvalidReceipt)
  var next = state
  let revisionError = incrementRevision(state, next)
  if revisionError != eleNone:
    return reject(state, revisionError)
  next.status = if receipt.outcome == eroSucceeded: elsSucceeded else: elsFailed
  next.receipt = some(receipt)
  persist(state, next, efeReceiptRecorded)

proc planEffectRecovery*(
    state: EffectLedgerState; leaseState: LeaseState; fact: ServingLeaseFact
): EffectLedgerPlan =
  if not validState(state):
    return reject(state, eleInvalidState)
  if state.status == elsExecuting:
    var next = state
    let revisionError = incrementRevision(state, next)
    if revisionError != eleNone:
      return reject(state, revisionError)
    next.status = elsUnknown
    return persist(state, next, efeUnknown)
  if state.status == elsClaimed:
    let lease = authorize(
      state, state.ownerNodeId, state.fenceToken, leaseState, fact
    )
    if lease.allowed:
      return replay(state)
    if lease.error notin {
      lfeNoActiveLease,
      lfeLeaseExpired,
      lfeStaleFence,
      lfeOwnerMismatch,
      lfeAuthorityStale,
      lfeClockEpochMismatch,
    }:
      return reject(state, eleLeaseRejected, lease.error)
    var next = state
    let revisionError = incrementRevision(state, next)
    if revisionError != eleNone:
      return reject(state, revisionError)
    next.status = elsPending
    next.ownerNodeId = ""
    next.fenceToken = 0
    return persist(state, next, efeClaimRecovered, lease.error)
  replay(state)

proc planEffectReconcile*(
    state: EffectLedgerState; request: EffectReconcileRequest
): EffectLedgerPlan =
  if not validState(state):
    return reject(state, eleInvalidState)
  if state.status != elsUnknown:
    return reject(state, eleInvalidState)
  if not request.operatorAuthorized or
      request.operatorId.len notin 1 .. MaxEffectIdentityBytes:
    return reject(state, eleReconcileUnauthorized)
  if request.reason.len == 0 or request.reason.len > MaxEffectReconcileReasonBytes:
    return reject(state, eleReconcileReasonRequired)
  var next = state
  let revisionError = incrementRevision(state, next)
  if revisionError != eleNone:
    return reject(state, revisionError)
  next.reconciledBy = request.operatorId
  next.reconcileReason = request.reason
  case request.command
  of ercRetry:
    if request.receipt.isSome:
      return reject(state, eleInvalidReceipt)
    next.status = elsPending
    next.ownerNodeId = ""
    next.fenceToken = 0
    return persist(state, next, efeManualRetry)
  of ercMarkSucceeded, ercMarkFailed:
    if request.receipt.isNone or not validReceipt(request.receipt.get()):
      return reject(state, eleInvalidReceipt)
    let expected = if request.command == ercMarkSucceeded:
        eroSucceeded
      else:
        eroFailed
    if request.receipt.get().outcome != expected:
      return reject(state, eleInvalidReceipt)
    next.status = if expected == eroSucceeded: elsSucceeded else: elsFailed
    next.receipt = request.receipt
    persist(state, next, efeReconciled)

proc settleEffectLedger*(
    plan: EffectLedgerPlan; persistenceSucceeded: bool
): EffectLedgerDecision =
  if plan.error != eleNone:
    return EffectLedgerDecision(
      effect: efeReject,
      error: plan.error,
      leaseError: plan.leaseError,
      state: plan.beforeState,
    )
  if plan.portEffect == elpCommitCanonical and not persistenceSucceeded:
    return EffectLedgerDecision(
      effect: efeReject,
      error: elePersistenceFailure,
      leaseError: plan.leaseError,
      state: plan.beforeState,
    )
  EffectLedgerDecision(
    effect: plan.effect,
    error: eleNone,
    leaseError: plan.leaseError,
    state: plan.nextState,
  )
