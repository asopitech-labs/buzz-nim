import std/[options, strutils, unittest]

import nimino_core/domain/ephemeral_policy

const
  Subject = "01".repeat(32)
  OriginA = "11".repeat(16)
  TransitionA = "aa".repeat(32)
  TransitionB = "bb".repeat(32)

proc command(
    observed = 1_000'u64; origin = OriginA; transition = TransitionA;
    scope = "community-a"; active = true
): EphemeralCommand =
  EphemeralCommand(
    scope: scope,
    kind: ekPresence,
    subject: Subject,
    value: if active: "online" else: "",
    active: active,
    observedAtMs: observed,
    ttlSecs: 180,
    originNodeId: origin,
    transitionId: transition,
  )

suite "Nimino ephemeral convergence policy":
  test "duplicate and reordered frames converge":
    let first = applyEphemeral(none(EphemeralState), command(), 1_000)
    check first.effect == eeApply
    check applyEphemeral(first.state, command(), 1_001).effect == eeReplay

    let newer = applyEphemeral(
      first.state,
      command(observed = 2_000, transition = TransitionB),
      2_000,
    )
    check newer.effect == eeApply
    let stale = applyEphemeral(newer.state, command(), 2_001)
    check stale.effect == eeStale
    check stale.state == newer.state

  test "tombstone prevents stale online resurrection":
    let online = applyEphemeral(none(EphemeralState), command(), 1_000)
    let offline = applyEphemeral(
      online.state,
      command(observed = 2_000, transition = TransitionB, active = false),
      2_000,
    )
    check offline.effect == eeApply
    check not offline.state.get().active
    check applyEphemeral(offline.state, command(), 2_001).effect == eeStale

  test "expiry and prune use inclusive cutoff":
    let live = applyEphemeral(none(EphemeralState), command(), 1_000).state.get()
    check pruneEphemeral(@[live], live.expiresAtMs).retained == @[live]
    check pruneEphemeral(@[live], live.expiresAtMs + 1).retained.len == 0
    check applyEphemeral(
      none(EphemeralState), command(), live.expiresAtMs + 1
    ).effect == eeExpired

  test "community and typing context are isolated":
    let first = applyEphemeral(none(EphemeralState), command(), 1_000)
    check applyEphemeral(
      first.state, command(scope = "community-b"), 1_000
    ).error == epeInvalidState

    var typing = command()
    typing.kind = ekTyping
    check applyEphemeral(none(EphemeralState), typing, 1_000).error == epeInvalidContext
    typing.context = "channel-a"
    check applyEphemeral(none(EphemeralState), typing, 1_000).effect == eeApply
