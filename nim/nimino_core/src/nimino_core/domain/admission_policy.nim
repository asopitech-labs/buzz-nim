## Cluster-wide request admission decisions.
##
## Chirps and the Rust host carry committed facts. Nimino owns replay claim,
## expiry, and pruning semantics.

import std/options

const
  MinReplayTtlSecs* = 120'u64
  MaxReplayTtlSecs* = 3600'u64
  MaxReplayScopeBytes* = 128
  MaxAdmissionNamespaceBytes* = 32
  MaxAdmissionKeyBytes* = 512
  MaxRateWindowSecs* = 3600'u64
  MaxRateBatch* = 64
  MaxInvalidationChannelBytes* = 128
  MaxInvalidationFactBytes* = 128

type
  AdmissionPolicyError* = enum
    apeNone
    apeInvalidState
    apeInvalidScope
    apeInvalidEventId
    apeInvalidTime
    apeTtlOutOfRange
    apeControlReplay
    apeInvalidNamespace
    apeInvalidKey
    apeInvalidWindow
    apePolicyConflict
    apeClockRegression
    apeCounterOverflow
    apeInvalidBatch

  ReplayClaimState* = object
    scope*: string
    eventId*: string
    expiresAtMs*: uint64
    lastControlIndex*: uint64

  ReplayClaimCommand* = object
    scope*: string
    eventId*: string
    observedAtMs*: uint64
    ttlSecs*: uint64

  ReplayClaimDecision* = object
    allowed*: bool
    error*: AdmissionPolicyError
    state*: Option[ReplayClaimState]

  ReplayPruneDecision* = object
    retained*: seq[ReplayClaimState]
    error*: AdmissionPolicyError

  RateLimitState* = object
    namespace*: string
    key*: string
    windowStartedAtMs*: uint64
    windowSecs*: uint64
    limit*: uint64
    count*: uint64
    lastControlIndex*: uint64

  RateLimitCommand* = object
    namespace*: string
    key*: string
    observedAtMs*: uint64
    windowSecs*: uint64
    limit*: uint64

  RateLimitDecision* = object
    allowed*: bool
    current*: uint64
    limit*: uint64
    resetInSecs*: uint64
    error*: AdmissionPolicyError
    state*: Option[RateLimitState]

  RateLimitBatchDecision* = object
    results*: seq[RateLimitDecision]
    error*: AdmissionPolicyError

  AuthorizationInvalidationKind* = enum
    aikBan
    aikMembership
    aikVisibility
    aikCommunity

  AuthorizationInvalidationError* = enum
    aieNone
    aieInvalidState
    aieInvalidScope
    aieInvalidSubject
    aieInvalidChannel
    aieInvalidFact
    aieInvalidRevision

  AuthorizationInvalidationEffect* = enum
    aieApply
    aieReplay
    aieStale
    aieReject

  AuthorizationInvalidationState* = object
    scope*: string
    kind*: AuthorizationInvalidationKind
    subject*: string
    channelId*: string
    factId*: string
    revision*: uint64

  AuthorizationInvalidationCommand* = object
    scope*: string
    kind*: AuthorizationInvalidationKind
    subject*: string
    channelId*: string
    factId*: string

  AuthorizationInvalidationDecision* = object
    effect*: AuthorizationInvalidationEffect
    error*: AuthorizationInvalidationError
    state*: Option[AuthorizationInvalidationState]

proc canonicalEventId(value: string): bool =
  if value.len != 64:
    return false
  for character in value:
    if character notin {'0' .. '9', 'a' .. 'f'}:
      return false
  true

proc validScope(value: string): bool =
  value.len in 1 .. MaxReplayScopeBytes

proc validState(state: ReplayClaimState): bool =
  validScope(state.scope) and canonicalEventId(state.eventId) and
    state.expiresAtMs > 0 and state.lastControlIndex > 0

proc reject(
    state: Option[ReplayClaimState]; error: AdmissionPolicyError
): ReplayClaimDecision =
  ReplayClaimDecision(error: error, state: state)

proc applyReplayClaim*(
    state: Option[ReplayClaimState]; command: ReplayClaimCommand;
    controlIndex: uint64
): ReplayClaimDecision =
  if not validScope(command.scope):
    return reject(state, apeInvalidScope)
  if not canonicalEventId(command.eventId):
    return reject(state, apeInvalidEventId)
  if command.observedAtMs == 0:
    return reject(state, apeInvalidTime)
  if command.ttlSecs notin MinReplayTtlSecs .. MaxReplayTtlSecs:
    return reject(state, apeTtlOutOfRange)
  if controlIndex == 0:
    return reject(state, apeControlReplay)
  if state.isSome:
    let current = state.get()
    if not validState(current) or current.scope != command.scope or
        current.eventId != command.eventId:
      return reject(state, apeInvalidState)
    if controlIndex <= current.lastControlIndex:
      return reject(state, apeControlReplay)
    # Inclusive expiry preserves the full 120-second NIP-98 skew window.
    if current.expiresAtMs >= command.observedAtMs:
      return ReplayClaimDecision(error: apeNone, state: state)
  if command.ttlSecs > (high(uint64) - command.observedAtMs) div 1000'u64:
    return reject(state, apeInvalidTime)
  ReplayClaimDecision(
    allowed: true,
    error: apeNone,
    state: some(
      ReplayClaimState(
        scope: command.scope,
        eventId: command.eventId,
        expiresAtMs: command.observedAtMs + command.ttlSecs * 1000'u64,
        lastControlIndex: controlIndex,
      )
    ),
  )

proc pruneReplayClaims*(
    states: seq[ReplayClaimState]; beforeMs: uint64
): ReplayPruneDecision =
  if beforeMs == 0:
    return ReplayPruneDecision(error: apeInvalidTime)
  for state in states:
    if not validState(state):
      return ReplayPruneDecision(error: apeInvalidState)
    if state.expiresAtMs >= beforeMs:
      result.retained.add(state)
  result.error = apeNone

proc validRateState(state: RateLimitState): bool =
  state.namespace.len in 1 .. MaxAdmissionNamespaceBytes and
    state.key.len in 1 .. MaxAdmissionKeyBytes and state.windowStartedAtMs > 0 and
    state.windowSecs in 1'u64 .. MaxRateWindowSecs and state.count > 0 and
    state.lastControlIndex > 0

proc rejectRate(
    state: Option[RateLimitState]; error: AdmissionPolicyError
): RateLimitDecision =
  RateLimitDecision(error: error, state: state)

proc applyRateLimitStep(
    state: Option[RateLimitState]; command: RateLimitCommand;
    controlIndex: uint64; sameControlEntry: bool
): RateLimitDecision =
  if command.namespace.len notin 1 .. MaxAdmissionNamespaceBytes:
    return rejectRate(state, apeInvalidNamespace)
  if command.key.len notin 1 .. MaxAdmissionKeyBytes:
    return rejectRate(state, apeInvalidKey)
  if command.observedAtMs == 0:
    return rejectRate(state, apeInvalidTime)
  if command.windowSecs notin 1'u64 .. MaxRateWindowSecs:
    return rejectRate(state, apeInvalidWindow)
  if controlIndex == 0:
    return rejectRate(state, apeControlReplay)

  let windowMs = command.windowSecs * 1000'u64
  let windowStartedAtMs = command.observedAtMs div windowMs * windowMs
  if windowStartedAtMs == 0 or windowStartedAtMs > high(uint64) - windowMs:
    return rejectRate(state, apeInvalidTime)

  var count = 1'u64
  if state.isSome:
    let current = state.get()
    if not validRateState(current) or current.namespace != command.namespace or
        current.key != command.key:
      return rejectRate(state, apeInvalidState)
    if controlIndex < current.lastControlIndex or
        (controlIndex == current.lastControlIndex and not sameControlEntry):
      return rejectRate(state, apeControlReplay)
    if windowStartedAtMs < current.windowStartedAtMs:
      return rejectRate(state, apeClockRegression)
    if windowStartedAtMs == current.windowStartedAtMs:
      if current.windowSecs != command.windowSecs or current.limit != command.limit:
        return rejectRate(state, apePolicyConflict)
      if current.count == high(uint64):
        return rejectRate(state, apeCounterOverflow)
      count = current.count + 1

  let remainingMs = windowStartedAtMs + windowMs - command.observedAtMs
  let resetInSecs = (remainingMs + 999'u64) div 1000'u64
  let next = RateLimitState(
    namespace: command.namespace,
    key: command.key,
    windowStartedAtMs: windowStartedAtMs,
    windowSecs: command.windowSecs,
    limit: command.limit,
    count: count,
    lastControlIndex: controlIndex,
  )
  RateLimitDecision(
    allowed: count <= command.limit,
    current: count,
    limit: command.limit,
    resetInSecs: resetInSecs,
    error: apeNone,
    state: some(next),
  )

proc applyRateLimit*(
    state: Option[RateLimitState]; command: RateLimitCommand;
    controlIndex: uint64
): RateLimitDecision =
  applyRateLimitStep(state, command, controlIndex, false)

proc applyRateLimitBatch*(
    states: seq[RateLimitState]; commands: seq[RateLimitCommand];
    controlIndex: uint64
): RateLimitBatchDecision =
  if commands.len notin 1 .. MaxRateBatch or states.len > MaxRateBatch:
    return RateLimitBatchDecision(error: apeInvalidBatch)
  if controlIndex == 0:
    return RateLimitBatchDecision(error: apeControlReplay)

  var projected = states
  for index, state in projected:
    if not validRateState(state):
      return RateLimitBatchDecision(error: apeInvalidState)
    if state.lastControlIndex >= controlIndex:
      return RateLimitBatchDecision(error: apeControlReplay)
    for prior in 0 ..< index:
      if projected[prior].namespace == state.namespace and
          projected[prior].key == state.key:
        return RateLimitBatchDecision(error: apeInvalidState)

  for command in commands:
    var stateIndex = -1
    for index, state in projected:
      if state.namespace == command.namespace and state.key == command.key:
        stateIndex = index
        break
    let current = if stateIndex < 0:
        none(RateLimitState)
      else:
        some(projected[stateIndex])
    let decision = applyRateLimitStep(current, command, controlIndex, true)
    if decision.error != apeNone:
      return RateLimitBatchDecision(error: decision.error)
    if decision.state.isNone:
      return RateLimitBatchDecision(error: apeInvalidState)
    if stateIndex < 0:
      projected.add(decision.state.get())
    else:
      projected[stateIndex] = decision.state.get()
    result.results.add(decision)
  result.error = apeNone

proc validInvalidationKey(
    scope: string; kind: AuthorizationInvalidationKind; subject, channelId: string
): AuthorizationInvalidationError =
  if not validScope(scope):
    return aieInvalidScope
  case kind
  of aikBan:
    if not canonicalEventId(subject):
      return aieInvalidSubject
    if channelId.len != 0:
      return aieInvalidChannel
  of aikMembership:
    if not canonicalEventId(subject):
      return aieInvalidSubject
    if channelId.len notin 1 .. MaxInvalidationChannelBytes:
      return aieInvalidChannel
  of aikVisibility:
    if subject.len != 0:
      return aieInvalidSubject
    if channelId.len notin 1 .. MaxInvalidationChannelBytes:
      return aieInvalidChannel
  of aikCommunity:
    if subject.len != 0:
      return aieInvalidSubject
    if channelId.len != 0:
      return aieInvalidChannel
  aieNone

proc validInvalidationState(state: AuthorizationInvalidationState): bool =
  validInvalidationKey(state.scope, state.kind, state.subject, state.channelId) ==
    aieNone and state.factId.len in 1 .. MaxInvalidationFactBytes and
    state.revision > 0

proc sameInvalidationKey(
    state: AuthorizationInvalidationState; command: AuthorizationInvalidationCommand
): bool =
  state.scope == command.scope and state.kind == command.kind and
    state.subject == command.subject and state.channelId == command.channelId

proc applyAuthorizationInvalidation*(
    state: Option[AuthorizationInvalidationState];
    command: AuthorizationInvalidationCommand; revision: uint64
): AuthorizationInvalidationDecision =
  let keyError = validInvalidationKey(
    command.scope, command.kind, command.subject, command.channelId
  )
  if keyError != aieNone:
    return AuthorizationInvalidationDecision(effect: aieReject, error: keyError, state: state)
  if command.factId.len notin 1 .. MaxInvalidationFactBytes:
    return AuthorizationInvalidationDecision(
      effect: aieReject, error: aieInvalidFact, state: state
    )
  if revision == 0:
    return AuthorizationInvalidationDecision(
      effect: aieReject, error: aieInvalidRevision, state: state
    )
  if state.isSome:
    let current = state.get()
    if not validInvalidationState(current) or not sameInvalidationKey(current, command):
      return AuthorizationInvalidationDecision(
        effect: aieReject, error: aieInvalidState, state: state
      )
    if revision < current.revision:
      return AuthorizationInvalidationDecision(effect: aieStale, error: aieNone, state: state)
    if revision == current.revision:
      return AuthorizationInvalidationDecision(
        effect: if current.factId == command.factId: aieReplay else: aieReject,
        error: if current.factId == command.factId: aieNone else: aieInvalidRevision,
        state: state,
      )
  AuthorizationInvalidationDecision(
    effect: aieApply,
    error: aieNone,
    state: some(
      AuthorizationInvalidationState(
        scope: command.scope,
        kind: command.kind,
        subject: command.subject,
        channelId: command.channelId,
        factId: command.factId,
        revision: revision,
      )
    ),
  )
