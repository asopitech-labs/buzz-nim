## Presence and typing convergence policy.
##
## Rust supplies authenticated transport facts. Nimino owns validation,
## total ordering, replay handling, TTL, tombstones, and pruning.

import std/options

const
  MaxEphemeralScopeBytes* = 128
  MaxEphemeralContextBytes* = 128
  MaxEphemeralValueBytes* = 512
  MaxEphemeralTtlSecs* = 600'u64

type
  EphemeralKind* = enum
    ekPresence
    ekTyping

  EphemeralPolicyError* = enum
    epeNone
    epeInvalidState
    epeInvalidScope
    epeInvalidSubject
    epeInvalidContext
    epeInvalidValue
    epeInvalidTime
    epeTtlOutOfRange
    epeInvalidOrigin
    epeInvalidTransition

  EphemeralEffect* = enum
    eeApply
    eeReplay
    eeStale
    eeExpired
    eeReject

  EphemeralState* = object
    scope*: string
    kind*: EphemeralKind
    subject*: string
    context*: string
    value*: string
    active*: bool
    observedAtMs*: uint64
    expiresAtMs*: uint64
    originNodeId*: string
    transitionId*: string

  EphemeralCommand* = object
    scope*: string
    kind*: EphemeralKind
    subject*: string
    context*: string
    value*: string
    active*: bool
    observedAtMs*: uint64
    ttlSecs*: uint64
    originNodeId*: string
    transitionId*: string

  EphemeralDecision* = object
    effect*: EphemeralEffect
    error*: EphemeralPolicyError
    state*: Option[EphemeralState]

  EphemeralPruneDecision* = object
    retained*: seq[EphemeralState]
    error*: EphemeralPolicyError

proc canonicalHex(value: string; length: int): bool =
  if value.len != length:
    return false
  for character in value:
    if character notin {'0' .. '9', 'a' .. 'f'}:
      return false
  true

proc validKey(
    scope: string; kind: EphemeralKind; subject, context: string
): EphemeralPolicyError =
  if scope.len notin 1 .. MaxEphemeralScopeBytes:
    return epeInvalidScope
  if not canonicalHex(subject, 64):
    return epeInvalidSubject
  case kind
  of ekPresence:
    if context.len != 0:
      return epeInvalidContext
  of ekTyping:
    if context.len notin 1 .. MaxEphemeralContextBytes:
      return epeInvalidContext
  epeNone

proc validState(state: EphemeralState): bool =
  validKey(state.scope, state.kind, state.subject, state.context) == epeNone and
    ((state.active and state.value.len in 1 .. MaxEphemeralValueBytes) or
      (not state.active and state.value.len == 0)) and
    state.observedAtMs > 0 and state.expiresAtMs >= state.observedAtMs and
    canonicalHex(state.originNodeId, 32) and canonicalHex(state.transitionId, 64)

proc sameKey(state: EphemeralState; command: EphemeralCommand): bool =
  state.scope == command.scope and state.kind == command.kind and
    state.subject == command.subject and state.context == command.context and
    state.originNodeId == command.originNodeId

proc compareVersion(left, right: EphemeralState): int =
  if left.observedAtMs != right.observedAtMs:
    return if left.observedAtMs < right.observedAtMs: -1 else: 1
  if left.originNodeId != right.originNodeId:
    return if left.originNodeId < right.originNodeId: -1 else: 1
  if left.transitionId != right.transitionId:
    return if left.transitionId < right.transitionId: -1 else: 1
  0

proc reject(
    state: Option[EphemeralState]; error: EphemeralPolicyError
): EphemeralDecision =
  EphemeralDecision(effect: eeReject, error: error, state: state)

proc applyEphemeral*(
    state: Option[EphemeralState]; command: EphemeralCommand; nowMs: uint64
): EphemeralDecision =
  let keyError = validKey(command.scope, command.kind, command.subject, command.context)
  if keyError != epeNone:
    return reject(state, keyError)
  if (command.active and command.value.len notin 1 .. MaxEphemeralValueBytes) or
      (not command.active and command.value.len != 0):
    return reject(state, epeInvalidValue)
  if command.observedAtMs == 0 or nowMs == 0:
    return reject(state, epeInvalidTime)
  if command.ttlSecs notin 1'u64 .. MaxEphemeralTtlSecs:
    return reject(state, epeTtlOutOfRange)
  if not canonicalHex(command.originNodeId, 32):
    return reject(state, epeInvalidOrigin)
  if not canonicalHex(command.transitionId, 64):
    return reject(state, epeInvalidTransition)
  if command.ttlSecs > (high(uint64) - command.observedAtMs) div 1000'u64:
    return reject(state, epeInvalidTime)
  if state.isSome and (not validState(state.get()) or not sameKey(state.get(), command)):
    return reject(state, epeInvalidState)

  let candidate = EphemeralState(
    scope: command.scope,
    kind: command.kind,
    subject: command.subject,
    context: command.context,
    value: command.value,
    active: command.active,
    observedAtMs: command.observedAtMs,
    expiresAtMs: command.observedAtMs + command.ttlSecs * 1000'u64,
    originNodeId: command.originNodeId,
    transitionId: command.transitionId,
  )
  let current = if state.isSome and state.get().expiresAtMs >= nowMs:
      state
    else:
      none(EphemeralState)
  if candidate.expiresAtMs < nowMs:
    return EphemeralDecision(effect: eeExpired, error: epeNone, state: current)
  if current.isSome:
    let ordering = compareVersion(candidate, current.get())
    if ordering == 0:
      return EphemeralDecision(
        effect: if candidate == current.get(): eeReplay else: eeReject,
        error: if candidate == current.get(): epeNone else: epeInvalidTransition,
        state: current,
      )
    if ordering < 0:
      return EphemeralDecision(effect: eeStale, error: epeNone, state: current)
  EphemeralDecision(effect: eeApply, error: epeNone, state: some(candidate))

proc pruneEphemeral*(
    states: seq[EphemeralState]; beforeMs: uint64
): EphemeralPruneDecision =
  if beforeMs == 0:
    return EphemeralPruneDecision(error: epeInvalidTime)
  for state in states:
    if not validState(state):
      return EphemeralPruneDecision(error: epeInvalidState)
    if state.expiresAtMs >= beforeMs:
      result.retained.add(state)
  result.error = epeNone
