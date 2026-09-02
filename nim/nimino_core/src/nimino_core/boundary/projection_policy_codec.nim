## Strict JSON adapter for resumable projection lifecycle policy.

import std/json

import ../domain/[data_contract, projection_lifecycle]
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError("INVALID_REQUEST", message, requestId, "domain.projection.policy")

proc shape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "projection payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "projection payload is missing " & key)

proc text(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc flag(node: JsonNode; key, requestId: string): bool =
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean")
  node[key].getBool()

proc u64(node: JsonNode; key, requestId: string): uint64 =
  if node[key].kind != JInt or node[key].getBiggestInt() < 0:
    invalid(requestId, key & " must be a non-negative integer")
  uint64(node[key].getBiggestInt())

proc projection(value, requestId: string): ProjectionKind =
  case value
  of "search": pkSearch
  of "thread": pkThread
  of "feed": pkFeed
  else: invalid(requestId, "projection is invalid")

proc projectionName(value: ProjectionKind): string =
  case value
  of pkSearch: "search"
  of pkThread: "thread"
  of pkFeed: "feed"

proc status(value, requestId: string): ProjectionBuildStatus =
  case value
  of "building": pbsBuilding
  of "ready": pbsReady
  of "published": pbsPublished
  of "cancelled": pbsCancelled
  else: invalid(requestId, "projection status is invalid")

proc statusName(value: ProjectionBuildStatus): string =
  case value
  of pbsBuilding: "building"
  of pbsReady: "ready"
  of pbsPublished: "published"
  of pbsCancelled: "cancelled"

proc effectName(value: ProjectionEffect): string =
  case value
  of peReject: "reject"
  of peStart: "start"
  of peStage: "stage"
  of peReady: "ready"
  of pePublish: "publish"
  of peCancel: "cancel"

proc errorName(value: ProjectionLifecycleError): string =
  case value
  of pleNone: "none"
  of pleInvalidState: "invalid_state"
  of pleScopeMismatch: "scope_mismatch"
  of pleOwnerMismatch: "owner_mismatch"
  of pleEpochMismatch: "epoch_mismatch"
  of pleSourceChanged: "source_changed"
  of pleRevisionConflict: "revision_conflict"
  of pleCursorConflict: "cursor_conflict"
  of pleBatchInvalid: "batch_invalid"
  of pleRecordInvalid: "record_invalid"
  of pleCurrentRowInvalid: "current_row_invalid"
  of pleStageFailure: "stage_failure"
  of plePublishUnavailable: "publish_unavailable"
  of plePublishFailure: "publish_failure"
  of pleRevisionOverflow: "revision_overflow"

proc effect(value, requestId: string): ProjectionEffect =
  case value
  of "reject": peReject
  of "start": peStart
  of "stage": peStage
  of "ready": peReady
  of "publish": pePublish
  of "cancel": peCancel
  else: invalid(requestId, "projection effect is invalid")

proc policyError(value, requestId: string): ProjectionLifecycleError =
  case value
  of "none": pleNone
  of "invalid_state": pleInvalidState
  of "scope_mismatch": pleScopeMismatch
  of "owner_mismatch": pleOwnerMismatch
  of "epoch_mismatch": pleEpochMismatch
  of "source_changed": pleSourceChanged
  of "revision_conflict": pleRevisionConflict
  of "cursor_conflict": pleCursorConflict
  of "batch_invalid": pleBatchInvalid
  of "record_invalid": pleRecordInvalid
  of "current_row_invalid": pleCurrentRowInvalid
  of "stage_failure": pleStageFailure
  of "publish_unavailable": plePublishUnavailable
  of "publish_failure": plePublishFailure
  of "revision_overflow": pleRevisionOverflow
  else: invalid(requestId, "projection error is invalid")

proc state(node: JsonNode; requestId: string): ProjectionBuildState =
  shape(node, [
    "valid", "projection", "communityId", "sourceCheckpoint", "sourceDigest",
    "epoch", "ownerNodeId", "revision", "cursor", "status",
  ], requestId)
  ProjectionBuildState(
    valid: flag(node, "valid", requestId),
    projection: projection(text(node, "projection", requestId), requestId),
    communityId: text(node, "communityId", requestId),
    sourceCheckpoint: u64(node, "sourceCheckpoint", requestId),
    sourceDigest: text(node, "sourceDigest", requestId),
    epoch: text(node, "epoch", requestId),
    ownerNodeId: text(node, "ownerNodeId", requestId),
    revision: u64(node, "revision", requestId),
    cursor: text(node, "cursor", requestId),
    status: status(text(node, "status", requestId), requestId),
  )

proc stateNode(value: ProjectionBuildState): JsonNode =
  %*{
    "valid": value.valid,
    "projection": projectionName(value.projection),
    "communityId": value.communityId,
    "sourceCheckpoint": value.sourceCheckpoint,
    "sourceDigest": value.sourceDigest,
    "epoch": value.epoch,
    "ownerNodeId": value.ownerNodeId,
    "revision": value.revision,
    "cursor": value.cursor,
    "status": statusName(value.status),
  }

proc canonicalRecord(node: JsonNode; requestId: string): CanonicalRecord =
  shape(node, ["sequence", "recordType", "key", "deleted", "value"], requestId)
  CanonicalRecord(
    sequence: u64(node, "sequence", requestId),
    recordType: text(node, "recordType", requestId),
    key: text(node, "key", requestId),
    deleted: flag(node, "deleted", requestId),
    value: node["value"],
  )

proc projectionRow(node: JsonNode; requestId: string): ProjectionRow =
  shape(node, ["recordType", "key", "value"], requestId)
  ProjectionRow(
    recordType: text(node, "recordType", requestId),
    key: text(node, "key", requestId),
    value: node["value"],
  )

proc stageRow(node: JsonNode; requestId: string): ProjectionStageRow =
  shape(node, ["recordType", "key", "deleted", "value"], requestId)
  ProjectionStageRow(
    recordType: text(node, "recordType", requestId),
    key: text(node, "key", requestId),
    deleted: flag(node, "deleted", requestId),
    value: node["value"],
  )

proc stageRowsNode(values: seq[ProjectionStageRow]): JsonNode =
  result = newJArray()
  for value in values:
    result.add(%*{
      "recordType": value.recordType,
      "key": value.key,
      "deleted": value.deleted,
      "value": value.value,
    })

proc batchPlan(node: JsonNode; requestId: string): ProjectionBatchPlan =
  shape(node, ["effect", "error", "beforeState", "nextState", "rows"], requestId)
  result = ProjectionBatchPlan(
    effect: effect(text(node, "effect", requestId), requestId),
    error: policyError(text(node, "error", requestId), requestId),
    beforeState: state(node["beforeState"], requestId),
    nextState: state(node["nextState"], requestId),
  )
  if node["rows"].kind != JArray:
    invalid(requestId, "rows must be an array")
  for row in node["rows"].items:
    result.rows.add(stageRow(row, requestId))

proc batchPlanNode(value: ProjectionBatchPlan): JsonNode =
  %*{
    "effect": effectName(value.effect),
    "error": errorName(value.error),
    "beforeState": stateNode(value.beforeState),
    "nextState": stateNode(value.nextState),
    "rows": stageRowsNode(value.rows),
  }

proc publishPlan(node: JsonNode; requestId: string): ProjectionPublishPlan =
  shape(node, [
    "effect", "error", "beforeState", "nextState", "intentId", "recordType",
    "sourceCheckpoint", "sourceDigest",
  ], requestId)
  ProjectionPublishPlan(
    effect: effect(text(node, "effect", requestId), requestId),
    error: policyError(text(node, "error", requestId), requestId),
    beforeState: state(node["beforeState"], requestId),
    nextState: state(node["nextState"], requestId),
    intentId: text(node, "intentId", requestId),
    recordType: text(node, "recordType", requestId),
    sourceCheckpoint: u64(node, "sourceCheckpoint", requestId),
    sourceDigest: text(node, "sourceDigest", requestId),
  )

proc publishPlanNode(value: ProjectionPublishPlan): JsonNode =
  %*{
    "effect": effectName(value.effect),
    "error": errorName(value.error),
    "beforeState": stateNode(value.beforeState),
    "nextState": stateNode(value.nextState),
    "intentId": value.intentId,
    "recordType": value.recordType,
    "sourceCheckpoint": value.sourceCheckpoint,
    "sourceDigest": value.sourceDigest,
  }

proc decisionNode(value: ProjectionDecision): JsonNode =
  %*{
    "effect": effectName(value.effect),
    "error": errorName(value.error),
    "state": stateNode(value.state),
  }

proc executeProjectionPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "projection decision is required")
  let decision = payload["decision"].getStr()
  case decision
  of "start":
    shape(payload, ["decision", "request"], requestId)
    let request = payload["request"]
    shape(request, [
      "projection", "communityId", "sourceCheckpoint", "sourceDigest", "epoch",
      "ownerNodeId",
    ], requestId)
    result = %*{"decision": decision, "result": decisionNode(startProjection(
      ProjectionStartRequest(
        projection: projection(text(request, "projection", requestId), requestId),
        communityId: text(request, "communityId", requestId),
        sourceCheckpoint: u64(request, "sourceCheckpoint", requestId),
        sourceDigest: text(request, "sourceDigest", requestId),
        epoch: text(request, "epoch", requestId),
        ownerNodeId: text(request, "ownerNodeId", requestId),
      )
    ))}
  of "batch":
    shape(payload, ["decision", "state", "request"], requestId)
    let request = payload["request"]
    shape(request, [
      "communityId", "epoch", "ownerNodeId", "expectedRevision", "expectedCursor",
      "sourceCheckpointMatches", "complete", "records", "currentRows",
    ], requestId)
    var facts = ProjectionBatchRequest(
      communityId: text(request, "communityId", requestId),
      epoch: text(request, "epoch", requestId),
      ownerNodeId: text(request, "ownerNodeId", requestId),
      expectedRevision: u64(request, "expectedRevision", requestId),
      expectedCursor: text(request, "expectedCursor", requestId),
      sourceCheckpointMatches: flag(request, "sourceCheckpointMatches", requestId),
      complete: flag(request, "complete", requestId),
    )
    if request["records"].kind != JArray or request["currentRows"].kind != JArray:
      invalid(requestId, "records and currentRows must be arrays")
    for record in request["records"].items:
      facts.records.add(canonicalRecord(record, requestId))
    for row in request["currentRows"].items:
      facts.currentRows.add(projectionRow(row, requestId))
    result = %*{"decision": decision, "result": batchPlanNode(
      planProjectionBatch(state(payload["state"], requestId), facts)
    )}
  of "settle_batch":
    shape(payload, ["decision", "plan", "stageSucceeded"], requestId)
    result = %*{"decision": decision, "result": decisionNode(
      settleProjectionBatch(
        batchPlan(payload["plan"], requestId),
        flag(payload, "stageSucceeded", requestId),
      )
    )}
  of "publish":
    shape(payload, ["decision", "state", "ownerNodeId"], requestId)
    result = %*{"decision": decision, "result": publishPlanNode(
      planProjectionPublish(
        state(payload["state"], requestId), text(payload, "ownerNodeId", requestId)
      )
    )}
  of "settle_publish":
    shape(payload, ["decision", "plan", "publishSucceeded"], requestId)
    result = %*{"decision": decision, "result": decisionNode(
      settleProjectionPublish(
        publishPlan(payload["plan"], requestId),
        flag(payload, "publishSucceeded", requestId),
      )
    )}
  of "cancel":
    shape(payload, ["decision", "state", "ownerNodeId"], requestId)
    result = %*{"decision": decision, "result": decisionNode(
      cancelProjection(
        state(payload["state"], requestId), text(payload, "ownerNodeId", requestId)
      )
    )}
  else:
    invalid(requestId, "unknown projection policy decision")
