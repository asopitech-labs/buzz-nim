## Strict JSON adapter for workflow effect-ledger policy.

import std/[json, options]

import ../domain/effect_ledger
import ../domain/lease_fencing
import ./lease_policy_codec
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError("INVALID_REQUEST", message, requestId, "domain.effect.policy")

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "effect policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "effect policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc boolAt(node: JsonNode; key, requestId: string): bool =
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean")
  node[key].getBool()

proc uint64At(node: JsonNode; key, requestId: string): uint64 =
  if node[key].kind != JInt or node[key].getBiggestInt() < 0:
    invalid(requestId, key & " must be a non-negative integer")
  uint64(node[key].getBiggestInt())

proc statusAt(value, requestId: string): EffectLedgerStatus =
  case value
  of "pending": elsPending
  of "claimed": elsClaimed
  of "executing": elsExecuting
  of "succeeded": elsSucceeded
  of "failed": elsFailed
  of "unknown": elsUnknown
  else: invalid(requestId, "status is not an effect ledger status")

proc outcomeAt(value, requestId: string): EffectReceiptOutcome =
  case value
  of "succeeded": eroSucceeded
  of "failed": eroFailed
  else: invalid(requestId, "outcome is not an effect receipt outcome")

proc reconcileCommandAt(value, requestId: string): EffectReconcileCommand =
  case value
  of "mark_succeeded": ercMarkSucceeded
  of "mark_failed": ercMarkFailed
  of "retry": ercRetry
  else: invalid(requestId, "command is not an effect reconcile command")

proc receiptAt(node: JsonNode; requestId: string): EffectReceipt =
  requireShape(node, ["outcome", "receiptId", "resultDigest"], requestId)
  EffectReceipt(
    outcome: outcomeAt(stringAt(node, "outcome", requestId), requestId),
    receiptId: stringAt(node, "receiptId", requestId),
    resultDigest: stringAt(node, "resultDigest", requestId),
  )

proc stateAt(node: JsonNode; requestId: string): EffectLedgerState =
  requireShape(
    node,
    [
      "valid", "communityId", "workflowId", "runId", "stepId",
      "idempotencyKey", "effectDigest", "leaseResourceId", "revision",
      "attempt", "status", "ownerNodeId", "fenceToken", "receipt",
      "reconciledBy", "reconcileReason",
    ],
    requestId,
  )
  let attempt = uint64At(node, "attempt", requestId)
  if attempt > uint64(high(uint32)):
    invalid(requestId, "attempt exceeds uint32")
  result = EffectLedgerState(
    valid: boolAt(node, "valid", requestId),
    communityId: stringAt(node, "communityId", requestId),
    workflowId: stringAt(node, "workflowId", requestId),
    runId: stringAt(node, "runId", requestId),
    stepId: stringAt(node, "stepId", requestId),
    idempotencyKey: stringAt(node, "idempotencyKey", requestId),
    effectDigest: stringAt(node, "effectDigest", requestId),
    leaseResourceId: stringAt(node, "leaseResourceId", requestId),
    revision: uint64At(node, "revision", requestId),
    attempt: uint32(attempt),
    status: statusAt(stringAt(node, "status", requestId), requestId),
    ownerNodeId: stringAt(node, "ownerNodeId", requestId),
    fenceToken: uint64At(node, "fenceToken", requestId),
    reconciledBy: stringAt(node, "reconciledBy", requestId),
    reconcileReason: stringAt(node, "reconcileReason", requestId),
  )
  result.receipt = if node["receipt"].kind == JNull:
      none(EffectReceipt)
    else:
      some(receiptAt(node["receipt"], requestId))

proc effectAt(value, requestId: string): EffectLedgerEffect =
  case value
  of "reject": efeReject
  of "replay": efeReplay
  of "claimed": efeClaimed
  of "execute_external": efeExecuteExternal
  of "receipt_recorded": efeReceiptRecorded
  of "claim_recovered": efeClaimRecovered
  of "unknown": efeUnknown
  of "manual_retry": efeManualRetry
  of "reconciled": efeReconciled
  else: invalid(requestId, "effect is not an effect ledger effect")

proc errorAt(value, requestId: string): EffectLedgerError =
  case value
  of "none": eleNone
  of "invalid_state": eleInvalidState
  of "invalid_identity": eleInvalidIdentity
  of "invalid_receipt": eleInvalidReceipt
  of "lease_rejected": eleLeaseRejected
  of "owner_mismatch": eleOwnerMismatch
  of "fence_mismatch": eleFenceMismatch
  of "claim_conflict": eleClaimConflict
  of "manual_reconcile_required": eleManualReconcileRequired
  of "reconcile_unauthorized": eleReconcileUnauthorized
  of "reconcile_reason_required": eleReconcileReasonRequired
  of "terminal_conflict": eleTerminalConflict
  of "persistence_failure": elePersistenceFailure
  of "revision_overflow": eleRevisionOverflow
  of "attempt_overflow": eleAttemptOverflow
  else: invalid(requestId, "error is not an effect ledger error")

proc leaseErrorAt(value, requestId: string): LeaseFenceError =
  for error in LeaseFenceError:
    if leaseErrorName(error) == value:
      return error
  invalid(requestId, "leaseError is not a lease fence error")

proc portEffectAt(value, requestId: string): EffectLedgerPortEffect =
  case value
  of "none": elpNone
  of "commit_canonical": elpCommitCanonical
  else: invalid(requestId, "portEffect is not an effect ledger port effect")

proc planAt(node: JsonNode; requestId: string): EffectLedgerPlan =
  requireShape(
    node,
    ["effect", "error", "leaseError", "portEffect", "beforeState", "nextState"],
    requestId,
  )
  EffectLedgerPlan(
    effect: effectAt(stringAt(node, "effect", requestId), requestId),
    error: errorAt(stringAt(node, "error", requestId), requestId),
    leaseError: leaseErrorAt(stringAt(node, "leaseError", requestId), requestId),
    portEffect: portEffectAt(stringAt(node, "portEffect", requestId), requestId),
    beforeState: stateAt(node["beforeState"], requestId),
    nextState: stateAt(node["nextState"], requestId),
  )

proc receiptNode(value: EffectReceipt): JsonNode =
  %*{
    "outcome": (if value.outcome == eroSucceeded: "succeeded" else: "failed"),
    "receiptId": value.receiptId,
    "resultDigest": value.resultDigest,
  }

proc statusName(value: EffectLedgerStatus): string =
  case value
  of elsPending: "pending"
  of elsClaimed: "claimed"
  of elsExecuting: "executing"
  of elsSucceeded: "succeeded"
  of elsFailed: "failed"
  of elsUnknown: "unknown"

proc stateNode(value: EffectLedgerState): JsonNode =
  result = %*{
    "valid": value.valid,
    "communityId": value.communityId,
    "workflowId": value.workflowId,
    "runId": value.runId,
    "stepId": value.stepId,
    "idempotencyKey": value.idempotencyKey,
    "effectDigest": value.effectDigest,
    "leaseResourceId": value.leaseResourceId,
    "revision": value.revision,
    "attempt": value.attempt,
    "status": statusName(value.status),
    "ownerNodeId": value.ownerNodeId,
    "fenceToken": value.fenceToken,
    "reconciledBy": value.reconciledBy,
    "reconcileReason": value.reconcileReason,
  }
  result["receipt"] = if value.receipt.isSome:
      receiptNode(value.receipt.get())
    else:
      newJNull()

proc effectName(value: EffectLedgerEffect): string =
  case value
  of efeReject: "reject"
  of efeReplay: "replay"
  of efeClaimed: "claimed"
  of efeExecuteExternal: "execute_external"
  of efeReceiptRecorded: "receipt_recorded"
  of efeClaimRecovered: "claim_recovered"
  of efeUnknown: "unknown"
  of efeManualRetry: "manual_retry"
  of efeReconciled: "reconciled"

proc errorName(value: EffectLedgerError): string =
  case value
  of eleNone: "none"
  of eleInvalidState: "invalid_state"
  of eleInvalidIdentity: "invalid_identity"
  of eleInvalidReceipt: "invalid_receipt"
  of eleLeaseRejected: "lease_rejected"
  of eleOwnerMismatch: "owner_mismatch"
  of eleFenceMismatch: "fence_mismatch"
  of eleClaimConflict: "claim_conflict"
  of eleManualReconcileRequired: "manual_reconcile_required"
  of eleReconcileUnauthorized: "reconcile_unauthorized"
  of eleReconcileReasonRequired: "reconcile_reason_required"
  of eleTerminalConflict: "terminal_conflict"
  of elePersistenceFailure: "persistence_failure"
  of eleRevisionOverflow: "revision_overflow"
  of eleAttemptOverflow: "attempt_overflow"

proc portEffectName(value: EffectLedgerPortEffect): string =
  if value == elpCommitCanonical: "commit_canonical" else: "none"

proc planNode(value: EffectLedgerPlan): JsonNode =
  result = %*{
    "effect": effectName(value.effect),
    "error": errorName(value.error),
    "leaseError": leaseErrorName(value.leaseError),
    "portEffect": portEffectName(value.portEffect),
  }
  result["beforeState"] = stateNode(value.beforeState)
  result["nextState"] = stateNode(value.nextState)

proc planResult(decision: string; plan: EffectLedgerPlan): JsonNode =
  result = %*{"decision": decision}
  result["result"] = planNode(plan)

proc executeEffectPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "effect policy decision is required")
  let decision = payload["decision"].getStr()
  case decision
  of "claim", "execute":
    requireShape(
      payload,
      ["decision", "state", "ownerNodeId", "fenceToken", "leaseState", "fact"],
      requestId,
    )
    let state = stateAt(payload["state"], requestId)
    let owner = stringAt(payload, "ownerNodeId", requestId)
    let fence = uint64At(payload, "fenceToken", requestId)
    let leaseState = lease_policy_codec.stateAt(payload["leaseState"], requestId)
    let fact = servingFactAt(payload["fact"], requestId)
    let plan = if decision == "claim":
        planEffectClaim(state, owner, fence, leaseState, fact)
      else:
        planEffectExecution(state, owner, fence, leaseState, fact)
    result = planResult(decision, plan)
  of "receipt":
    requireShape(
      payload,
      ["decision", "state", "ownerNodeId", "fenceToken", "receipt"],
      requestId,
    )
    result = planResult(
      decision,
      planEffectReceipt(
        stateAt(payload["state"], requestId),
        stringAt(payload, "ownerNodeId", requestId),
        uint64At(payload, "fenceToken", requestId),
        receiptAt(payload["receipt"], requestId),
      ),
    )
  of "recover":
    requireShape(payload, ["decision", "state", "leaseState", "fact"], requestId)
    result = planResult(
      decision,
      planEffectRecovery(
        stateAt(payload["state"], requestId),
        lease_policy_codec.stateAt(payload["leaseState"], requestId),
        servingFactAt(payload["fact"], requestId),
      ),
    )
  of "reconcile":
    requireShape(payload, ["decision", "state", "request"], requestId)
    let request = payload["request"]
    requireShape(
      request,
      ["operatorAuthorized", "operatorId", "reason", "command", "receipt"],
      requestId,
    )
    var reconcile = EffectReconcileRequest(
      operatorAuthorized: boolAt(request, "operatorAuthorized", requestId),
      operatorId: stringAt(request, "operatorId", requestId),
      reason: stringAt(request, "reason", requestId),
      command: reconcileCommandAt(stringAt(request, "command", requestId), requestId),
    )
    reconcile.receipt = if request["receipt"].kind == JNull:
        none(EffectReceipt)
      else:
        some(receiptAt(request["receipt"], requestId))
    result = planResult(
      decision,
      planEffectReconcile(stateAt(payload["state"], requestId), reconcile),
    )
  of "settle":
    requireShape(payload, ["decision", "plan", "persistenceSucceeded"], requestId)
    let settled = settleEffectLedger(
      planAt(payload["plan"], requestId),
      boolAt(payload, "persistenceSucceeded", requestId),
    )
    result = %*{
      "decision": decision,
      "result": {
        "effect": effectName(settled.effect),
        "error": errorName(settled.error),
        "leaseError": leaseErrorName(settled.leaseError),
      },
    }
    result["result"]["state"] = stateNode(settled.state)
  else:
    invalid(requestId, "unknown effect policy decision")
