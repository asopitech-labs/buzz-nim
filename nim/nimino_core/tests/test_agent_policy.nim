import std/[options, unittest]

import nimino_core/domain/agent_policy

suite "Nimino agent and persona policy":
  test "persona values override defaults and triggers replace shallowly":
    let resolved = resolvePersonaBehavior(
      PersonaBehavior(
        model: some("gpt-5"),
        subscribe: some(newSeq[string]()),
        triggers: some(PersonaTriggers(mentions: some(false))),
      ),
      PersonaBehavior(
        model: some("default"),
        subscribe: some(@["general"]),
        triggers: some(PersonaTriggers(
          mentions: some(true), keywords: some(@["incident"])
        )),
        threadReplies: some(false),
      ),
    )
    check resolved.model == some("gpt-5")
    check resolved.subscribe == some(newSeq[string]())
    check resolved.triggers.get().mentions == false
    check resolved.triggers.get().keywords.len == 0
    check resolved.threadReplies == false
    check resolved.broadcastReplies == false

  test "ordered trigger rules match once and filter errors fail closed":
    let event = AgentEventFacts(
      content: "P1 incident", author: "alice", kind: 9,
      channelId: "general", timestamp: 10, mentioned: true,
    )
    let rules = @[
      AgentTriggerRule(
        name: "wrong-kind", allChannels: true, kinds: @[1'i64]
      ),
      AgentTriggerRule(
        name: "incident", allChannels: true, kinds: @[9'i64],
        requireMention: true,
        filter: "str_contains(content, \"P1\") && kind == 9",
        promptTag: "urgent",
      ),
    ]
    let matched = decideAgentTrigger(event, rules)
    check matched.matched
    check matched.ruleIndex == 1
    check matched.promptTag == "urgent"

    let broken = decideAgentTrigger(event, @[
      AgentTriggerRule(name: "broken", allChannels: true, filter: "missing")
    ])
    check not broken.matched
    check broken.error == ateInvalidFilter

  test "cancel timeout reaps before a bounded restart":
    let running = AgentLifecycleState(
      phase: apRunning, turnId: "turn-1"
    )
    let cancel = decideAgentLifecycle(AgentLifecycleRequest(
      state: running, command: alcCancel, commandTurnId: "turn-1",
      nowMs: 100,
    ))
    check cancel.allowed
    check cancel.action == alaSendCancel
    check cancel.nextState.phase == apCancelling

    let timedOut = decideAgentLifecycle(AgentLifecycleRequest(
      state: cancel.nextState, command: alcCancelTimeout,
      commandTurnId: "turn-1", pendingWork: true, nowMs: 200,
    ))
    check timedOut.action == alaReapAndWait
    check timedOut.nextState.phase == apRestartWait
    check timedOut.nextState.retryAtMs == 5_200

    let early = decideAgentLifecycle(AgentLifecycleRequest(
      state: timedOut.nextState, command: alcRetry,
      pendingWork: true, nowMs: 5_199,
    ))
    check early.action == alaNoop
    let retry = decideAgentLifecycle(AgentLifecycleRequest(
      state: timedOut.nextState, command: alcRetry,
      pendingWork: true, nowMs: 5_200,
    ))
    check retry.action == alaSpawn
    check retry.nextState.phase == apStarting
    check retry.nextState.attempt == 1

  test "stale starts and turns cannot replace current state":
    let starting = AgentLifecycleState(phase: apStarting, attempt: 2)
    let stale = decideAgentLifecycle(AgentLifecycleRequest(
      state: starting, command: alcStarted, commandAttempt: 1,
    ))
    check not stale.allowed
    check stale.error == aleStaleAttempt
    check stale.nextState == starting

    let running = AgentLifecycleState(phase: apRunning, turnId: "current")
    let wrongTurn = decideAgentLifecycle(AgentLifecycleRequest(
      state: running, command: alcTurnFinished, commandTurnId: "old",
    ))
    check not wrongTurn.allowed
    check wrongTurn.error == aleInvalidTurn

    let saturated = decideAgentLifecycle(AgentLifecycleRequest(
      state: AgentLifecycleState(phase: apStarting, attempt: high(int64)),
      command: alcStartFailed, commandAttempt: high(int64),
      nowMs: high(int64),
    ))
    check saturated.nextState.retryAtMs == high(int64)

    let exhausted = decideAgentLifecycle(AgentLifecycleRequest(
      state: saturated.nextState, command: alcRetry,
      pendingWork: true, nowMs: high(int64),
    ))
    check not exhausted.allowed
    check exhausted.error == aleInvalidTransition
    check exhausted.nextState == saturated.nextState

  test "shutdown is terminal and idempotent":
    let stopped = decideAgentLifecycle(AgentLifecycleRequest(
      state: AgentLifecycleState(phase: apReady), command: alcShutdown,
    ))
    check stopped.action == alaStop
    check stopped.nextState.phase == apStopped
    check decideAgentLifecycle(AgentLifecycleRequest(
      state: stopped.nextState, command: alcShutdown
    )).action == alaNoop
