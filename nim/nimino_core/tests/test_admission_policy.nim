import std/[options, unittest]

import nimino_core/domain/admission_policy

const EventId = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

proc command(scope = "community-a"; observed = 1_000'u64): ReplayClaimCommand =
  ReplayClaimCommand(
    scope: scope,
    eventId: EventId,
    observedAtMs: observed,
    ttlSecs: MinReplayTtlSecs,
  )

suite "Nimino cluster admission policy":
  test "exactly one live replay claim is admitted":
    let first = applyReplayClaim(none(ReplayClaimState), command(), 1)
    check first.allowed
    check first.error == apeNone

    let duplicate = applyReplayClaim(first.state, command(observed = 2_000), 2)
    check not duplicate.allowed
    check duplicate.error == apeNone
    check duplicate.state == first.state

  test "expiry and community scope are deterministic":
    let first = applyReplayClaim(none(ReplayClaimState), command(), 1)
    let atExpiry = applyReplayClaim(first.state, command(observed = 121_000), 2)
    check not atExpiry.allowed
    let afterExpiry = applyReplayClaim(first.state, command(observed = 121_001), 3)
    check afterExpiry.allowed
    check afterExpiry.state.get().lastControlIndex == 3

    let other = applyReplayClaim(
      none(ReplayClaimState), command(scope = "community-b"), 4
    )
    check other.allowed

  test "prune retains inclusive expiry and rejects corrupt state":
    let first = applyReplayClaim(none(ReplayClaimState), command(), 1).state.get()
    check pruneReplayClaims(@[first], first.expiresAtMs).retained == @[first]
    check pruneReplayClaims(@[first], first.expiresAtMs + 1).retained.len == 0

    var corrupt = first
    corrupt.eventId = "not-an-event-id"
    check pruneReplayClaims(@[corrupt], 1).error == apeInvalidState

  test "invalid and stale facts fail closed":
    check applyReplayClaim(
      none(ReplayClaimState), command(scope = ""), 1
    ).error == apeInvalidScope
    var shortTtl = command()
    shortTtl.ttlSecs = MinReplayTtlSecs - 1
    check applyReplayClaim(
      none(ReplayClaimState), shortTtl, 1
    ).error == apeTtlOutOfRange
    let first = applyReplayClaim(none(ReplayClaimState), command(), 2)
    check applyReplayClaim(first.state, command(), 2).error == apeControlReplay

  test "fixed-window totals are cluster-wide and namespace isolated":
    let first = applyRateLimit(
      none(RateLimitState),
      RateLimitCommand(
        namespace: "principal", key: "community-a:alice:api",
        observedAtMs: 60_001, windowSecs: 60, limit: 2,
      ),
      1,
    )
    check first.allowed
    check first.current == 1
    let second = applyRateLimit(
      first.state,
      RateLimitCommand(
        namespace: "principal", key: "community-a:alice:api",
        observedAtMs: 60_999, windowSecs: 60, limit: 2,
      ),
      2,
    )
    check second.allowed
    let denied = applyRateLimit(
      second.state,
      RateLimitCommand(
        namespace: "principal", key: "community-a:alice:api",
        observedAtMs: 61_000, windowSecs: 60, limit: 2,
      ),
      3,
    )
    check not denied.allowed
    check denied.current == 3

    let isolated = applyRateLimit(
      none(RateLimitState),
      RateLimitCommand(
        namespace: "ip", key: "community-a:alice:api",
        observedAtMs: 61_000, windowSecs: 60, limit: 2,
      ),
      4,
    )
    check isolated.allowed

  test "rate windows roll deterministically and reject policy drift":
    let first = applyRateLimit(
      none(RateLimitState),
      RateLimitCommand(
        namespace: "principal", key: "k", observedAtMs: 60_001,
        windowSecs: 60, limit: 1,
      ),
      1,
    )
    let rolled = applyRateLimit(
      first.state,
      RateLimitCommand(
        namespace: "principal", key: "k", observedAtMs: 120_000,
        windowSecs: 60, limit: 1,
      ),
      2,
    )
    check rolled.allowed
    check rolled.current == 1

    var drift = RateLimitCommand(
      namespace: "principal", key: "k", observedAtMs: 120_001,
      windowSecs: 60, limit: 2,
    )
    check applyRateLimit(rolled.state, drift, 3).error == apePolicyConflict
    drift.limit = 1
    drift.observedAtMs = 60_001
    check applyRateLimit(rolled.state, drift, 3).error == apeClockRegression

  test "one control entry atomically consumes a same-key rate batch":
    let command = RateLimitCommand(
      namespace: "principal", key: "community-a:alice:api",
      observedAtMs: 60_001, windowSecs: 60, limit: 2,
    )
    let batch = applyRateLimitBatch(
      @[], @[command, command, command], 7
    )
    check batch.error == apeNone
    check batch.results.len == 3
    check batch.results[0].allowed
    check batch.results[1].allowed
    check not batch.results[2].allowed
    check batch.results[2].current == 3
    check batch.results[2].state.get().lastControlIndex == 7
    check applyRateLimit(
      batch.results[2].state, command, 7
    ).error == apeControlReplay

  test "authorization invalidation revisions ignore replay and reorder":
    let command = AuthorizationInvalidationCommand(
      scope: "community-a",
      kind: aikMembership,
      subject: EventId,
      channelId: "channel-a",
      factId: "fact-a",
    )
    let first = applyAuthorizationInvalidation(
      none(AuthorizationInvalidationState), command, 10
    )
    check first.effect == aieApply
    check first.state.get().revision == 10
    check applyAuthorizationInvalidation(first.state, command, 10).effect == aieReplay

    var newer = command
    newer.factId = "fact-b"
    let advanced = applyAuthorizationInvalidation(first.state, newer, 11)
    check advanced.effect == aieApply
    check applyAuthorizationInvalidation(advanced.state, command, 10).effect == aieStale

  test "authorization invalidation keys preserve community and surface shape":
    let ban = AuthorizationInvalidationCommand(
      scope: "community-a", kind: aikBan, subject: EventId, factId: "ban-a"
    )
    check applyAuthorizationInvalidation(
      none(AuthorizationInvalidationState), ban, 1
    ).effect == aieApply

    var invalid = ban
    invalid.scope = ""
    check applyAuthorizationInvalidation(
      none(AuthorizationInvalidationState), invalid, 1
    ).error == aieInvalidScope
    invalid = ban
    invalid.channelId = "foreign-channel"
    check applyAuthorizationInvalidation(
      none(AuthorizationInvalidationState), invalid, 1
    ).error == aieInvalidChannel
