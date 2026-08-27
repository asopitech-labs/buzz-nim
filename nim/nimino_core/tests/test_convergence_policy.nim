import std/[options, unittest]

import nimino_core/domain/convergence_policy

const
  IdA = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  IdB = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  IdC = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
  DigestA = "1111111111111111111111111111111111111111111111111111111111111111"
  DigestB = "2222222222222222222222222222222222222222222222222222222222222222"
  DigestC = "3333333333333333333333333333333333333333333333333333333333333333"

proc record(
    id: string; logicalTime: int64; digest: string; kind = rmkLive;
    tombstone = tskVersioned; community = "community-a";
    key = "event:coordinate"
): ReplicaRecord =
  ReplicaRecord(
    communityId: community,
    logicalKey: key,
    recordId: id,
    logicalTime: logicalTime,
    contentDigest: digest,
    kind: kind,
    tombstoneKind: tombstone,
  )

proc restriction(
    revision: uint64; state: RestrictionState; transitionId, digest: string
): RestrictionVersion =
  RestrictionVersion(
    communityId: "community-a",
    subjectId: "member-a",
    revision: revision,
    state: state,
    transitionId: transitionId,
    contentDigest: digest,
  )

proc shuffled(values: seq[ReplicaRecord]; seed: var uint64): seq[ReplicaRecord] =
  result = values
  if result.len < 2:
    return
  for index in countdown(result.high, 1):
    seed = seed * 6364136223846793005'u64 + 1442695040888963407'u64
    let selected = int(seed mod uint64(index + 1))
    swap(result[index], result[selected])

proc reduce(values: seq[ReplicaRecord]): ReplicaRecord =
  var current = none(ReplicaRecord)
  for value in values:
    let merged = mergeReplicaRecord(current, value)
    doAssert merged.error == cpeNone
    current = merged.winner
  current.get()

suite "Nimino deterministic convergence policy":
  test "same id with different content is quarantined independent of arrival order":
    let first = observeIdentity(
      none(IdentityState),
      IdentityObservation(
        communityId: "community-a", recordId: IdA, contentDigest: DigestA
      ),
    )
    let collision = observeIdentity(
      some(first.state),
      IdentityObservation(
        communityId: "community-a", recordId: IdA, contentDigest: DigestB
      ),
    )
    check collision.error == cpeIdentityCollision
    check collision.effect == cmeQuarantine
    check collision.state.quarantined
    check collision.state.digestBounds == @[DigestA, DigestB]

    let reverse = observeIdentity(
      some(
        observeIdentity(
          none(IdentityState),
          IdentityObservation(
            communityId: "community-a", recordId: IdA, contentDigest: DigestB
          ),
        ).state
      ),
      IdentityObservation(
        communityId: "community-a", recordId: IdA, contentDigest: DigestA
      ),
    )
    check reverse.state == collision.state

  test "replaceable winner is invariant across randomized delivery orders":
    let values = @[
      record(IdA, 10, DigestA),
      record(IdC, 11, DigestC),
      record(IdB, 11, DigestB),
    ]
    let expected = record(IdB, 11, DigestB)
    var seed = 0x5eed'u64
    for _ in 0 ..< 512:
      check reduce(shuffled(values, seed)) == expected

  test "tombstones and bans cannot be resurrected by stale or concurrent state":
    let live = record(IdA, 10, DigestA)
    let deleted = record(IdB, 11, DigestB, rmkTombstone)
    check mergeReplicaRecord(some(deleted), live).winner.get() == deleted
    let newerLive = record(IdC, 12, DigestC)
    check mergeReplicaRecord(some(newerLive), deleted).winner.get() == newerLive

    let permanent = record(
      IdB, 11, DigestB, rmkTombstone, tskPermanent
    )
    let muchLaterLive = record(IdC, 100, DigestC)
    check mergeReplicaRecord(some(permanent), muchLaterLive).winner.get() == permanent

    let ban = restriction(7, rsBanned, IdB, DigestB)
    let staleRelease = restriction(6, rsReleased, IdA, DigestA)
    check mergeRestriction(some(ban), staleRelease).winner.get() == ban

    let concurrentRelease = restriction(7, rsReleased, IdA, DigestA)
    check mergeRestriction(some(concurrentRelease), ban).winner.get() == ban
    let newerRelease = restriction(8, rsReleased, IdC, DigestC)
    check mergeRestriction(some(newerRelease), ban).winner.get() == newerRelease

  test "partitioned replicas converge after rejoin":
    let v1 = record(IdA, 10, DigestA)
    let v2 = record(IdC, 11, DigestC)
    let deleted = record(IdB, 12, DigestB, rmkTombstone)
    var nodes = @[some(v1), some(v2), some(deleted)]
    for index in 0 ..< nodes.len:
      for candidate in [v1, v2, deleted]:
        nodes[index] = mergeReplicaRecord(nodes[index], candidate).winner
    check nodes[0] == nodes[1]
    check nodes[1] == nodes[2]
    check nodes[0].get() == deleted

  test "retention watermarks merge monotonically":
    let first = RetentionWatermark(
      communityId: "community-a",
      scopeId: "events",
      prunedThrough: 5,
      tombstoneProtectedThrough: 10,
    )
    let second = RetentionWatermark(
      communityId: "community-a",
      scopeId: "events",
      prunedThrough: 8,
      tombstoneProtectedThrough: 8,
    )
    let merged = mergeRetention(first, second)
    check merged.error == cpeNone
    check merged.watermark.prunedThrough == 8
    check merged.watermark.tombstoneProtectedThrough == 10
    check mergeRetention(second, first).watermark == merged.watermark

  test "malicious cross-community and malformed facts fail closed":
    let current = record(IdA, 10, DigestA)
    check mergeReplicaRecord(
      some(current), record(IdB, 11, DigestB, community = "community-b")
    ).error == cpeScopeMismatch
    check mergeReplicaRecord(
      some(current), record(IdB, 11, "not-a-digest")
    ).error == cpeDigestInvalid

    let sameIdCollision = mergeReplicaRecord(
      some(current), record(IdA, 10, DigestB)
    )
    check sameIdCollision.error == cpeIdentityCollision
    check sameIdCollision.effect == cmeQuarantine
    check sameIdCollision.winner.isNone

    let movedKeyCollision = mergeReplicaRecord(
      some(current), record(IdA, 10, DigestB, key = "event:other")
    )
    check movedKeyCollision.error == cpeIdentityCollision
    check movedKeyCollision.effect == cmeQuarantine
