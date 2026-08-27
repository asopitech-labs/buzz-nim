import std/[options, unittest]

import nimino_core/domain/object_sync_policy

const
  HashA = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  HashB = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
  HashC = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
  ManifestHash = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"

proc descriptor(digest: string; size: uint64; kind = okMedia): ObjectDescriptor =
  ObjectDescriptor(digest: digest, size: size, kind: kind)

proc manifest(): ObjectManifest =
  ObjectManifest(
    communityId: "community-a",
    manifestId: ManifestHash,
    generation: 7,
    objects: @[
      descriptor(HashA, 20),
      descriptor(HashB, 30, okGitPack),
      descriptor(HashC, 40, okGitManifest),
    ],
  )

proc origin(node: string; digests: seq[string]; available = true): ObjectOriginFact =
  ObjectOriginFact(nodeId: node, available: available, digests: digests)

proc request(
    mode: ObjectFetchMode; requested = ""; pins: seq[string] = @[];
    facts: seq[ObjectLocalFact] = @[]; origins: seq[ObjectOriginFact] = @[]
): ObjectSyncRequest =
  ObjectSyncRequest(
    communityId: "community-a",
    manifest: manifest(),
    manifestDigestVerified: true,
    lifecycleAllowsSync: true,
    mode: mode,
    requestedDigest: requested,
    pinnedDigests: pins,
    localFacts: facts,
    origins: origins,
    maxFetches: 4,
  )

suite "Nimino object sync, pin, and GC policy":
  test "eager fetch is bounded resumable and chooses a deterministic origin":
    let plan = planObjectSync(
      request(
        ofmEager,
        facts = @[
          ObjectLocalFact(
            digest: HashA, size: 20, present: true, verified: true
          ),
          ObjectLocalFact(
            digest: HashB,
            size: 30,
            partial: true,
            partialOffset: 12,
          ),
        ],
        origins = @[
          origin("node-c", @[HashB, HashC]),
          origin("node-b", @[HashB, HashC]),
        ],
      )
    )
    check plan.error == opeNone
    check plan.actions.len == 2
    check plan.actions[0].digest == HashB
    check plan.actions[0].sourceNodeId == "node-b"
    check plan.actions[0].resumeOffset == 12
    check plan.actions[1].digest == HashC

  test "lazy mode fetches only demand and pins survive rejoin":
    let lazy = planObjectSync(
      request(
        ofmLazy,
        requested = HashA,
        origins = @[origin("node-a", @[HashA, HashB])],
      )
    )
    check lazy.error == opeNone
    check lazy.actions.len == 1
    check lazy.actions[0].digest == HashA

    let rejoined = planObjectSync(
      request(
        ofmLazy,
        pins = @[HashB],
        origins = @[origin("node-a", @[HashB])],
      )
    )
    check rejoined.actions.len == 1
    check rejoined.actions[0].digest == HashB

  test "checksum and missing origin fail closed":
    var badManifest = request(ofmEager)
    badManifest.manifestDigestVerified = false
    check planObjectSync(badManifest).error == opeManifestDigestMismatch

    let corruptLocal = planObjectSync(
      request(
        ofmLazy,
        requested = HashA,
        facts = @[ObjectLocalFact(digest: HashA, size: 20, present: true)],
        origins = @[origin("node-a", @[HashA])],
      )
    )
    check corruptLocal.error == opeLocalChecksumMismatch

    check planObjectSync(
      request(ofmLazy, requested = HashC, origins = @[origin("node-a", @[HashA])])
    ).error == opeMissingOrigin

  test "pin transitions are revisioned and idempotent":
    let initial = initPinState("community-a")
    let pinned = decidePin(
      initial,
      PinRequest(
        communityId: "community-a",
        expectedRevision: 0,
        digest: HashA,
        pin: true,
      ),
    )
    check pinned.error == opeNone
    check pinned.state.revision == 1
    check pinned.state.digests == @[HashA]

    let duplicate = decidePin(
      pinned.state,
      PinRequest(
        communityId: "community-a",
        expectedRevision: 1,
        digest: HashA,
        pin: true,
      ),
    )
    check duplicate.effect == oaeNoop
    check duplicate.state == pinned.state

  test "GC keeps references pins partials and grace-period objects":
    let objects = @[
      ObjectLocalFact(
        digest: HashA,
        size: 20,
        present: true,
        verified: true,
        unreferencedSinceEpoch: some(1'u64),
      ),
      ObjectLocalFact(
        digest: HashB,
        size: 30,
        present: true,
        verified: true,
        unreferencedSinceEpoch: some(1'u64),
      ),
      ObjectLocalFact(
        digest: HashC,
        size: 40,
        present: true,
        verified: true,
        unreferencedSinceEpoch: some(9'u64),
      ),
      ObjectLocalFact(
        digest: ManifestHash,
        size: 50,
        present: true,
        verified: true,
        partial: true,
        unreferencedSinceEpoch: some(1'u64),
      ),
    ]
    let plan = planObjectGc(
      ObjectGcRequest(
        communityId: "community-a",
        currentEpoch: 10,
        graceEpochs: 5,
        referencedDigests: @[HashA],
        pinnedDigests: @[HashB],
        objects: objects,
        maxDeletes: 2,
      )
    )
    check plan.error == opeNone
    check plan.deleteDigests.len == 0

    var eligible = objects
    eligible[0].digest = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee"
    let deletePlan = planObjectGc(
      ObjectGcRequest(
        communityId: "community-a",
        currentEpoch: 10,
        graceEpochs: 5,
        pinnedDigests: @[HashB],
        objects: eligible,
        maxDeletes: 1,
      )
    )
    check deletePlan.deleteDigests == @[eligible[0].digest]
