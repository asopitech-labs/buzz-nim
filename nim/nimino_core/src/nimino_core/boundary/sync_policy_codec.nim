## Strict JSON adapter for bounded anti-entropy policy.

import std/[json, options]

import ../domain/[convergence_policy, sync_protocol]
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError("INVALID_REQUEST", message, requestId, "domain.sync.policy")

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "sync policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "sync policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc boolAt(node: JsonNode; key, requestId: string): bool =
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean")
  node[key].getBool()

proc inventoryFactAt(node: JsonNode; requestId: string): tuple[
  recordType, key, identity, contentDigest: string; deleted: bool
] =
  requireShape(
    node,
    ["recordType", "key", "deleted", "identity", "contentDigest"],
    requestId,
  )
  (
    stringAt(node, "recordType", requestId),
    stringAt(node, "key", requestId),
    stringAt(node, "identity", requestId),
    stringAt(node, "contentDigest", requestId),
    boolAt(node, "deleted", requestId),
  )

proc replicaRecord(
    communityId: string;
    fact: tuple[recordType, key, identity, contentDigest: string; deleted: bool]
): ReplicaRecord =
  ReplicaRecord(
    communityId: communityId,
    logicalKey: fact.recordType & "\x1f" & fact.key,
    recordId: fact.identity,
    logicalTime: 0,
    contentDigest: fact.contentDigest,
    kind: (if fact.deleted: rmkTombstone else: rmkLive),
    tombstoneKind: (if fact.deleted: tskPermanent else: tskVersioned),
  )

proc convergenceEffectName(value: ConvergenceEffect): string =
  case value
  of cmeReject: "reject"
  of cmeInsert: "insert"
  of cmeKeep: "keep"
  of cmeReplace: "replace"
  of cmeDuplicate: "duplicate"
  of cmeQuarantine: "quarantine"
  of cmeMerge: "replace"

proc convergenceErrorName(value: ConvergencePolicyError): string =
  case value
  of cpeNone: "none"
  of cpeIdentityInvalid: "identity_invalid"
  of cpeDigestInvalid: "digest_invalid"
  of cpeVersionInvalid: "version_invalid"
  of cpeScopeMismatch: "scope_mismatch"
  of cpeIdentityCollision: "identity_collision"
  of cpeFactConflict: "fact_conflict"
  of cpeRetentionInvalid: "retention_invalid"

proc mergeInventoryNode(payload: JsonNode; requestId: string): JsonNode =
  let communityId = stringAt(payload, "communityId", requestId)
  if communityId.len == 0 or payload["records"].kind != JArray or
      payload["records"].len > int(MaxSyncRecords):
    invalid(requestId, "inventory scope or record bound is invalid")
  result = newJArray()
  for pairNode in payload["records"].items:
    requireShape(pairNode, ["current", "incoming"], requestId)
    let incomingFact = inventoryFactAt(pairNode["incoming"], requestId)
    let current = if pairNode["current"].kind == JNull:
        none(ReplicaRecord)
      else:
        some(replicaRecord(communityId, inventoryFactAt(pairNode["current"], requestId)))
    let decision = mergeReplicaRecord(
      current, replicaRecord(communityId, incomingFact)
    )
    result.add(
      %*{
        "effect": convergenceEffectName(decision.effect),
        "error": convergenceErrorName(decision.error),
      }
    )

proc uint64At(node: JsonNode; key, requestId: string): uint64 =
  if node[key].kind != JInt:
    invalid(requestId, key & " must be a non-negative integer")
  let value = node[key].getBiggestInt()
  if value < 0:
    invalid(requestId, key & " must be a non-negative integer")
  uint64(value)

proc uint16At(node: JsonNode; key, requestId: string): uint16 =
  let value = uint64At(node, key, requestId)
  if value > uint64(uint16.high):
    invalid(requestId, key & " exceeds uint16")
  uint16(value)

proc uint32At(node: JsonNode; key, requestId: string): uint32 =
  let value = uint64At(node, key, requestId)
  if value > uint64(uint32.high):
    invalid(requestId, key & " exceeds uint32")
  uint32(value)

proc phaseAt(node: JsonNode; key, requestId: string): SyncPhase =
  case stringAt(node, key, requestId)
  of "idle": spIdle
  of "waiting_batch": spWaitingBatch
  of "applying": spApplying
  of "complete": spComplete
  of "cancelled": spCancelled
  else: invalid(requestId, key & " is not a sync phase")

proc envelopeAt(node: JsonNode; requestId: string): SyncEnvelope =
  requireShape(
    node,
    [
      "protocol",
      "version",
      "sessionId",
      "communityId",
      "senderNodeId",
      "receiverNodeId",
    ],
    requestId,
  )
  SyncEnvelope(
    protocol: stringAt(node, "protocol", requestId),
    version: uint16At(node, "version", requestId),
    sessionId: stringAt(node, "sessionId", requestId),
    communityId: stringAt(node, "communityId", requestId),
    senderNodeId: stringAt(node, "senderNodeId", requestId),
    receiverNodeId: stringAt(node, "receiverNodeId", requestId),
  )

proc digestAt(node: JsonNode; requestId: string): DigestFrame =
  requireShape(node, ["envelope", "checkpoint", "prefixDigest"], requestId)
  DigestFrame(
    envelope: envelopeAt(node["envelope"], requestId),
    checkpoint: uint64At(node, "checkpoint", requestId),
    prefixDigest: stringAt(node, "prefixDigest", requestId),
  )

proc rangeRequestAt(node: JsonNode; requestId: string): RangeRequestFrame =
  requireShape(
    node,
    ["envelope", "afterCheckpoint", "limitRecords", "maxEncodedBytes"],
    requestId,
  )
  RangeRequestFrame(
    envelope: envelopeAt(node["envelope"], requestId),
    afterCheckpoint: uint64At(node, "afterCheckpoint", requestId),
    limitRecords: uint16At(node, "limitRecords", requestId),
    maxEncodedBytes: uint32At(node, "maxEncodedBytes", requestId),
  )

proc recordAt(node: JsonNode; requestId: string): SyncRecord =
  requireShape(
    node,
    ["sequence", "recordType", "key", "deleted", "payload", "contentDigest"],
    requestId,
  )
  SyncRecord(
    sequence: uint64At(node, "sequence", requestId),
    recordType: stringAt(node, "recordType", requestId),
    key: stringAt(node, "key", requestId),
    deleted: boolAt(node, "deleted", requestId),
    payload: stringAt(node, "payload", requestId),
    contentDigest: stringAt(node, "contentDigest", requestId),
  )

proc recordsAt(node: JsonNode; requestId: string): seq[SyncRecord] =
  if node.kind != JArray:
    invalid(requestId, "records must be an array")
  for record in node.items:
    result.add(recordAt(record, requestId))

proc batchAt(node: JsonNode; requestId: string): RangeBatchFrame =
  requireShape(
    node,
    [
      "envelope",
      "batchId",
      "baseCheckpoint",
      "baseDigest",
      "throughCheckpoint",
      "resultDigest",
      "encodedBytes",
      "digestVerified",
      "records",
    ],
    requestId,
  )
  RangeBatchFrame(
    envelope: envelopeAt(node["envelope"], requestId),
    batchId: stringAt(node, "batchId", requestId),
    baseCheckpoint: uint64At(node, "baseCheckpoint", requestId),
    baseDigest: stringAt(node, "baseDigest", requestId),
    throughCheckpoint: uint64At(node, "throughCheckpoint", requestId),
    resultDigest: stringAt(node, "resultDigest", requestId),
    encodedBytes: uint32At(node, "encodedBytes", requestId),
    digestVerified: boolAt(node, "digestVerified", requestId),
    records: recordsAt(node["records"], requestId),
  )

proc cancelAt(node: JsonNode; requestId: string): CancelFrame =
  requireShape(node, ["envelope", "reason"], requestId)
  CancelFrame(
    envelope: envelopeAt(node["envelope"], requestId),
    reason: stringAt(node, "reason", requestId),
  )

proc stateAt(node: JsonNode; requestId: string): SyncState =
  requireShape(
    node,
    [
      "valid",
      "revision",
      "phase",
      "sessionId",
      "communityId",
      "localNodeId",
      "remoteNodeId",
      "checkpoint",
      "checkpointDigest",
      "remoteCheckpoint",
      "remoteDigest",
      "maxRecords",
      "maxEncodedBytes",
      "timeoutTicks",
      "deadlineTick",
      "pendingBatchId",
    ],
    requestId,
  )
  SyncState(
    valid: boolAt(node, "valid", requestId),
    revision: uint64At(node, "revision", requestId),
    phase: phaseAt(node, "phase", requestId),
    sessionId: stringAt(node, "sessionId", requestId),
    communityId: stringAt(node, "communityId", requestId),
    localNodeId: stringAt(node, "localNodeId", requestId),
    remoteNodeId: stringAt(node, "remoteNodeId", requestId),
    checkpoint: uint64At(node, "checkpoint", requestId),
    checkpointDigest: stringAt(node, "checkpointDigest", requestId),
    remoteCheckpoint: uint64At(node, "remoteCheckpoint", requestId),
    remoteDigest: stringAt(node, "remoteDigest", requestId),
    maxRecords: uint16At(node, "maxRecords", requestId),
    maxEncodedBytes: uint32At(node, "maxEncodedBytes", requestId),
    timeoutTicks: uint64At(node, "timeoutTicks", requestId),
    deadlineTick: uint64At(node, "deadlineTick", requestId),
    pendingBatchId: stringAt(node, "pendingBatchId", requestId),
  )

proc planAt(node: JsonNode; requestId: string): RangeBatchPlan =
  requireShape(
    node,
    [
      "effect",
      "error",
      "beforeState",
      "inflightState",
      "nextState",
      "expectedCheckpoint",
      "throughCheckpoint",
      "records",
    ],
    requestId,
  )
  proc effectAt(): SyncEffect =
    case stringAt(node, "effect", requestId)
    of "reject": syReject
    of "noop": syNoop
    of "request_range": syRequestRange
    of "apply_batch": syApplyBatch
    of "acknowledge_duplicate": syAcknowledgeDuplicate
    of "complete": syComplete
    of "cancel": syCancel
    else: invalid(requestId, "effect is not a sync effect")
  proc errorAt(): SyncError =
    case stringAt(node, "error", requestId)
    of "none": seNone
    of "invalid_state": seInvalidState
    of "protocol_mismatch": seProtocolMismatch
    of "scope_mismatch": seScopeMismatch
    of "digest_invalid": seDigestInvalid
    of "digest_mismatch": seDigestMismatch
    of "remote_behind": seRemoteBehind
    of "phase_invalid": sePhaseInvalid
    of "backpressure": seBackpressure
    of "batch_bounds": seBatchBounds
    of "sequence_gap": seSequenceGap
    of "peer_timeout": sePeerTimeout
    of "cancelled": seCancelled
    of "store_failure": seStoreFailure
    of "store_checkpoint_mismatch": seStoreCheckpointMismatch
    of "stale_settlement": seStaleSettlement
    of "reason_required": seReasonRequired
    of "tick_overflow": seTickOverflow
    of "revision_overflow": seRevisionOverflow
    else: invalid(requestId, "error is not a sync error")
  RangeBatchPlan(
    effect: effectAt(),
    error: errorAt(),
    beforeState: stateAt(node["beforeState"], requestId),
    inflightState: stateAt(node["inflightState"], requestId),
    nextState: stateAt(node["nextState"], requestId),
    expectedCheckpoint: uint64At(node, "expectedCheckpoint", requestId),
    throughCheckpoint: uint64At(node, "throughCheckpoint", requestId),
    records: recordsAt(node["records"], requestId),
  )

proc phaseName(value: SyncPhase): string =
  case value
  of spIdle: "idle"
  of spWaitingBatch: "waiting_batch"
  of spApplying: "applying"
  of spComplete: "complete"
  of spCancelled: "cancelled"

proc effectName(value: SyncEffect): string =
  case value
  of syReject: "reject"
  of syNoop: "noop"
  of syRequestRange: "request_range"
  of syRequestSnapshot: "request_snapshot"
  of syApplyBatch: "apply_batch"
  of syAcknowledgeDuplicate: "acknowledge_duplicate"
  of syComplete: "complete"
  of syCancel: "cancel"

proc errorName(value: SyncError): string =
  case value
  of seNone: "none"
  of seInvalidState: "invalid_state"
  of seProtocolMismatch: "protocol_mismatch"
  of seScopeMismatch: "scope_mismatch"
  of seDigestInvalid: "digest_invalid"
  of seDigestMismatch: "digest_mismatch"
  of seRemoteBehind: "remote_behind"
  of sePhaseInvalid: "phase_invalid"
  of seBackpressure: "backpressure"
  of seBatchBounds: "batch_bounds"
  of seSequenceGap: "sequence_gap"
  of sePeerTimeout: "peer_timeout"
  of seCancelled: "cancelled"
  of seStoreFailure: "store_failure"
  of seStoreCheckpointMismatch: "store_checkpoint_mismatch"
  of seStaleSettlement: "stale_settlement"
  of seReasonRequired: "reason_required"
  of seTickOverflow: "tick_overflow"
  of seRevisionOverflow: "revision_overflow"

proc envelopeNode(value: SyncEnvelope): JsonNode =
  %*{
    "protocol": value.protocol,
    "version": value.version,
    "sessionId": value.sessionId,
    "communityId": value.communityId,
    "senderNodeId": value.senderNodeId,
    "receiverNodeId": value.receiverNodeId,
  }

proc rangeRequestNode(value: RangeRequestFrame): JsonNode =
  result = %*{
    "afterCheckpoint": value.afterCheckpoint,
    "limitRecords": value.limitRecords,
    "maxEncodedBytes": value.maxEncodedBytes,
  }
  result["envelope"] = envelopeNode(value.envelope)

proc recordNode(value: SyncRecord): JsonNode =
  %*{
    "sequence": value.sequence,
    "recordType": value.recordType,
    "key": value.key,
    "deleted": value.deleted,
    "payload": value.payload,
    "contentDigest": value.contentDigest,
  }

proc recordsNode(values: seq[SyncRecord]): JsonNode =
  result = newJArray()
  for value in values:
    result.add(recordNode(value))

proc stateNode(value: SyncState): JsonNode =
  %*{
    "valid": value.valid,
    "revision": value.revision,
    "phase": phaseName(value.phase),
    "sessionId": value.sessionId,
    "communityId": value.communityId,
    "localNodeId": value.localNodeId,
    "remoteNodeId": value.remoteNodeId,
    "checkpoint": value.checkpoint,
    "checkpointDigest": value.checkpointDigest,
    "remoteCheckpoint": value.remoteCheckpoint,
    "remoteDigest": value.remoteDigest,
    "maxRecords": value.maxRecords,
    "maxEncodedBytes": value.maxEncodedBytes,
    "timeoutTicks": value.timeoutTicks,
    "deadlineTick": value.deadlineTick,
    "pendingBatchId": value.pendingBatchId,
  }

proc decisionNode(value: SyncDecision): JsonNode =
  result = %*{"effect": effectName(value.effect), "error": errorName(value.error)}
  result["state"] = stateNode(value.state)

proc planNode(value: RangeBatchPlan): JsonNode =
  result = %*{
    "effect": effectName(value.effect),
    "error": errorName(value.error),
    "expectedCheckpoint": value.expectedCheckpoint,
    "throughCheckpoint": value.throughCheckpoint,
  }
  result["beforeState"] = stateNode(value.beforeState)
  result["inflightState"] = stateNode(value.inflightState)
  result["nextState"] = stateNode(value.nextState)
  result["records"] = recordsNode(value.records)

proc readPlanNode(value: RangeReadPlan): JsonNode =
  %*{
    "allowed": value.allowed,
    "error": errorName(value.error),
    "communityId": value.communityId,
    "afterCheckpoint": value.afterCheckpoint,
    "limitRecords": value.limitRecords,
    "maxEncodedBytes": value.maxEncodedBytes,
  }

proc executeSyncPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "sync policy decision is required")

  let decision = payload["decision"].getStr()
  case decision
  of "accept_digest":
    requireShape(payload, ["decision", "state", "frame", "nowTick"], requestId)
    result = %*{"decision": decision}
    result["result"] = decisionNode(
      acceptRemoteDigest(
        stateAt(payload["state"], requestId),
        digestAt(payload["frame"], requestId),
        uint64At(payload, "nowTick", requestId),
      )
    )
  of "next_range":
    requireShape(payload, ["decision", "state"], requestId)
    result = %*{"decision": decision}
    let frame = nextRangeRequest(stateAt(payload["state"], requestId))
    result["frame"] = if frame.isSome: rangeRequestNode(frame.get()) else: newJNull()
  of "plan_range_read":
    requireShape(
      payload,
      [
        "decision",
        "frame",
        "sessionId",
        "communityId",
        "sourceNodeId",
        "targetNodeId",
        "sourceCheckpoint",
      ],
      requestId,
    )
    result = %*{"decision": decision}
    result["plan"] = readPlanNode(
      planRangeRead(
        rangeRequestAt(payload["frame"], requestId),
        stringAt(payload, "sessionId", requestId),
        stringAt(payload, "communityId", requestId),
        stringAt(payload, "sourceNodeId", requestId),
        stringAt(payload, "targetNodeId", requestId),
        uint64At(payload, "sourceCheckpoint", requestId),
      )
    )
  of "plan_batch":
    requireShape(payload, ["decision", "state", "frame", "nowTick"], requestId)
    result = %*{"decision": decision}
    result["plan"] = planNode(
      planRangeBatch(
        stateAt(payload["state"], requestId),
        batchAt(payload["frame"], requestId),
        uint64At(payload, "nowTick", requestId),
      )
    )
  of "settle_batch":
    requireShape(
      payload,
      [
        "decision",
        "plan",
        "currentState",
        "storeSucceeded",
        "committedCheckpoint",
      ],
      requestId,
    )
    result = %*{"decision": decision}
    result["result"] = decisionNode(
      settleRangeBatch(
        planAt(payload["plan"], requestId),
        stateAt(payload["currentState"], requestId),
        boolAt(payload, "storeSucceeded", requestId),
        uint64At(payload, "committedCheckpoint", requestId),
      )
    )
  of "stop":
    requireShape(payload, ["decision", "state", "reason"], requestId)
    result = %*{"decision": decision}
    result["result"] = decisionNode(
      stopSync(
        stateAt(payload["state"], requestId),
        stringAt(payload, "reason", requestId),
      )
    )
  of "cancel":
    requireShape(payload, ["decision", "state", "frame"], requestId)
    result = %*{"decision": decision}
    result["result"] = decisionNode(
      cancelSync(
        stateAt(payload["state"], requestId),
        cancelAt(payload["frame"], requestId),
      )
    )
  of "check_deadline":
    requireShape(payload, ["decision", "state", "nowTick"], requestId)
    result = %*{"decision": decision}
    result["result"] = decisionNode(
      checkSyncDeadline(
        stateAt(payload["state"], requestId),
        uint64At(payload, "nowTick", requestId),
      )
    )
  of "merge_inventory":
    requireShape(payload, ["decision", "communityId", "records"], requestId)
    result = %*{"decision": decision}
    result["results"] = mergeInventoryNode(payload, requestId)
  else:
    invalid(requestId, "unknown sync policy decision")
