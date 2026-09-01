import std/[json, unittest]

import nimino_core/domain/[data_contract, projection_lifecycle]

const Digest = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"

proc event(
    sequence: uint64; key: string; content: string; createdAt: int64;
    parent = ""; root = ""
): CanonicalRecord =
  CanonicalRecord(
    sequence: sequence,
    recordType: "event",
    key: key,
    value: %*{
      "event": {"content": content, "created_at": createdAt},
      "parentId": parent,
      "rootId": root,
    },
  )

proc start(kind: ProjectionKind): ProjectionBuildState =
  startProjection(
    ProjectionStartRequest(
      projection: kind,
      communityId: "community-a",
      sourceCheckpoint: 3,
      sourceDigest: Digest,
      epoch: "epoch-1",
      ownerNodeId: "node-a",
    )
  ).state

proc applyRows(current: var seq[ProjectionRow]; mutations: seq[ProjectionStageRow]) =
  for mutation in mutations:
    var found = -1
    for index, row in current:
      if row.key == mutation.key:
        found = index
        break
    if mutation.deleted:
      if found >= 0:
        current.delete(found)
    elif found >= 0:
      current[found] = ProjectionRow(
        recordType: mutation.recordType, key: mutation.key, value: mutation.value
      )
    else:
      current.add(
        ProjectionRow(
          recordType: mutation.recordType, key: mutation.key, value: mutation.value
        )
      )

proc run(
    kind: ProjectionKind; batches: seq[seq[CanonicalRecord]]
): tuple[state: ProjectionBuildState, rows: seq[ProjectionRow]] =
  result.state = start(kind)
  for index, records in batches:
    let plan = planProjectionBatch(
      result.state,
      ProjectionBatchRequest(
        communityId: "community-a",
        epoch: "epoch-1",
        ownerNodeId: "node-a",
        expectedRevision: result.state.revision,
        expectedCursor: result.state.cursor,
        sourceCheckpointMatches: true,
        complete: index == batches.high,
        records: records,
        currentRows: result.rows,
      ),
    )
    doAssert plan.error == pleNone
    applyRows(result.rows, plan.rows)
    result.state = settleProjectionBatch(plan, true).state

suite "Nimino projection rebuild lifecycle":
  let records = @[
    event(1, "a-root", "root", 10),
    event(2, "b-reply", "reply", 11, "a-root", "a-root"),
    event(3, "c-reply", "nested", 12, "b-reply", "a-root"),
  ]

  test "search thread and feed rebuild from zero state":
    let search = run(pkSearch, @[records])
    check search.state.status == pbsReady
    check search.rows.len == 3
    check search.rows[1].value["content"].getStr() == "reply"

    let feed = run(pkFeed, @[records])
    check feed.rows.len == 3
    check feed.rows[2].value["createdAt"].getInt() == 12

    let thread = run(pkThread, @[records])
    check thread.rows.len == 3
    check thread.rows[0].value["replyCount"].getInt() == 1
    check thread.rows[0].value["descendantCount"].getInt() == 2
    check thread.rows[1].value["replyCount"].getInt() == 1

  test "partial rebuild resumes and equals one-shot output":
    let oneShot = run(pkThread, @[records])
    let resumed = run(pkThread, @[records[0 .. 1], records[2 .. 2]])
    check resumed.state.status == pbsReady
    check resumed.state.revision == 2
    check resumed.state.cursor == "c-reply"
    check resumed.rows == oneShot.rows

  test "source epoch owner revision and cursor changes fail closed":
    let state = start(pkSearch)
    var request = ProjectionBatchRequest(
      communityId: "community-a",
      epoch: "epoch-1",
      ownerNodeId: "node-a",
      sourceCheckpointMatches: true,
      records: @[records[0]],
    )
    request.sourceCheckpointMatches = false
    check planProjectionBatch(state, request).error == pleSourceChanged
    request.sourceCheckpointMatches = true
    request.ownerNodeId = "node-b"
    check planProjectionBatch(state, request).error == pleOwnerMismatch
    request.ownerNodeId = "node-a"
    request.epoch = "epoch-2"
    check planProjectionBatch(state, request).error == pleEpochMismatch
    request.epoch = "epoch-1"
    request.expectedRevision = 1
    check planProjectionBatch(state, request).error == pleRevisionConflict
    request.expectedRevision = 0
    request.expectedCursor = "later"
    check planProjectionBatch(state, request).error == pleCursorConflict

  test "stage and publish failures retain the prior durable state":
    let state = start(pkFeed)
    let plan = planProjectionBatch(
      state,
      ProjectionBatchRequest(
        communityId: "community-a",
        epoch: "epoch-1",
        ownerNodeId: "node-a",
        sourceCheckpointMatches: true,
        complete: true,
        records: records,
      ),
    )
    check settleProjectionBatch(plan, false).state == state
    let ready = settleProjectionBatch(plan, true).state
    let publish = planProjectionPublish(ready, "node-a")
    check publish.recordType == "feed_index"
    check publish.sourceCheckpoint == 3
    check settleProjectionPublish(publish, false).state == ready
    check settleProjectionPublish(publish, true).state.status == pbsPublished
