import std/[json, options, strutils, unittest]

import nimino_core/domain/data_contract

suite "Nimino data contract v1":
  test "classifies durable truth, rebuildable cache, and append-only logs":
    check DataProtocolName == "nimino.data"
    check DataProtocolVersion == 1
    check classifyRecord("event") == some(dcCanonical)
    check classifyRecord("thread_index") == some(dcCache)
    check classifyRecord("audit_entry") == some(dcLog)
    check classifyRecord("unknown") == none(DataClass)

  test "canonical commits are scoped, optimistic, and class-pure":
    let valid = WriteIntent(
      kind: wiCanonicalCommit,
      intentId: "commit-42",
      communityId: "community-a",
      checkpoint: some(41'u64),
      writes: @[
        RecordWrite(recordType: "event", key: "event-42", value: %*{"kind": 9})
      ],
    )
    check validate(valid) == dceNone

    var mixed = valid
    mixed.writes.add RecordWrite(
      recordType: "thread_index",
      key: "event-42",
      value: %*{},
    )
    check validate(mixed) == dceClassMismatch

    var unguarded = valid
    unguarded.checkpoint = none(uint64)
    check validate(unguarded) == dceCheckpointRequired

    var missingValue = valid
    missingValue.writes[0].value = nil
    check validate(missingValue) == dceValueRequired

    var duplicate = valid
    duplicate.writes.add valid.writes[0]
    check validate(duplicate) == dceDuplicateKey

    var tombstone = valid
    tombstone.writes[0].deleted = true
    tombstone.writes[0].value = newJNull()
    check validate(tombstone) == dceNone

  test "cache replacement and log append keep their lifecycle semantics":
    let clearCache = WriteIntent(
      kind: wiCacheReplace,
      intentId: "rebuild-thread",
      communityId: "community-a",
      checkpoint: some(42'u64),
    )
    check validate(clearCache) == dceNone

    let appendLog = WriteIntent(
      kind: wiLogAppend,
      intentId: "audit-42",
      communityId: "community-a",
      writes: @[
        RecordWrite(
          recordType: "audit_entry", key: "audit-42", value: %*{"action": "write"}
        )
      ],
    )
    check validate(appendLog) == dceNone

    var invalidDelete = appendLog
    invalidDelete.writes[0].deleted = true
    check validate(invalidDelete) == dceDeleteForbidden

  test "query intent cannot escape its record class or community":
    let byKey = QueryIntent(
      kind: qiByKey,
      communityId: "community-a",
      recordType: "channel",
      key: "channel-1",
      limit: 1,
    )
    check validate(byKey) == dceNone

    var unscoped = byKey
    unscoped.communityId = ""
    check validate(unscoped) == dceCommunityRequired

    var cursorByKey = byKey
    cursorByKey.after = some(1'u64)
    check validate(cursorByKey) == dceQueryShape

    let invalidChanges = QueryIntent(
      kind: qiChanges,
      communityId: "community-a",
      recordType: "thread_index",
      limit: 100,
    )
    check validate(invalidChanges) == dceQueryShape

    var changes = invalidChanges
    changes.recordType = ""
    check validate(changes) == dceNone

  test "projection rebuild output is bound to exact source input":
    let input = ProjectionRebuildInput(
      projection: pkThread,
      communityId: "community-a",
      throughSequence: 42,
      sourceDigest: repeat('a', 64),
      records: @[
        CanonicalRecord(
          sequence: 42, recordType: "event", key: "event-42", value: %*{"kind": 9}
        )
      ],
    )
    let output = ProjectionRebuildOutput(
      projection: pkThread,
      communityId: "community-a",
      throughSequence: 42,
      sourceDigest: repeat('a', 64),
      rows: @[
        ProjectionRow(
          recordType: "thread_index", key: "event-42", value: %*{"replyCount": 0}
        )
      ],
    )
    check validate(input) == dceNone
    check validate(input, output) == dceNone

    var stale = output
    stale.throughSequence = 41
    check validate(input, stale) == dceProjectionMismatch
