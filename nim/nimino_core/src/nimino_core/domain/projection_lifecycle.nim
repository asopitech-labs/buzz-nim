## Resumable search, thread, and feed projection lifecycle owned by Nimino.

import std/[json, options, sets]

import nimino_core/domain/data_contract

const MaxProjectionBatch* = 1_000

type
  ProjectionBuildStatus* = enum
    pbsBuilding
    pbsReady
    pbsPublished
    pbsCancelled

  ProjectionEffect* = enum
    peReject
    peStart
    peStage
    peReady
    pePublish
    peCancel

  ProjectionLifecycleError* = enum
    pleNone
    pleInvalidState
    pleScopeMismatch
    pleOwnerMismatch
    pleEpochMismatch
    pleSourceChanged
    pleRevisionConflict
    pleCursorConflict
    pleBatchInvalid
    pleRecordInvalid
    pleCurrentRowInvalid
    pleStageFailure
    plePublishUnavailable
    plePublishFailure
    pleRevisionOverflow

  ProjectionBuildState* = object
    valid*: bool
    projection*: ProjectionKind
    communityId*: string
    sourceCheckpoint*: uint64
    sourceDigest*: string
    epoch*: string
    ownerNodeId*: string
    revision*: uint64
    cursor*: string
    status*: ProjectionBuildStatus

  ProjectionStartRequest* = object
    projection*: ProjectionKind
    communityId*: string
    sourceCheckpoint*: uint64
    sourceDigest*: string
    epoch*: string
    ownerNodeId*: string

  ProjectionStageRow* = object
    recordType*: string
    key*: string
    deleted*: bool
    value*: JsonNode

  ProjectionBatchRequest* = object
    communityId*: string
    epoch*: string
    ownerNodeId*: string
    expectedRevision*: uint64
    expectedCursor*: string
    sourceCheckpointMatches*: bool
    complete*: bool
    records*: seq[CanonicalRecord]
    currentRows*: seq[ProjectionRow]

  ProjectionBatchPlan* = object
    effect*: ProjectionEffect
    error*: ProjectionLifecycleError
    beforeState*: ProjectionBuildState
    nextState*: ProjectionBuildState
    rows*: seq[ProjectionStageRow]

  ProjectionDecision* = object
    effect*: ProjectionEffect
    error*: ProjectionLifecycleError
    state*: ProjectionBuildState

  ProjectionPublishPlan* = object
    effect*: ProjectionEffect
    error*: ProjectionLifecycleError
    beforeState*: ProjectionBuildState
    nextState*: ProjectionBuildState
    intentId*: string
    recordType*: string
    sourceCheckpoint*: uint64
    sourceDigest*: string

proc isSha256(value: string): bool =
  if value.len != 64:
    return false
  for character in value:
    if character notin {'0' .. '9', 'a' .. 'f'}:
      return false
  true

proc validState(state: ProjectionBuildState): bool =
  state.valid and state.communityId.len > 0 and state.epoch.len > 0 and
    state.ownerNodeId.len > 0 and isSha256(state.sourceDigest)

proc startProjection*(request: ProjectionStartRequest): ProjectionDecision =
  if request.communityId.len == 0 or request.epoch.len == 0 or
      request.ownerNodeId.len == 0 or not isSha256(request.sourceDigest):
    return ProjectionDecision(effect: peReject, error: pleInvalidState)
  ProjectionDecision(
    effect: peStart,
    error: pleNone,
    state: ProjectionBuildState(
      valid: true,
      projection: request.projection,
      communityId: request.communityId,
      sourceCheckpoint: request.sourceCheckpoint,
      sourceDigest: request.sourceDigest,
      epoch: request.epoch,
      ownerNodeId: request.ownerNodeId,
      status: pbsBuilding,
    ),
  )

proc rejectBatch(
    state: ProjectionBuildState; error: ProjectionLifecycleError
): ProjectionBatchPlan =
  ProjectionBatchPlan(
    effect: peReject,
    error: error,
    beforeState: state,
    nextState: state,
  )

proc validProjectionRow(row: ProjectionRow; expectedType: string): bool =
  row.recordType == expectedType and row.key.len > 0 and not row.value.isNil

proc stringField(value: JsonNode; name: string): string =
  if not value.isNil and value.kind == JObject and value.hasKey(name) and
      value[name].kind == JString:
    value[name].getStr()
  else:
    ""

proc intField(value: JsonNode; name: string): Option[int64] =
  if not value.isNil and value.kind == JObject and value.hasKey(name) and
      value[name].kind == JInt:
    some(value[name].getInt().int64)
  else:
    none(int64)

proc rowIndex(rows: seq[ProjectionStageRow]; key: string): int =
  for index, row in rows:
    if row.key == key:
      return index
  -1

proc currentRow(
    current: seq[ProjectionRow]; staged: seq[ProjectionStageRow];
    recordType, key: string
): JsonNode =
  let stagedIndex = rowIndex(staged, key)
  if stagedIndex >= 0:
    return if staged[stagedIndex].deleted:
        newJObject()
      else:
        staged[stagedIndex].value
  for row in current:
    if row.recordType == recordType and row.key == key:
      return row.value
  newJObject()

proc setRow(rows: var seq[ProjectionStageRow]; row: ProjectionStageRow) =
  let index = rowIndex(rows, row.key)
  if index >= 0:
    rows[index] = row
  else:
    rows.add(row)

proc countValue(value: JsonNode; name: string): int64 =
  intField(value, name).get(0)

proc threadRow(
    current: seq[ProjectionRow]; staged: seq[ProjectionStageRow]; key: string
): JsonNode =
  let value = currentRow(current, staged, "thread_index", key)
  %*{
    "replyCount": countValue(value, "replyCount"),
    "descendantCount": countValue(value, "descendantCount"),
  }

proc incrementThread(
    rows: var seq[ProjectionStageRow]; current: seq[ProjectionRow]; key,
    field: string
) =
  if key.len == 0:
    return
  var value = threadRow(current, rows, key)
  value[field] = %(value[field].getInt() + 1)
  rows.setRow(
    ProjectionStageRow(
      recordType: "thread_index", key: key, value: value
    )
  )

proc projectRecord(
    projection: ProjectionKind; record: CanonicalRecord;
    current: seq[ProjectionRow]; rows: var seq[ProjectionStageRow]
): ProjectionLifecycleError =
  let nullValue = record.value.isNil or record.value.kind == JNull
  if record.recordType != "event" or record.key.len == 0 or
      record.deleted != nullValue:
    return pleRecordInvalid
  let recordType = projectionRecordType(projection)
  if record.deleted:
    rows.setRow(
      ProjectionStageRow(
        recordType: recordType,
        key: record.key,
        deleted: true,
        value: newJNull(),
      )
    )
    return pleNone

  case projection
  of pkSearch:
    let content = stringField(record.value, "content")
    rows.setRow(
      ProjectionStageRow(
        recordType: recordType,
        key: record.key,
        value: %*{"sourceSequence": record.sequence, "content": content},
      )
    )
  of pkFeed:
    let createdAt = intField(record.value, "createdAt")
    if createdAt.isNone:
      return pleRecordInvalid
    rows.setRow(
      ProjectionStageRow(
        recordType: recordType,
        key: record.key,
        value: %*{
          "sourceSequence": record.sequence,
          "createdAt": createdAt.get(),
          "eventId": record.key,
        },
      )
    )
  of pkThread:
    rows.setRow(
      ProjectionStageRow(
        recordType: recordType,
        key: record.key,
        value: threadRow(current, rows, record.key),
      )
    )
    incrementThread(rows, current, stringField(record.value, "parentId"), "replyCount")
    incrementThread(
      rows, current, stringField(record.value, "rootId"), "descendantCount"
    )
  pleNone

proc planProjectionBatch*(
    state: ProjectionBuildState; request: ProjectionBatchRequest
): ProjectionBatchPlan =
  if not validState(state) or state.status != pbsBuilding:
    return rejectBatch(state, pleInvalidState)
  if request.communityId != state.communityId:
    return rejectBatch(state, pleScopeMismatch)
  if request.epoch != state.epoch:
    return rejectBatch(state, pleEpochMismatch)
  if request.ownerNodeId != state.ownerNodeId:
    return rejectBatch(state, pleOwnerMismatch)
  if not request.sourceCheckpointMatches:
    return rejectBatch(state, pleSourceChanged)
  if request.expectedRevision != state.revision:
    return rejectBatch(state, pleRevisionConflict)
  if request.expectedCursor != state.cursor:
    return rejectBatch(state, pleCursorConflict)
  if request.records.len > MaxProjectionBatch or
      (request.records.len == 0 and not request.complete):
    return rejectBatch(state, pleBatchInvalid)
  if state.revision == high(uint64):
    return rejectBatch(state, pleRevisionOverflow)

  let expectedType = projectionRecordType(state.projection)
  var currentKeys = initHashSet[string]()
  for row in request.currentRows:
    if not validProjectionRow(row, expectedType) or row.key in currentKeys:
      return rejectBatch(state, pleCurrentRowInvalid)
    currentKeys.incl(row.key)

  var previous = state.cursor
  var rows: seq[ProjectionStageRow]
  for record in request.records:
    if record.sequence > state.sourceCheckpoint or record.key <= previous:
      return rejectBatch(state, pleCursorConflict)
    previous = record.key
    let error = projectRecord(state.projection, record, request.currentRows, rows)
    if error != pleNone:
      return rejectBatch(state, error)

  var next = state
  next.revision += 1
  next.cursor = previous
  next.status = if request.complete: pbsReady else: pbsBuilding
  ProjectionBatchPlan(
    effect: (if request.complete: peReady else: peStage),
    error: pleNone,
    beforeState: state,
    nextState: next,
    rows: rows,
  )

proc settleProjectionBatch*(
    plan: ProjectionBatchPlan; stageSucceeded: bool
): ProjectionDecision =
  if plan.error != pleNone:
    return ProjectionDecision(
      effect: peReject, error: plan.error, state: plan.beforeState
    )
  if not stageSucceeded:
    return ProjectionDecision(
      effect: peReject, error: pleStageFailure, state: plan.beforeState
    )
  ProjectionDecision(effect: plan.effect, error: pleNone, state: plan.nextState)

proc planProjectionPublish*(
    state: ProjectionBuildState; ownerNodeId: string
): ProjectionPublishPlan =
  if not validState(state) or state.status != pbsReady:
    return ProjectionPublishPlan(
      effect: peReject, error: plePublishUnavailable, beforeState: state
    )
  if ownerNodeId != state.ownerNodeId:
    return ProjectionPublishPlan(
      effect: peReject, error: pleOwnerMismatch, beforeState: state
    )
  var next = state
  next.status = pbsPublished
  ProjectionPublishPlan(
    effect: pePublish,
    error: pleNone,
    beforeState: state,
    nextState: next,
    intentId: "projection-" & projectionRecordType(state.projection) & "-" & state.epoch,
    recordType: projectionRecordType(state.projection),
    sourceCheckpoint: state.sourceCheckpoint,
    sourceDigest: state.sourceDigest,
  )

proc settleProjectionPublish*(
    plan: ProjectionPublishPlan; publishSucceeded: bool
): ProjectionDecision =
  if plan.error != pleNone:
    return ProjectionDecision(
      effect: peReject, error: plan.error, state: plan.beforeState
    )
  if not publishSucceeded:
    return ProjectionDecision(
      effect: peReject, error: plePublishFailure, state: plan.beforeState
    )
  ProjectionDecision(effect: pePublish, error: pleNone, state: plan.nextState)

proc cancelProjection*(
    state: ProjectionBuildState; ownerNodeId: string
): ProjectionDecision =
  if not validState(state):
    return ProjectionDecision(effect: peReject, error: pleInvalidState, state: state)
  if ownerNodeId != state.ownerNodeId:
    return ProjectionDecision(effect: peReject, error: pleOwnerMismatch, state: state)
  var next = state
  next.status = pbsCancelled
  ProjectionDecision(effect: peCancel, error: pleNone, state: next)
