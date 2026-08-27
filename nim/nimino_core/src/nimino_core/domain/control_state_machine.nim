## Deterministic replicated-control decisions owned by Nimino.
##
## The caller persists every returned plan before settling it. Chirps supplies
## authenticated messages only; the Rust store supplies atomic bytes only.

import std/[options, sequtils, sets]

type
  ControlVoterPhase* = enum
    cvStableOld
    cvJoint
    cvStableNew

  ControlEntryKind* = enum
    cekCommand
    cekBeginJoint
    cekFinalize

  ControlEffect* = enum
    ceReject
    ceElectLeader
    ceAppend
    ceCommit
    ceApply
    ceSnapshot

  ControlStoreActionKind* = enum
    cpaMetadata
    cpaLog
    cpaSnapshot

  ControlStateError* = enum
    cseNone
    cseInvalidVoters
    cseStaleTerm
    cseCandidateNotVoter
    cseQuorumRequired
    cseLeaderRequired
    cseAuthorityStale
    csePendingEntry
    cseEntryKindInvalid
    cseCommandRequired
    cseLogGap
    cseCommitOrder
    cseApplyOrder
    cseSnapshotUnavailable
    cseFactConflict
    cseStoreFailure
    cseCorruptRecovery

  ControlEntry* = object
    index*: uint64
    term*: uint64
    voterEpoch*: uint64
    kind*: ControlEntryKind
    commandId*: string
    payload*: string
    targetVoters*: seq[string]

  ControlSnapshotState* = object
    lastIncludedIndex*: uint64
    lastIncludedTerm*: uint64
    voterEpoch*: uint64
    phase*: ControlVoterPhase
    oldVoters*: seq[string]
    newVoters*: seq[string]
    statePayload*: string

  ControlState* = object
    valid*: bool
    metadataRevision*: uint64
    term*: uint64
    votedFor*: Option[string]
    voterEpoch*: uint64
    phase*: ControlVoterPhase
    oldVoters*: seq[string]
    newVoters*: seq[string]
    leaderId*: string
    leaderTerm*: uint64
    leaderProof*: seq[string]
    lastIndex*: uint64
    commitIndex*: uint64
    appliedIndex*: uint64
    snapshot*: Option[ControlSnapshotState]
    log*: seq[ControlEntry]

  ElectionRequest* = object
    term*: uint64
    candidateId*: string
    supporters*: seq[string]

  AppendRequest* = object
    leaderId*: string
    term*: uint64
    kind*: ControlEntryKind
    commandId*: string
    payload*: string
    targetVoters*: seq[string]

  CommitRequest* = object
    index*: uint64
    leaderId*: string
    term*: uint64
    supporters*: seq[string]

  ControlStoreAction* = object
    kind*: ControlStoreActionKind
    expectedMetadataRevision*: uint64
    previousIndex*: uint64

  ControlPlan* = object
    effect*: ControlEffect
    error*: ControlStateError
    beforeState*: ControlState
    nextState*: ControlState
    actions*: seq[ControlStoreAction]
    appliedEntry*: Option[ControlEntry]

  ControlDecision* = object
    effect*: ControlEffect
    error*: ControlStateError
    state*: ControlState
    appliedEntry*: Option[ControlEntry]

  RecoveryInput* = object
    metadataRevision*: uint64
    term*: uint64
    votedFor*: Option[string]
    commitIndex*: uint64
    appliedIndex*: uint64
    initialVoters*: seq[string]
    snapshot*: Option[ControlSnapshotState]
    entries*: seq[ControlEntry]

  ControlRecovery* = object
    error*: ControlStateError
    state*: ControlState

proc validVoterSet(voters: seq[string]): bool =
  if voters.len == 0:
    return false
  var seen = initHashSet[string]()
  for voter in voters:
    if voter.len == 0 or voter in seen:
      return false
    seen.incl(voter)
  true

proc majority(voters: seq[string]): int =
  voters.len div 2 + 1

proc supportersIn(supporters, voters: seq[string]): int =
  var seen = initHashSet[string]()
  for supporter in supporters:
    if supporter in voters:
      seen.incl(supporter)
  seen.len

proc hasQuorum*(
    phase: ControlVoterPhase; oldVoters, newVoters, supporters: seq[string]
): bool =
  case phase
  of cvStableOld:
    supportersIn(supporters, oldVoters) >= majority(oldVoters)
  of cvJoint:
    supportersIn(supporters, oldVoters) >= majority(oldVoters) and
      supportersIn(supporters, newVoters) >= majority(newVoters)
  of cvStableNew:
    supportersIn(supporters, newVoters) >= majority(newVoters)

proc activeVoters(state: ControlState): seq[string] =
  case state.phase
  of cvStableOld:
    result = state.oldVoters
  of cvJoint:
    result = state.oldVoters
    for voter in state.newVoters:
      if voter notin result:
        result.add(voter)
  of cvStableNew:
    result = state.newVoters

proc validStateShape(state: ControlState): bool =
  if not state.valid or state.voterEpoch == 0 or
      not validVoterSet(state.oldVoters) or state.appliedIndex > state.commitIndex or
      state.commitIndex > state.lastIndex:
    return false
  if state.snapshot.isSome and
      state.snapshot.get().lastIncludedIndex > state.appliedIndex:
    return false
  case state.phase
  of cvStableOld:
    state.newVoters.len == 0
  of cvJoint, cvStableNew:
    validVoterSet(state.newVoters)

proc initControlState*(voters: seq[string]): ControlState =
  ControlState(
    valid: validVoterSet(voters),
    voterEpoch: 1,
    phase: cvStableOld,
    oldVoters: voters,
  )

proc reject(state: ControlState; error: ControlStateError): ControlPlan =
  ControlPlan(
    effect: ceReject,
    error: error,
    beforeState: state,
    nextState: state,
  )

proc plan(
    beforeState, nextState: ControlState; effect: ControlEffect;
    actions: seq[ControlStoreAction]; appliedEntry = none(ControlEntry)
): ControlPlan =
  ControlPlan(
    effect: effect,
    error: cseNone,
    beforeState: beforeState,
    nextState: nextState,
    actions: actions,
    appliedEntry: appliedEntry,
  )

proc nextMetadataRevision(state: ControlState): Option[uint64] =
  if state.metadataRevision == high(uint64):
    return none(uint64)
  some(state.metadataRevision + 1)

proc normalizedProof(state: ControlState; supporters: seq[string]): seq[string] =
  for voter in activeVoters(state):
    if voter in supporters:
      result.add(voter)

proc planElection*(state: ControlState; request: ElectionRequest): ControlPlan =
  if not validStateShape(state):
    return reject(state, cseInvalidVoters)
  if request.term < state.term or
      (request.term == state.term and state.votedFor.isSome and
      state.votedFor.get() != request.candidateId):
    return reject(state, cseStaleTerm)
  if request.candidateId.len == 0 or request.candidateId notin activeVoters(state):
    return reject(state, cseCandidateNotVoter)
  if not hasQuorum(state.phase, state.oldVoters, state.newVoters, request.supporters):
    return reject(state, cseQuorumRequired)
  let revision = nextMetadataRevision(state)
  if revision.isNone:
    return reject(state, cseFactConflict)

  var next = state
  next.metadataRevision = revision.get()
  next.term = request.term
  next.votedFor = some(request.candidateId)
  next.leaderId = request.candidateId
  next.leaderTerm = request.term
  next.leaderProof = normalizedProof(state, request.supporters)
  plan(
    state,
    next,
    ceElectLeader,
    @[
      ControlStoreAction(
        kind: cpaMetadata,
        expectedMetadataRevision: state.metadataRevision,
      )
    ],
  )

proc entryAllowed(state: ControlState; request: AppendRequest): bool =
  case request.kind
  of cekCommand:
    request.targetVoters.len == 0
  of cekBeginJoint:
    state.phase == cvStableOld and validVoterSet(request.targetVoters)
  of cekFinalize:
    state.phase == cvJoint and request.targetVoters.len == 0

proc planAppend*(state: ControlState; request: AppendRequest): ControlPlan =
  if not validStateShape(state):
    return reject(state, cseInvalidVoters)
  if state.leaderId.len == 0 or request.leaderId != state.leaderId:
    return reject(state, cseLeaderRequired)
  if request.term != state.term or state.leaderTerm != state.term or
      not hasQuorum(
        state.phase, state.oldVoters, state.newVoters, state.leaderProof
      ):
    return reject(state, cseAuthorityStale)
  if state.lastIndex != state.commitIndex:
    return reject(state, csePendingEntry)
  if request.commandId.len == 0:
    return reject(state, cseCommandRequired)
  if not entryAllowed(state, request):
    return reject(state, cseEntryKindInvalid)
  if state.lastIndex == high(uint64):
    return reject(state, cseFactConflict)

  let entry = ControlEntry(
    index: state.lastIndex + 1,
    term: state.term,
    voterEpoch: state.voterEpoch,
    kind: request.kind,
    commandId: request.commandId,
    payload: request.payload,
    targetVoters: request.targetVoters,
  )
  var next = state
  next.log = state.log & @[entry]
  next.lastIndex = entry.index
  plan(
    state,
    next,
    ceAppend,
    @[ControlStoreAction(kind: cpaLog, previousIndex: state.lastIndex)],
  )

proc entryAt(state: ControlState; index: uint64): Option[ControlEntry] =
  for entry in state.log:
    if entry.index == index:
      return some(entry)
  none(ControlEntry)

proc planCommit*(state: ControlState; request: CommitRequest): ControlPlan =
  if not validStateShape(state):
    return reject(state, cseInvalidVoters)
  if state.leaderId.len == 0 or request.leaderId != state.leaderId:
    return reject(state, cseLeaderRequired)
  if request.term != state.term or state.leaderTerm != state.term or
      not hasQuorum(
        state.phase, state.oldVoters, state.newVoters, state.leaderProof
      ):
    return reject(state, cseAuthorityStale)
  if request.index != state.commitIndex + 1 or request.index > state.lastIndex:
    return reject(state, cseCommitOrder)
  let pending = entryAt(state, request.index)
  if pending.isNone:
    return reject(state, cseLogGap)
  if not hasQuorum(state.phase, state.oldVoters, state.newVoters, request.supporters):
    return reject(state, cseQuorumRequired)
  let revision = nextMetadataRevision(state)
  if revision.isNone:
    return reject(state, cseFactConflict)

  let entry = pending.get()
  if entry.voterEpoch != state.voterEpoch:
    return reject(state, cseAuthorityStale)
  var next = state
  next.metadataRevision = revision.get()
  next.commitIndex = request.index
  case entry.kind
  of cekCommand:
    discard
  of cekBeginJoint:
    if state.phase != cvStableOld or not validVoterSet(entry.targetVoters) or
        state.voterEpoch == high(uint64):
      return reject(state, cseEntryKindInvalid)
    next.phase = cvJoint
    next.newVoters = entry.targetVoters
    next.voterEpoch = state.voterEpoch + 1
    next.leaderId = ""
    next.leaderTerm = 0
    next.leaderProof = @[]
  of cekFinalize:
    if state.phase != cvJoint or state.voterEpoch == high(uint64):
      return reject(state, cseEntryKindInvalid)
    next.phase = cvStableNew
    next.voterEpoch = state.voterEpoch + 1
    next.leaderId = ""
    next.leaderTerm = 0
    next.leaderProof = @[]
  plan(
    state,
    next,
    ceCommit,
    @[
      ControlStoreAction(
        kind: cpaMetadata,
        expectedMetadataRevision: state.metadataRevision,
      )
    ],
  )

proc planApply*(state: ControlState): ControlPlan =
  if state.appliedIndex >= state.commitIndex:
    return reject(state, cseApplyOrder)
  let entry = entryAt(state, state.appliedIndex + 1)
  if entry.isNone:
    return reject(state, cseLogGap)
  let revision = nextMetadataRevision(state)
  if revision.isNone:
    return reject(state, cseFactConflict)

  var next = state
  next.metadataRevision = revision.get()
  next.appliedIndex = state.appliedIndex + 1
  plan(
    state,
    next,
    ceApply,
    @[
      ControlStoreAction(
        kind: cpaMetadata,
        expectedMetadataRevision: state.metadataRevision,
      )
    ],
    entry,
  )

proc planSnapshot*(state: ControlState; statePayload: string): ControlPlan =
  let snapshotIndex = if state.snapshot.isSome:
      state.snapshot.get().lastIncludedIndex
    else:
      0'u64
  # ponytail: v1 snapshots only a fully applied prefix. Persist historical
  # authority-at-index only if pipelined apply becomes necessary.
  if state.appliedIndex == 0 or state.appliedIndex <= snapshotIndex or
      state.appliedIndex != state.commitIndex or statePayload.len == 0:
    return reject(state, cseSnapshotUnavailable)
  let included = entryAt(state, state.appliedIndex)
  if included.isNone:
    return reject(state, cseLogGap)
  let revision = nextMetadataRevision(state)
  if revision.isNone:
    return reject(state, cseFactConflict)

  let snapshot = ControlSnapshotState(
    lastIncludedIndex: state.appliedIndex,
    lastIncludedTerm: included.get().term,
    voterEpoch: state.voterEpoch,
    phase: state.phase,
    oldVoters: state.oldVoters,
    newVoters: state.newVoters,
    statePayload: statePayload,
  )
  var next = state
  next.metadataRevision = revision.get()
  next.snapshot = some(snapshot)
  next.log = state.log.filterIt(it.index > snapshot.lastIncludedIndex)
  plan(
    state,
    next,
    ceSnapshot,
    @[
      ControlStoreAction(
        kind: cpaSnapshot,
        expectedMetadataRevision: state.metadataRevision,
      )
    ],
  )

proc settleControlPlan*(plan: ControlPlan; storeSucceeded: bool): ControlDecision =
  if plan.error != cseNone:
    return ControlDecision(
      effect: ceReject,
      error: plan.error,
      state: plan.beforeState,
    )
  if plan.actions.len > 0 and not storeSucceeded:
    return ControlDecision(
      effect: ceReject,
      error: cseStoreFailure,
      state: plan.beforeState,
    )
  ControlDecision(
    effect: plan.effect,
    error: cseNone,
    state: plan.nextState,
    appliedEntry: plan.appliedEntry,
  )

proc recoveryFailure(): ControlRecovery =
  ControlRecovery(error: cseCorruptRecovery)

proc applyRecoveredConfiguration(state: var ControlState; entry: ControlEntry): bool =
  if entry.voterEpoch != state.voterEpoch:
    return false
  case entry.kind
  of cekCommand:
    true
  of cekBeginJoint:
    if state.phase != cvStableOld or not validVoterSet(entry.targetVoters) or
        state.voterEpoch == high(uint64):
      return false
    state.phase = cvJoint
    state.newVoters = entry.targetVoters
    state.voterEpoch += 1
    true
  of cekFinalize:
    if state.phase != cvJoint or state.voterEpoch == high(uint64):
      return false
    state.phase = cvStableNew
    state.voterEpoch += 1
    true

proc recoverControlState*(input: RecoveryInput): ControlRecovery =
  var state: ControlState
  if input.snapshot.isSome:
    let snapshot = input.snapshot.get()
    if snapshot.lastIncludedIndex == 0 or snapshot.lastIncludedTerm == 0 or
        snapshot.lastIncludedTerm > input.term or snapshot.voterEpoch == 0:
      return recoveryFailure()
    state = ControlState(
      valid: true,
      voterEpoch: snapshot.voterEpoch,
      phase: snapshot.phase,
      oldVoters: snapshot.oldVoters,
      newVoters: snapshot.newVoters,
      lastIndex: snapshot.lastIncludedIndex,
      commitIndex: snapshot.lastIncludedIndex,
      appliedIndex: snapshot.lastIncludedIndex,
      snapshot: input.snapshot,
    )
  else:
    state = initControlState(input.initialVoters)
  if not validStateShape(state):
    return recoveryFailure()

  state.metadataRevision = input.metadataRevision
  state.term = input.term
  state.votedFor = input.votedFor
  state.leaderId = ""
  state.leaderTerm = 0
  state.leaderProof = @[]
  let snapshotIndex = state.lastIndex
  var expected = snapshotIndex
  for entry in input.entries:
    if expected == high(uint64) or entry.index != expected + 1 or
        entry.term == 0 or entry.term > input.term or entry.commandId.len == 0:
      return recoveryFailure()
    expected = entry.index
    if entry.index <= input.commitIndex and not applyRecoveredConfiguration(state, entry):
      return recoveryFailure()
  state.log = input.entries
  state.lastIndex = expected
  if input.appliedIndex < snapshotIndex or input.appliedIndex > input.commitIndex or
      input.commitIndex > state.lastIndex:
    return recoveryFailure()
  state.commitIndex = input.commitIndex
  state.appliedIndex = input.appliedIndex
  ControlRecovery(error: cseNone, state: state)
