## Lease, fencing, ownership, and routing decisions owned by Nimino.
##
## Lease grants become authoritative only when applied from a committed
## control entry. Chirps transports facts; adapters supply monotonic time and
## execute an allowed route or side effect.

import std/[algorithm, options, sets]

type
  LeaseApplyMode* = enum
    lamLive
    lamRecovery

  LeaseEffect* = enum
    leReject
    lePropose
    leActivate
    leReplay
    leRoute
    leAuthorize

  LeaseFenceError* = enum
    lfeNone
    lfeInvalidState
    lfeTransitionRequired
    lfeInvalidEligibleOwners
    lfeLeaseDurationInvalid
    lfeQuorumUnavailable
    lfeAuthorityInvalid
    lfeControlNotCommitted
    lfeAuthorityStale
    lfeControlReplay
    lfeStaleFence
    lfeFutureFence
    lfeOwnerMismatch
    lfeResourceMismatch
    lfeNoActiveLease
    lfeClockEpochMismatch
    lfeClockRegression
    lfeLeaseExpired
    lfeReplayConflict
    lfeTickOverflow

  LeaseAuthority* = object
    leaderId*: string
    term*: uint64
    voterEpoch*: uint64
    quorumAvailable*: bool

  LeaseCommand* = object
    resourceId*: string
    transitionId*: string
    ownerId*: string
    eligibleOwners*: seq[string]
    expectedPreviousFence*: uint64
    fenceToken*: uint64
    durationTicks*: uint64
    leaderId*: string
    term*: uint64
    voterEpoch*: uint64

  ActiveLease* = object
    resourceId*: string
    ownerId*: string
    fenceToken*: uint64
    leaderId*: string
    term*: uint64
    voterEpoch*: uint64
    clockEpoch*: string
    activatedAtTick*: uint64
    expiresAtTick*: uint64

  LeaseState* = object
    valid*: bool
    resourceId*: string
    lastFenceToken*: uint64
    lastControlIndex*: uint64
    lastCommand*: Option[LeaseCommand]
    activeLease*: Option[ActiveLease]

  LeasePlan* = object
    effect*: LeaseEffect
    error*: LeaseFenceError
    beforeState*: LeaseState
    command*: Option[LeaseCommand]

  CommittedLeaseFact* = object
    committed*: bool
    controlIndex*: uint64
    leaderId*: string
    term*: uint64
    voterEpoch*: uint64
    clockEpoch*: string
    nowTick*: uint64

  ServingLeaseFact* = object
    quorumAvailable*: bool
    leaderId*: string
    term*: uint64
    voterEpoch*: uint64
    clockEpoch*: string
    nowTick*: uint64

  SingletonEffectAttempt* = object
    resourceId*: string
    ownerId*: string
    fenceToken*: uint64

  LeaseDecision* = object
    effect*: LeaseEffect
    error*: LeaseFenceError
    state*: LeaseState

  LeaseRoute* = object
    allowed*: bool
    error*: LeaseFenceError
    ownerId*: string
    fenceToken*: uint64

  SingletonEffectDecision* = object
    allowed*: bool
    error*: LeaseFenceError

proc normalizedOwners(owners: seq[string]): tuple[valid: bool, owners: seq[string]] =
  var seen = initHashSet[string]()
  for owner in owners:
    if owner.len == 0:
      return
    if owner notin seen:
      seen.incl(owner)
      result.owners.add(owner)
  result.owners.sort()
  result.valid = result.owners.len > 0

proc validAuthority(authority: LeaseAuthority): bool =
  authority.leaderId.len > 0 and authority.term > 0 and authority.voterEpoch > 0

proc validCommand(command: LeaseCommand): bool =
  let normalized = normalizedOwners(command.eligibleOwners)
  command.resourceId.len > 0 and command.transitionId.len > 0 and
    command.durationTicks > 0 and command.fenceToken > 0 and
    command.expectedPreviousFence < high(uint64) and
    command.fenceToken == command.expectedPreviousFence + 1 and validAuthority(
      LeaseAuthority(
        leaderId: command.leaderId,
        term: command.term,
        voterEpoch: command.voterEpoch,
      )
    ) and normalized.valid and normalized.owners == command.eligibleOwners and
    command.ownerId == normalized.owners[0]

proc validState(state: LeaseState): bool =
  if not state.valid or state.resourceId.len == 0:
    return false
  if state.lastCommand.isNone:
    return state.lastFenceToken == 0 and state.lastControlIndex == 0 and
      state.activeLease.isNone
  let command = state.lastCommand.get()
  if state.lastControlIndex == 0 or not validCommand(command) or
      command.resourceId != state.resourceId or
      command.fenceToken != state.lastFenceToken:
    return false
  if state.activeLease.isSome:
    let active = state.activeLease.get()
    if active.resourceId != state.resourceId or active.ownerId != command.ownerId or
        active.fenceToken != state.lastFenceToken or
        active.leaderId != command.leaderId or active.term != command.term or
        active.voterEpoch != command.voterEpoch or active.clockEpoch.len == 0 or
        active.expiresAtTick <= active.activatedAtTick:
      return false
  true

proc initLeaseState*(resourceId: string): LeaseState =
  LeaseState(valid: resourceId.len > 0, resourceId: resourceId)

proc rejectPlan(state: LeaseState; error: LeaseFenceError): LeasePlan =
  LeasePlan(effect: leReject, error: error, beforeState: state)

proc sameIntent(
    command: LeaseCommand; authority: LeaseAuthority; owners: seq[string];
    transitionId: string; durationTicks: uint64
): bool =
  command.transitionId == transitionId and command.eligibleOwners == owners and
    command.durationTicks == durationTicks and
    command.leaderId == authority.leaderId and command.term == authority.term and
    command.voterEpoch == authority.voterEpoch

proc planLeaseGrant*(
    state: LeaseState; authority: LeaseAuthority; transitionId: string;
    eligibleOwners: seq[string]; durationTicks: uint64
): LeasePlan =
  if not validState(state):
    return rejectPlan(state, lfeInvalidState)
  if transitionId.len == 0:
    return rejectPlan(state, lfeTransitionRequired)
  let normalized = normalizedOwners(eligibleOwners)
  if not normalized.valid:
    return rejectPlan(state, lfeInvalidEligibleOwners)
  if durationTicks == 0:
    return rejectPlan(state, lfeLeaseDurationInvalid)
  if not validAuthority(authority):
    return rejectPlan(state, lfeAuthorityInvalid)
  if not authority.quorumAvailable:
    return rejectPlan(state, lfeQuorumUnavailable)

  if state.lastCommand.isSome and
      state.lastCommand.get().transitionId == transitionId:
    if sameIntent(
      state.lastCommand.get(),
      authority,
      normalized.owners,
      transitionId,
      durationTicks,
    ):
      return LeasePlan(
        effect: leReplay,
        error: lfeNone,
        beforeState: state,
        command: state.lastCommand,
      )
    return rejectPlan(state, lfeReplayConflict)

  if state.lastFenceToken == high(uint64):
    return rejectPlan(state, lfeFutureFence)
  let command = LeaseCommand(
    resourceId: state.resourceId,
    transitionId: transitionId,
    ownerId: normalized.owners[0],
    eligibleOwners: normalized.owners,
    expectedPreviousFence: state.lastFenceToken,
    fenceToken: state.lastFenceToken + 1,
    durationTicks: durationTicks,
    leaderId: authority.leaderId,
    term: authority.term,
    voterEpoch: authority.voterEpoch,
  )
  LeasePlan(
    effect: lePropose,
    error: lfeNone,
    beforeState: state,
    command: some(command),
  )

proc rejectDecision(state: LeaseState; error: LeaseFenceError): LeaseDecision =
  LeaseDecision(effect: leReject, error: error, state: state)

proc applyCommittedLease*(
    state: LeaseState; command: LeaseCommand; fact: CommittedLeaseFact;
    mode: LeaseApplyMode
): LeaseDecision =
  if not validState(state):
    return rejectDecision(state, lfeInvalidState)
  if command.resourceId != state.resourceId:
    return rejectDecision(state, lfeResourceMismatch)
  if command.durationTicks == 0:
    return rejectDecision(state, lfeLeaseDurationInvalid)
  if not validCommand(command):
    return rejectDecision(state, lfeInvalidEligibleOwners)
  if not fact.committed:
    return rejectDecision(state, lfeControlNotCommitted)
  if fact.controlIndex == 0 or fact.leaderId.len == 0 or fact.term == 0 or
      fact.voterEpoch == 0:
    return rejectDecision(state, lfeAuthorityInvalid)
  if command.leaderId != fact.leaderId or command.term != fact.term or
      command.voterEpoch != fact.voterEpoch:
    return rejectDecision(state, lfeAuthorityStale)

  if state.lastCommand.isSome and
      command.transitionId == state.lastCommand.get().transitionId:
    if command == state.lastCommand.get() and
        fact.controlIndex == state.lastControlIndex:
      return LeaseDecision(effect: leReplay, error: lfeNone, state: state)
    return rejectDecision(state, lfeReplayConflict)

  if fact.controlIndex <= state.lastControlIndex:
    return rejectDecision(state, lfeControlReplay)
  if command.fenceToken <= state.lastFenceToken:
    return rejectDecision(state, lfeStaleFence)
  if state.lastFenceToken == high(uint64) or
      command.fenceToken != state.lastFenceToken + 1:
    return rejectDecision(state, lfeFutureFence)
  if command.expectedPreviousFence != state.lastFenceToken:
    return rejectDecision(state, lfeReplayConflict)

  var next = state
  next.lastFenceToken = command.fenceToken
  next.lastControlIndex = fact.controlIndex
  next.lastCommand = some(command)
  next.activeLease = none(ActiveLease)
  if mode == lamLive:
    if fact.clockEpoch.len == 0:
      return rejectDecision(state, lfeClockEpochMismatch)
    if command.durationTicks > high(uint64) - fact.nowTick:
      return rejectDecision(state, lfeTickOverflow)
    next.activeLease = some(
      ActiveLease(
        resourceId: command.resourceId,
        ownerId: command.ownerId,
        fenceToken: command.fenceToken,
        leaderId: command.leaderId,
        term: command.term,
        voterEpoch: command.voterEpoch,
        clockEpoch: fact.clockEpoch,
        activatedAtTick: fact.nowTick,
        expiresAtTick: fact.nowTick + command.durationTicks,
      )
    )
  LeaseDecision(effect: leActivate, error: lfeNone, state: next)

proc currentLeaseError(
    state: LeaseState; fact: ServingLeaseFact
): LeaseFenceError =
  if not validState(state):
    return lfeInvalidState
  if state.activeLease.isNone:
    return lfeNoActiveLease
  if not fact.quorumAvailable:
    return lfeQuorumUnavailable
  let active = state.activeLease.get()
  if fact.leaderId != active.leaderId or fact.term != active.term or
      fact.voterEpoch != active.voterEpoch:
    return lfeAuthorityStale
  if fact.clockEpoch != active.clockEpoch:
    return lfeClockEpochMismatch
  if fact.nowTick < active.activatedAtTick:
    return lfeClockRegression
  if fact.nowTick >= active.expiresAtTick:
    return lfeLeaseExpired
  lfeNone

proc routeSingleton*(state: LeaseState; fact: ServingLeaseFact): LeaseRoute =
  let error = currentLeaseError(state, fact)
  if error != lfeNone:
    return LeaseRoute(error: error)
  let active = state.activeLease.get()
  LeaseRoute(
    allowed: true,
    error: lfeNone,
    ownerId: active.ownerId,
    fenceToken: active.fenceToken,
  )

proc authorizeSingletonEffect*(
    state: LeaseState; attempt: SingletonEffectAttempt; fact: ServingLeaseFact
): SingletonEffectDecision =
  if not validState(state):
    return SingletonEffectDecision(error: lfeInvalidState)
  if attempt.resourceId != state.resourceId:
    return SingletonEffectDecision(error: lfeResourceMismatch)
  if attempt.fenceToken < state.lastFenceToken:
    return SingletonEffectDecision(error: lfeStaleFence)
  if attempt.fenceToken > state.lastFenceToken:
    return SingletonEffectDecision(error: lfeFutureFence)
  if state.activeLease.isNone:
    return SingletonEffectDecision(error: lfeNoActiveLease)
  if attempt.ownerId != state.activeLease.get().ownerId:
    return SingletonEffectDecision(error: lfeOwnerMismatch)
  let error = currentLeaseError(state, fact)
  if error != lfeNone:
    return SingletonEffectDecision(error: error)
  SingletonEffectDecision(allowed: true, error: lfeNone)
