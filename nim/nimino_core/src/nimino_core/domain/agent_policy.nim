## Pure persona, trigger-routing, and agent lifecycle policy.
##
## Adapters parse files and ACP/Nostr frames, supervise processes, and execute
## returned actions. This module performs no I/O.

import std/[options, tables]

import ./workflow_policy

const
  AgentRetryInitialMs* = 5_000'i64
  AgentRetryMaxMs* = 300_000'i64

type
  PersonaTriggers* = object
    mentions*: Option[bool]
    keywords*: Option[seq[string]]
    allMessages*: Option[bool]

  PersonaBehavior* = object
    model*: Option[string]
    temperature*: Option[float64]
    maxContextTokens*: Option[int64]
    subscribe*: Option[seq[string]]
    triggers*: Option[PersonaTriggers]
    threadReplies*: Option[bool]
    broadcastReplies*: Option[bool]

  ResolvedPersonaTriggers* = object
    mentions*: bool
    keywords*: seq[string]
    allMessages*: bool

  ResolvedPersonaBehavior* = object
    model*: Option[string]
    temperature*: Option[float64]
    maxContextTokens*: Option[int64]
    subscribe*: Option[seq[string]]
    triggers*: Option[ResolvedPersonaTriggers]
    threadReplies*: bool
    broadcastReplies*: bool

  AgentEventFacts* = object
    content*: string
    author*: string
    kind*: int64
    channelId*: string
    timestamp*: int64
    mentioned*: bool

  AgentTriggerRule* = object
    name*: string
    allChannels*: bool
    channels*: seq[string]
    kinds*: seq[int64]
    requireMention*: bool
    filter*: string
    promptTag*: string

  AgentTriggerError* = enum
    ateNone
    ateInvalidRule
    ateInvalidFilter

  AgentTriggerDecision* = object
    matched*: bool
    ruleIndex*: int
    promptTag*: string
    error*: AgentTriggerError

  AgentPhase* = enum
    apAbsent
    apStarting
    apReady
    apRunning
    apCancelling
    apRestartWait
    apStopped

  AgentLifecycleCommand* = enum
    alcStart
    alcStarted
    alcStartFailed
    alcBeginTurn
    alcCancel
    alcTurnFinished
    alcCancelTimeout
    alcProcessExited
    alcRetry
    alcShutdown

  AgentLifecycleAction* = enum
    alaReject
    alaNoop
    alaSpawn
    alaAcceptStart
    alaBeginTurn
    alaSendCancel
    alaReturnReady
    alaReapAndWait
    alaStop

  AgentLifecycleError* = enum
    aleNone
    aleInvalidTransition
    aleStaleAttempt
    aleInvalidTurn

  AgentLifecycleState* = object
    phase*: AgentPhase
    attempt*: int64
    retryAtMs*: int64
    turnId*: string

  AgentLifecycleRequest* = object
    state*: AgentLifecycleState
    command*: AgentLifecycleCommand
    commandAttempt*: int64
    commandTurnId*: string
    pendingWork*: bool
    nowMs*: int64

  AgentLifecycleDecision* = object
    allowed*: bool
    error*: AgentLifecycleError
    action*: AgentLifecycleAction
    nextState*: AgentLifecycleState

proc choose[T](primary, fallback: Option[T]): Option[T] =
  if primary.isSome: primary else: fallback

proc optionBool(value: Option[bool]; fallback: bool): bool =
  if value.isSome: value.get() else: fallback

proc resolvePersonaBehavior*(
    persona, defaults: PersonaBehavior
): ResolvedPersonaBehavior =
  let triggerSource =
    if persona.triggers.isSome: persona.triggers
    else: defaults.triggers
  var resolvedTriggers: Option[ResolvedPersonaTriggers]
  if triggerSource.isSome:
    let triggers = triggerSource.get()
    resolvedTriggers = some(ResolvedPersonaTriggers(
      mentions: optionBool(triggers.mentions, true),
      keywords:
        if triggers.keywords.isSome: triggers.keywords.get() else: @[],
      allMessages: optionBool(triggers.allMessages, false),
    ))
  ResolvedPersonaBehavior(
    model: choose(persona.model, defaults.model),
    temperature: choose(persona.temperature, defaults.temperature),
    maxContextTokens: choose(
      persona.maxContextTokens, defaults.maxContextTokens
    ),
    subscribe: choose(persona.subscribe, defaults.subscribe),
    triggers: resolvedTriggers,
    threadReplies: optionBool(
      choose(persona.threadReplies, defaults.threadReplies), true
    ),
    broadcastReplies: optionBool(
      choose(persona.broadcastReplies, defaults.broadcastReplies), false
    ),
  )

proc contains(values: openArray[string]; value: string): bool =
  for candidate in values:
    if candidate == value:
      return true

proc contains(values: openArray[int64]; value: int64): bool =
  for candidate in values:
    if candidate == value:
      return true

proc decideAgentTrigger*(
    event: AgentEventFacts; rules: openArray[AgentTriggerRule]
): AgentTriggerDecision =
  for index, rule in rules:
    if rule.name.len == 0 or (not rule.allChannels and rule.channels.len == 0):
      return AgentTriggerDecision(ruleIndex: -1, error: ateInvalidRule)
    if not rule.allChannels and not contains(rule.channels, event.channelId):
      continue
    if rule.kinds.len > 0 and not contains(rule.kinds, event.kind):
      continue
    if rule.requireMention and not event.mentioned:
      continue
    if rule.filter.len > 0:
      var values = initTable[string, WorkflowValue]()
      values["content"] = stringValue(event.content)
      values["author"] = stringValue(event.author)
      values["kind"] = intValue(event.kind)
      values["channel_id"] = stringValue(event.channelId)
      values["timestamp"] = intValue(event.timestamp)
      let filter = evaluateWorkflowCondition(rule.filter, values)
      if filter.error != wpeNone:
        return AgentTriggerDecision(ruleIndex: -1, error: ateInvalidFilter)
      if not filter.value:
        continue
    return AgentTriggerDecision(
      matched: true,
      ruleIndex: index,
      promptTag: if rule.promptTag.len > 0: rule.promptTag else: rule.name,
      error: ateNone,
    )
  AgentTriggerDecision(ruleIndex: -1, error: ateNone)

proc retryDelayMs(attempt: int64): int64 =
  var delay = AgentRetryInitialMs
  let doublingSteps = min(max(attempt, 1'i64) - 1, 6'i64)
  for _ in 0 ..< doublingSteps:
    if delay >= AgentRetryMaxMs div 2:
      return AgentRetryMaxMs
    delay *= 2
  min(delay, AgentRetryMaxMs)

proc rejectLifecycle(
    state: AgentLifecycleState; error: AgentLifecycleError
): AgentLifecycleDecision =
  AgentLifecycleDecision(
    allowed: false, error: error, action: alaReject, nextState: state
  )

proc acceptLifecycle(
    state: AgentLifecycleState; action: AgentLifecycleAction
): AgentLifecycleDecision =
  AgentLifecycleDecision(
    allowed: true, error: aleNone, action: action, nextState: state
  )

proc restartWait(
    state: AgentLifecycleState; nowMs: int64
): AgentLifecycleState =
  let delay = retryDelayMs(state.attempt)
  AgentLifecycleState(
    phase: apRestartWait,
    attempt: state.attempt,
    retryAtMs: if nowMs > high(int64) - delay: high(int64) else: nowMs + delay,
  )

proc matchingTurn(request: AgentLifecycleRequest): bool =
  request.commandTurnId.len > 0 and
    request.commandTurnId == request.state.turnId

proc decideAgentLifecycle*(
    request: AgentLifecycleRequest
): AgentLifecycleDecision =
  let state = request.state
  if request.nowMs < 0 or state.attempt < 0 or state.retryAtMs < 0:
    return rejectLifecycle(state, aleInvalidTransition)
  if state.phase == apStopped:
    if request.command == alcShutdown:
      return acceptLifecycle(state, alaNoop)
    return rejectLifecycle(state, aleInvalidTransition)

  case request.command
  of alcStart:
    if state.phase != apAbsent:
      return rejectLifecycle(state, aleInvalidTransition)
    if not request.pendingWork:
      return acceptLifecycle(state, alaNoop)
    acceptLifecycle(
      AgentLifecycleState(phase: apStarting, attempt: 1), alaSpawn
    )
  of alcStarted:
    if state.phase != apStarting:
      return rejectLifecycle(state, aleInvalidTransition)
    if request.commandAttempt != state.attempt:
      return rejectLifecycle(state, aleStaleAttempt)
    acceptLifecycle(AgentLifecycleState(phase: apReady), alaAcceptStart)
  of alcStartFailed:
    if state.phase != apStarting:
      return rejectLifecycle(state, aleInvalidTransition)
    if request.commandAttempt != state.attempt:
      return rejectLifecycle(state, aleStaleAttempt)
    acceptLifecycle(restartWait(state, request.nowMs), alaReapAndWait)
  of alcBeginTurn:
    if state.phase != apReady:
      return rejectLifecycle(state, aleInvalidTransition)
    if not request.pendingWork:
      return acceptLifecycle(state, alaNoop)
    if request.commandTurnId.len == 0:
      return rejectLifecycle(state, aleInvalidTurn)
    acceptLifecycle(
      AgentLifecycleState(
        phase: apRunning, attempt: state.attempt,
        turnId: request.commandTurnId,
      ),
      alaBeginTurn,
    )
  of alcCancel:
    if state.phase == apCancelling and matchingTurn(request):
      return acceptLifecycle(state, alaNoop)
    if state.phase != apRunning:
      return rejectLifecycle(state, aleInvalidTransition)
    if not matchingTurn(request):
      return rejectLifecycle(state, aleInvalidTurn)
    var next = state
    next.phase = apCancelling
    acceptLifecycle(next, alaSendCancel)
  of alcTurnFinished:
    if state.phase notin {apRunning, apCancelling}:
      return rejectLifecycle(state, aleInvalidTransition)
    if not matchingTurn(request):
      return rejectLifecycle(state, aleInvalidTurn)
    acceptLifecycle(AgentLifecycleState(phase: apReady), alaReturnReady)
  of alcCancelTimeout:
    if state.phase != apCancelling:
      return rejectLifecycle(state, aleInvalidTransition)
    if not matchingTurn(request):
      return rejectLifecycle(state, aleInvalidTurn)
    acceptLifecycle(restartWait(state, request.nowMs), alaReapAndWait)
  of alcProcessExited:
    if state.phase in {apAbsent, apRestartWait}:
      return acceptLifecycle(state, alaNoop)
    acceptLifecycle(restartWait(state, request.nowMs), alaReapAndWait)
  of alcRetry:
    if state.phase != apRestartWait:
      return rejectLifecycle(state, aleInvalidTransition)
    if not request.pendingWork or request.nowMs < state.retryAtMs:
      return acceptLifecycle(state, alaNoop)
    if state.attempt == high(int64):
      return rejectLifecycle(state, aleInvalidTransition)
    acceptLifecycle(
      AgentLifecycleState(
        phase: apStarting, attempt: state.attempt + 1
      ),
      alaSpawn,
    )
  of alcShutdown:
    acceptLifecycle(AgentLifecycleState(phase: apStopped), alaStop)
