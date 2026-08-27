## Manual data verification and repair policy for a Nimino replica.
##
## Adapters report bounded digests and execute the returned repair directive.
## They do not choose the authoritative replica or declare convergence.

import std/[algorithm, sequtils, sets]

const
  MaxRepairNodes* = 5
  MaxRepairIdentityBytes* = 256
  MaxRepairReasonBytes* = 1_024

type
  RepairPhase* = enum
    rpUnverified
    rpQuarantined
    rpRepairing
    rpVerifying
    rpComplete
    rpBlockedEffects

  RepairEffect* = enum
    reReject
    reVerified
    reQuarantine
    reBeginRepair
    reVerifyRepair
    reRepairFailed
    reManualEffectReconcile
    reReplay

  RepairError* = enum
    rpeNone
    rpeInvalidState
    rpeInvalidFacts
    rpeTargetMissing
    rpeNoQuorum
    rpeOperatorUnauthorized
    rpeReasonRequired
    rpeSourceMismatch
    rpePersistenceFailure
    rpeAdapterFailure
    rpeRevisionOverflow

  ReplicaHealthFact* = object
    communityId*: string
    nodeId*: string
    readable*: bool
    checkpoint*: uint64
    canonicalDigest*: string
    projectionDigest*: string
    objectDigest*: string
    unknownEffects*: uint32

  RepairState* = object
    valid*: bool
    communityId*: string
    targetNodeId*: string
    sourceNodeId*: string
    repairId*: string
    operatorId*: string
    reason*: string
    checkpoint*: uint64
    canonicalDigest*: string
    projectionDigest*: string
    objectDigest*: string
    revision*: uint64
    phase*: RepairPhase

  RepairRequest* = object
    communityId*: string
    targetNodeId*: string
    sourceNodeId*: string
    repairId*: string
    operatorAuthorized*: bool
    operatorId*: string
    reason*: string

  RepairPortEffect* = enum
    rpoNone
    rpoCommitRepairMarker

  RepairPlan* = object
    effect*: RepairEffect
    error*: RepairError
    portEffect*: RepairPortEffect
    beforeState*: RepairState
    nextState*: RepairState
    replaceCanonical*: bool
    rebuildProjections*: bool
    repairObjects*: bool

  RepairDecision* = object
    effect*: RepairEffect
    error*: RepairError
    state*: RepairState
    replaceCanonical*: bool
    rebuildProjections*: bool
    repairObjects*: bool

proc isSha256(value: string): bool =
  if value.len != 64:
    return false
  for character in value:
    if character notin {'0' .. '9', 'a' .. 'f'}:
      return false
  true

proc validIdentity(value: string): bool =
  value.len in 1 .. MaxRepairIdentityBytes

proc validReadableFact(fact: ReplicaHealthFact): bool =
  validIdentity(fact.communityId) and validIdentity(fact.nodeId) and
    isSha256(fact.canonicalDigest) and isSha256(fact.projectionDigest) and
    isSha256(fact.objectDigest)

proc sameSnapshot(left, right: ReplicaHealthFact): bool =
  left.checkpoint == right.checkpoint and
    left.canonicalDigest == right.canonicalDigest and
    left.projectionDigest == right.projectionDigest and
    left.objectDigest == right.objectDigest and
    left.unknownEffects == right.unknownEffects

proc validState(state: RepairState): bool =
  if not state.valid or not validIdentity(state.communityId) or
      not validIdentity(state.targetNodeId):
    return false
  if state.phase == rpUnverified:
    return state.sourceNodeId.len == 0 and state.repairId.len == 0 and
      state.revision == 0
  if not validIdentity(state.sourceNodeId) or
      not isSha256(state.canonicalDigest) or
      not isSha256(state.projectionDigest) or not isSha256(state.objectDigest):
    return false
  let auditEmpty = state.repairId.len == 0 and state.operatorId.len == 0 and
    state.reason.len == 0
  let auditValid = validIdentity(state.repairId) and
    validIdentity(state.operatorId) and
    state.reason.len in 1 .. MaxRepairReasonBytes
  auditEmpty or auditValid

proc initRepairState*(communityId, targetNodeId: string): RepairState =
  result = RepairState(
    valid: true,
    communityId: communityId,
    targetNodeId: targetNodeId,
    phase: rpUnverified,
  )
  result.valid = validState(result)

proc reject(state: RepairState; error: RepairError): RepairPlan =
  RepairPlan(
    effect: reReject,
    error: error,
    beforeState: state,
    nextState: state,
  )

proc snapshotState(
    state: RepairState; source: ReplicaHealthFact; phase: RepairPhase;
    effect: RepairEffect
): RepairPlan =
  if state.revision == high(uint64):
    return reject(state, rpeRevisionOverflow)
  var next = state
  next.sourceNodeId = source.nodeId
  next.checkpoint = source.checkpoint
  next.canonicalDigest = source.canonicalDigest
  next.projectionDigest = source.projectionDigest
  next.objectDigest = source.objectDigest
  next.revision += 1
  next.phase = phase
  RepairPlan(
    effect: effect,
    portEffect: rpoCommitRepairMarker,
    beforeState: state,
    nextState: next,
  )

proc authoritativeFact(
    state: RepairState; facts: seq[ReplicaHealthFact]
): tuple[error: RepairError; fact: ReplicaHealthFact] =
  if facts.len == 0 or facts.len > MaxRepairNodes or facts.len mod 2 == 0:
    return (rpeInvalidFacts, ReplicaHealthFact())
  var nodes = initHashSet[string]()
  var targetFound = false
  for fact in facts:
    if not validIdentity(fact.nodeId) or fact.nodeId in nodes or
        fact.communityId != state.communityId or
        (fact.readable and not validReadableFact(fact)):
      return (rpeInvalidFacts, ReplicaHealthFact())
    nodes.incl(fact.nodeId)
    targetFound = targetFound or fact.nodeId == state.targetNodeId
  if not targetFound:
    return (rpeTargetMissing, ReplicaHealthFact())

  let quorum = facts.len div 2 + 1
  var candidates = facts.filterIt(it.readable)
  candidates.sort(proc(left, right: ReplicaHealthFact): int =
    cmp(left.nodeId, right.nodeId)
  )
  for candidate in candidates:
    var matches = 0
    for fact in candidates:
      if sameSnapshot(candidate, fact):
        inc matches
    if matches >= quorum:
      return (rpeNone, candidate)
  (rpeNoQuorum, ReplicaHealthFact())

proc planRepairVerification*(
    state: RepairState; facts: seq[ReplicaHealthFact]
): RepairPlan =
  if not validState(state) or state.phase notin {
      rpUnverified, rpQuarantined, rpVerifying, rpComplete, rpBlockedEffects
    }:
    return reject(state, rpeInvalidState)
  let authority = authoritativeFact(state, facts)
  if authority.error != rpeNone:
    return reject(state, authority.error)
  var target: ReplicaHealthFact
  var targetFound = false
  for fact in facts:
    if fact.nodeId == state.targetNodeId:
      target = fact
      targetFound = true
      break
  if not targetFound:
    return reject(state, rpeTargetMissing)

  if target.readable and sameSnapshot(target, authority.fact):
    let effect = if authority.fact.unknownEffects > 0:
        reManualEffectReconcile
      else:
        reVerified
    let phase = if authority.fact.unknownEffects > 0:
        rpBlockedEffects
      else:
        rpComplete
    if state.phase == phase and state.sourceNodeId == authority.fact.nodeId and
        state.checkpoint == authority.fact.checkpoint and
        state.canonicalDigest == authority.fact.canonicalDigest and
        state.projectionDigest == authority.fact.projectionDigest and
        state.objectDigest == authority.fact.objectDigest:
      return RepairPlan(
        effect: reReplay, beforeState: state, nextState: state
      )
    return snapshotState(state, authority.fact, phase, effect)
  if state.phase == rpQuarantined and state.sourceNodeId == authority.fact.nodeId and
      state.checkpoint == authority.fact.checkpoint and
      state.canonicalDigest == authority.fact.canonicalDigest and
      state.projectionDigest == authority.fact.projectionDigest and
      state.objectDigest == authority.fact.objectDigest:
    return RepairPlan(effect: reReplay, beforeState: state, nextState: state)
  snapshotState(state, authority.fact, rpQuarantined, reQuarantine)

proc planManualRepair*(
    state: RepairState; request: RepairRequest
): RepairPlan =
  if not validState(state) or state.phase != rpQuarantined:
    return reject(state, rpeInvalidState)
  if not request.operatorAuthorized or not validIdentity(request.operatorId):
    return reject(state, rpeOperatorUnauthorized)
  if request.reason.len == 0 or request.reason.len > MaxRepairReasonBytes:
    return reject(state, rpeReasonRequired)
  if request.communityId != state.communityId or
      request.targetNodeId != state.targetNodeId or
      request.sourceNodeId != state.sourceNodeId or
      not validIdentity(request.repairId):
    return reject(state, rpeSourceMismatch)
  if state.revision == high(uint64):
    return reject(state, rpeRevisionOverflow)
  var next = state
  next.repairId = request.repairId
  next.operatorId = request.operatorId
  next.reason = request.reason
  next.revision += 1
  next.phase = rpRepairing
  RepairPlan(
    effect: reBeginRepair,
    portEffect: rpoCommitRepairMarker,
    beforeState: state,
    nextState: next,
    replaceCanonical: true,
    rebuildProjections: true,
    repairObjects: true,
  )

proc settleRepairPlan*(
    plan: RepairPlan; persistenceSucceeded: bool
): RepairDecision =
  if plan.error != rpeNone:
    return RepairDecision(
      effect: reReject, error: plan.error, state: plan.beforeState
    )
  if plan.portEffect == rpoCommitRepairMarker and not persistenceSucceeded:
    return RepairDecision(
      effect: reReject,
      error: rpePersistenceFailure,
      state: plan.beforeState,
    )
  RepairDecision(
    effect: plan.effect,
    state: plan.nextState,
    replaceCanonical: plan.replaceCanonical,
    rebuildProjections: plan.rebuildProjections,
    repairObjects: plan.repairObjects,
  )

proc settleRepairAdapters*(
    state: RepairState; adaptersSucceeded: bool
): RepairDecision =
  if not validState(state) or state.phase != rpRepairing:
    return RepairDecision(effect: reReject, error: rpeInvalidState, state: state)
  if state.revision == high(uint64):
    return RepairDecision(
      effect: reReject, error: rpeRevisionOverflow, state: state
    )
  var next = state
  next.revision += 1
  next.phase = if adaptersSucceeded: rpVerifying else: rpQuarantined
  RepairDecision(
    effect: (if adaptersSucceeded: reVerifyRepair else: reRepairFailed),
    error: (if adaptersSucceeded: rpeNone else: rpeAdapterFailure),
    state: next,
  )

proc recoverInterruptedRepair*(state: RepairState): RepairDecision =
  if not validState(state):
    return RepairDecision(effect: reReject, error: rpeInvalidState, state: state)
  if state.phase != rpRepairing:
    return RepairDecision(effect: reReplay, state: state)
  if state.revision == high(uint64):
    return RepairDecision(
      effect: reReject, error: rpeRevisionOverflow, state: state
    )
  var next = state
  next.revision += 1
  next.phase = rpQuarantined
  RepairDecision(effect: reRepairFailed, error: rpeAdapterFailure, state: next)
