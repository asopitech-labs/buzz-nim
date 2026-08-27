import std/[options, strutils, unittest]

import nimino_core/domain/event_policy

const
  EventA = repeat('a', 64)
  EventB = repeat('b', 64)
  EventC = repeat('c', 64)
  AuthorA = repeat('1', 64)
  AuthorB = repeat('2', 64)

suite "Nimino event and message policy":
  test "classifies only supported storable event shapes":
    check classifyEvent(9, 0, 0) ==
      EventClassification(disposition: edStored, error: epeNone)
    check classifyEvent(20001, 0, 0).disposition == edEphemeral
    check classifyEvent(10001, 0, 0).disposition == edReplaceable
    check classifyEvent(30023, 1, 4).disposition == edParameterized
    check classifyEvent(30023, 0, 0).error == epeDTagRequired
    check classifyEvent(30023, 2, 4).error == epeDTagCardinality
    check classifyEvent(22242, 0, 0).error == epeAuthNotStorable
    check classifyEvent(65535, 0, 0).error == epeUnsupportedKind

  test "replacement ordering is deterministic":
    let current = EventVersion(createdAt: 10, eventId: EventB)
    check decideReplacement(EventVersion(createdAt: 11, eventId: EventC), some(current)) ==
      rdReplace
    check decideReplacement(EventVersion(createdAt: 10, eventId: EventA), some(current)) ==
      rdReplace
    check decideReplacement(EventVersion(createdAt: 10, eventId: EventB), some(current)) ==
      rdDuplicate
    check decideReplacement(
      EventVersion(createdAt: 10, eventId: EventB.toUpperAscii()), some(current)
    ) == rdDuplicate
    check decideReplacement(EventVersion(createdAt: 9, eventId: EventA), some(current)) ==
      rdStale

  test "thread plan derives ancestry and counter mutations":
    let parent = ThreadParentFacts(
      eventId: EventB,
      createdAt: 9,
      channelId: "channel-a",
      metadata: some(ThreadMetadataFacts(rootId: EventA, depth: 1)),
    )
    let decision = decideThread(ThreadRequest(
      eventId: EventC,
      createdAt: 10,
      channelId: "channel-a",
      tags: @[
        @["e", EventA.toUpperAscii(), "", "root"],
        @["e", EventB.toUpperAscii(), "", "reply"],
        @["broadcast", "1"],
      ],
      parent: some(parent),
      rootCreatedAt: some(8'i64),
    ))
    check decision.error == epeNone
    check decision.plan.depth == 2
    check decision.plan.rootId == EventA
    check decision.plan.parentId == EventB
    check decision.plan.parentReplyDelta == 1
    check decision.plan.rootDescendantDelta == 1
    check decision.plan.broadcast

    let rejected = decideThread(ThreadRequest(
      eventId: EventC,
      createdAt: 10,
      channelId: "channel-a",
      tags: @[
        @["e", EventC, "", "root"],
        @["e", EventB, "", "reply"],
      ],
      parent: some(parent),
    ))
    check rejected.error == epeThreadRootMismatch

  test "deletion is author-owned, monotonic, and counter-safe":
    let target = DeletionTargetFacts(
      eventId: EventB,
      author: AuthorA,
      createdAt: 9,
      active: true,
      parentId: some(EventA),
      rootId: some(EventA),
    )
    let accepted = decideDeletion(DeletionRequest(
      actor: AuthorA,
      createdAt: 10,
      eTargets: @[EventB.toUpperAscii()],
      target: some(target),
    ))
    check accepted.action == daDeleteEvent
    check accepted.parentReplyDelta == -1
    check accepted.rootDescendantDelta == -1

    var inactive = target
    inactive.active = false
    let replayed = decideDeletion(DeletionRequest(
      actor: AuthorA,
      createdAt: 10,
      eTargets: @[EventB],
      target: some(inactive),
    ))
    check replayed.action == daNoop
    check replayed.parentReplyDelta == 0
    check replayed.rootDescendantDelta == 0

    var foreign = target
    foreign.author = AuthorB
    check decideDeletion(DeletionRequest(
      actor: AuthorA,
      createdAt: 10,
      eTargets: @[EventB],
      target: some(foreign),
    )).error == epeDeleteAuthorMismatch

    var newer = target
    newer.createdAt = 11
    check decideDeletion(DeletionRequest(
      actor: AuthorA,
      createdAt: 10,
      eTargets: @[EventB],
      target: some(newer),
    )).action == daKeepNewer

  test "reaction policy rejects missing targets and active duplicates":
    check decideReaction(ReactionRequest(targetExists: false)).error ==
      epeReactionTargetMissing
    check decideReaction(ReactionRequest(targetExists: true, activeDuplicate: true)).action ==
      raDuplicate

    let custom = decideReaction(ReactionRequest(
      targetExists: true,
      content: ":party_parrot:",
      tags: @[@["emoji", "party_parrot", "https://example.test/parrot.png"]],
    ))
    check custom.error == epeNone
    check custom.action == raInsert
    check custom.emoji == ":party_parrot:"
