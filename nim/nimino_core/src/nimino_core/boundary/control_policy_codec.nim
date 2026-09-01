## Strict JSON adapter for replicated control policy.

import std/[json, options]

import ../domain/control_state_machine
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError("INVALID_REQUEST", message, requestId, "domain.control.policy")

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "control policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "control policy payload is missing " & key)

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

proc stringsAt(node: JsonNode; key, requestId: string): seq[string] =
  if node[key].kind != JArray:
    invalid(requestId, key & " must be an array")
  for item in node[key].items:
    if item.kind != JString:
      invalid(requestId, key & " must contain strings")
    result.add(item.getStr())

proc optionalStringAt(node: JsonNode; key, requestId: string): Option[string] =
  case node[key].kind
  of JNull: none(string)
  of JString: some(node[key].getStr())
  else: invalid(requestId, key & " must be a string or null")

proc phaseAt(node: JsonNode; key, requestId: string): ControlVoterPhase =
  case stringAt(node, key, requestId)
  of "stable_old": cvStableOld
  of "joint": cvJoint
  of "stable_new": cvStableNew
  else: invalid(requestId, key & " is not a voter phase")

proc entryKindAt(node: JsonNode; key, requestId: string): ControlEntryKind =
  case stringAt(node, key, requestId)
  of "command": cekCommand
  of "begin_joint": cekBeginJoint
  of "finalize": cekFinalize
  else: invalid(requestId, key & " is not a control entry kind")

proc effectAt(node: JsonNode; key, requestId: string): ControlEffect =
  case stringAt(node, key, requestId)
  of "reject": ceReject
  of "vote": ceVote
  of "elect_leader": ceElectLeader
  of "append": ceAppend
  of "commit": ceCommit
  of "apply": ceApply
  of "snapshot": ceSnapshot
  else: invalid(requestId, key & " is not a control effect")

proc errorAt(node: JsonNode; key, requestId: string): ControlStateError =
  case stringAt(node, key, requestId)
  of "none": cseNone
  of "invalid_voters": cseInvalidVoters
  of "stale_term": cseStaleTerm
  of "candidate_not_voter": cseCandidateNotVoter
  of "candidate_log_stale": cseCandidateLogStale
  of "quorum_required": cseQuorumRequired
  of "leader_required": cseLeaderRequired
  of "authority_stale": cseAuthorityStale
  of "pending_entry": csePendingEntry
  of "entry_kind_invalid": cseEntryKindInvalid
  of "command_required": cseCommandRequired
  of "log_gap": cseLogGap
  of "commit_order": cseCommitOrder
  of "apply_order": cseApplyOrder
  of "snapshot_unavailable": cseSnapshotUnavailable
  of "fact_conflict": cseFactConflict
  of "store_failure": cseStoreFailure
  of "corrupt_recovery": cseCorruptRecovery
  else: invalid(requestId, key & " is not a control error")

proc actionKindAt(node: JsonNode; key, requestId: string): ControlStoreActionKind =
  case stringAt(node, key, requestId)
  of "metadata": cpaMetadata
  of "log": cpaLog
  of "snapshot": cpaSnapshot
  else: invalid(requestId, key & " is not a control store action")

proc entryAt(node: JsonNode; requestId: string): ControlEntry =
  requireShape(
    node,
    ["index", "term", "voterEpoch", "kind", "commandId", "payload", "targetVoters"],
    requestId,
  )
  ControlEntry(
    index: uint64At(node, "index", requestId),
    term: uint64At(node, "term", requestId),
    voterEpoch: uint64At(node, "voterEpoch", requestId),
    kind: entryKindAt(node, "kind", requestId),
    commandId: stringAt(node, "commandId", requestId),
    payload: stringAt(node, "payload", requestId),
    targetVoters: stringsAt(node, "targetVoters", requestId),
  )

proc entriesAt(node: JsonNode; key, requestId: string): seq[ControlEntry] =
  if node[key].kind != JArray:
    invalid(requestId, key & " must be an array")
  for item in node[key].items:
    result.add(entryAt(item, requestId))

proc snapshotAt(node: JsonNode; requestId: string): ControlSnapshotState =
  requireShape(
    node,
    [
      "lastIncludedIndex", "lastIncludedTerm", "voterEpoch", "phase",
      "oldVoters", "newVoters", "statePayload",
    ],
    requestId,
  )
  ControlSnapshotState(
    lastIncludedIndex: uint64At(node, "lastIncludedIndex", requestId),
    lastIncludedTerm: uint64At(node, "lastIncludedTerm", requestId),
    voterEpoch: uint64At(node, "voterEpoch", requestId),
    phase: phaseAt(node, "phase", requestId),
    oldVoters: stringsAt(node, "oldVoters", requestId),
    newVoters: stringsAt(node, "newVoters", requestId),
    statePayload: stringAt(node, "statePayload", requestId),
  )

proc optionalSnapshotAt(
    node: JsonNode; key, requestId: string
): Option[ControlSnapshotState] =
  if node[key].kind == JNull:
    none(ControlSnapshotState)
  else:
    some(snapshotAt(node[key], requestId))

proc stateAt(node: JsonNode; requestId: string): ControlState =
  requireShape(
    node,
    [
      "valid", "metadataRevision", "term", "votedFor", "voterEpoch", "phase",
      "oldVoters", "newVoters", "leaderId", "leaderTerm", "leaderProof",
      "lastIndex", "commitIndex", "appliedIndex", "snapshot", "log",
    ],
    requestId,
  )
  ControlState(
    valid: boolAt(node, "valid", requestId),
    metadataRevision: uint64At(node, "metadataRevision", requestId),
    term: uint64At(node, "term", requestId),
    votedFor: optionalStringAt(node, "votedFor", requestId),
    voterEpoch: uint64At(node, "voterEpoch", requestId),
    phase: phaseAt(node, "phase", requestId),
    oldVoters: stringsAt(node, "oldVoters", requestId),
    newVoters: stringsAt(node, "newVoters", requestId),
    leaderId: stringAt(node, "leaderId", requestId),
    leaderTerm: uint64At(node, "leaderTerm", requestId),
    leaderProof: stringsAt(node, "leaderProof", requestId),
    lastIndex: uint64At(node, "lastIndex", requestId),
    commitIndex: uint64At(node, "commitIndex", requestId),
    appliedIndex: uint64At(node, "appliedIndex", requestId),
    snapshot: optionalSnapshotAt(node, "snapshot", requestId),
    log: entriesAt(node, "log", requestId),
  )

proc actionAt(node: JsonNode; requestId: string): ControlStoreAction =
  requireShape(
    node, ["kind", "expectedMetadataRevision", "previousIndex"], requestId
  )
  ControlStoreAction(
    kind: actionKindAt(node, "kind", requestId),
    expectedMetadataRevision: uint64At(node, "expectedMetadataRevision", requestId),
    previousIndex: uint64At(node, "previousIndex", requestId),
  )

proc planAt(node: JsonNode; requestId: string): ControlPlan =
  requireShape(
    node,
    ["effect", "error", "beforeState", "nextState", "actions", "appliedEntry"],
    requestId,
  )
  var actions: seq[ControlStoreAction]
  if node["actions"].kind != JArray:
    invalid(requestId, "actions must be an array")
  for item in node["actions"].items:
    actions.add(actionAt(item, requestId))
  ControlPlan(
    effect: effectAt(node, "effect", requestId),
    error: errorAt(node, "error", requestId),
    beforeState: stateAt(node["beforeState"], requestId),
    nextState: stateAt(node["nextState"], requestId),
    actions: actions,
    appliedEntry: (if node["appliedEntry"].kind == JNull:
        none(ControlEntry)
      else:
        some(entryAt(node["appliedEntry"], requestId))),
  )

proc phaseName(value: ControlVoterPhase): string =
  case value
  of cvStableOld: "stable_old"
  of cvJoint: "joint"
  of cvStableNew: "stable_new"

proc entryKindName(value: ControlEntryKind): string =
  case value
  of cekCommand: "command"
  of cekBeginJoint: "begin_joint"
  of cekFinalize: "finalize"

proc effectName(value: ControlEffect): string =
  case value
  of ceReject: "reject"
  of ceVote: "vote"
  of ceElectLeader: "elect_leader"
  of ceAppend: "append"
  of ceCommit: "commit"
  of ceApply: "apply"
  of ceSnapshot: "snapshot"

proc errorName(value: ControlStateError): string =
  case value
  of cseNone: "none"
  of cseInvalidVoters: "invalid_voters"
  of cseStaleTerm: "stale_term"
  of cseCandidateNotVoter: "candidate_not_voter"
  of cseCandidateLogStale: "candidate_log_stale"
  of cseQuorumRequired: "quorum_required"
  of cseLeaderRequired: "leader_required"
  of cseAuthorityStale: "authority_stale"
  of csePendingEntry: "pending_entry"
  of cseEntryKindInvalid: "entry_kind_invalid"
  of cseCommandRequired: "command_required"
  of cseLogGap: "log_gap"
  of cseCommitOrder: "commit_order"
  of cseApplyOrder: "apply_order"
  of cseSnapshotUnavailable: "snapshot_unavailable"
  of cseFactConflict: "fact_conflict"
  of cseStoreFailure: "store_failure"
  of cseCorruptRecovery: "corrupt_recovery"

proc actionKindName(value: ControlStoreActionKind): string =
  case value
  of cpaMetadata: "metadata"
  of cpaLog: "log"
  of cpaSnapshot: "snapshot"

proc stringsNode(values: seq[string]): JsonNode =
  result = newJArray()
  for value in values:
    result.add(%value)

proc entryNode(value: ControlEntry): JsonNode =
  %*{
    "index": value.index,
    "term": value.term,
    "voterEpoch": value.voterEpoch,
    "kind": entryKindName(value.kind),
    "commandId": value.commandId,
    "payload": value.payload,
    "targetVoters": stringsNode(value.targetVoters),
  }

proc entriesNode(values: seq[ControlEntry]): JsonNode =
  result = newJArray()
  for value in values:
    result.add(entryNode(value))

proc snapshotNode(value: ControlSnapshotState): JsonNode =
  %*{
    "lastIncludedIndex": value.lastIncludedIndex,
    "lastIncludedTerm": value.lastIncludedTerm,
    "voterEpoch": value.voterEpoch,
    "phase": phaseName(value.phase),
    "oldVoters": stringsNode(value.oldVoters),
    "newVoters": stringsNode(value.newVoters),
    "statePayload": value.statePayload,
  }

proc stateNode(value: ControlState): JsonNode =
  result = %*{
    "valid": value.valid,
    "metadataRevision": value.metadataRevision,
    "term": value.term,
    "voterEpoch": value.voterEpoch,
    "phase": phaseName(value.phase),
    "oldVoters": stringsNode(value.oldVoters),
    "newVoters": stringsNode(value.newVoters),
    "leaderId": value.leaderId,
    "leaderTerm": value.leaderTerm,
    "leaderProof": stringsNode(value.leaderProof),
    "lastIndex": value.lastIndex,
    "commitIndex": value.commitIndex,
    "appliedIndex": value.appliedIndex,
    "log": entriesNode(value.log),
  }
  result["votedFor"] = if value.votedFor.isSome: %value.votedFor.get() else: newJNull()
  result["snapshot"] = if value.snapshot.isSome:
      snapshotNode(value.snapshot.get())
    else:
      newJNull()

proc actionNode(value: ControlStoreAction): JsonNode =
  %*{
    "kind": actionKindName(value.kind),
    "expectedMetadataRevision": value.expectedMetadataRevision,
    "previousIndex": value.previousIndex,
  }

proc planNode(value: ControlPlan): JsonNode =
  var actions = newJArray()
  for action in value.actions:
    actions.add(actionNode(action))
  result = %*{
    "effect": effectName(value.effect),
    "error": errorName(value.error),
    "actions": actions,
  }
  result["beforeState"] = stateNode(value.beforeState)
  result["nextState"] = stateNode(value.nextState)
  result["appliedEntry"] = if value.appliedEntry.isSome:
      entryNode(value.appliedEntry.get())
    else:
      newJNull()

proc decisionNode(value: ControlDecision): JsonNode =
  result = %*{"effect": effectName(value.effect), "error": errorName(value.error)}
  result["state"] = stateNode(value.state)
  result["appliedEntry"] = if value.appliedEntry.isSome:
      entryNode(value.appliedEntry.get())
    else:
      newJNull()

proc executeControlPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "control policy decision is required")
  let decision = payload["decision"].getStr()
  case decision
  of "vote":
    requireShape(payload, ["decision", "state", "request"], requestId)
    let request = payload["request"]
    requireShape(request, ["term", "candidateId", "lastIndex", "lastTerm"], requestId)
    result = %*{"decision": "plan"}
    result["plan"] = planNode(
      planVote(
        stateAt(payload["state"], requestId),
        VoteRequest(
          term: uint64At(request, "term", requestId),
          candidateId: stringAt(request, "candidateId", requestId),
          lastIndex: uint64At(request, "lastIndex", requestId),
          lastTerm: uint64At(request, "lastTerm", requestId),
        ),
      )
    )
  of "election":
    requireShape(payload, ["decision", "state", "request"], requestId)
    let request = payload["request"]
    requireShape(request, ["term", "candidateId", "supporters"], requestId)
    result = %*{"decision": "plan"}
    result["plan"] = planNode(
      planElection(
        stateAt(payload["state"], requestId),
        ElectionRequest(
          term: uint64At(request, "term", requestId),
          candidateId: stringAt(request, "candidateId", requestId),
          supporters: stringsAt(request, "supporters", requestId),
        ),
      )
    )
  of "append":
    requireShape(payload, ["decision", "state", "request"], requestId)
    let request = payload["request"]
    requireShape(
      request,
      ["leaderId", "term", "kind", "commandId", "payload", "targetVoters"],
      requestId,
    )
    result = %*{"decision": "plan"}
    result["plan"] = planNode(
      planAppend(
        stateAt(payload["state"], requestId),
        AppendRequest(
          leaderId: stringAt(request, "leaderId", requestId),
          term: uint64At(request, "term", requestId),
          kind: entryKindAt(request, "kind", requestId),
          commandId: stringAt(request, "commandId", requestId),
          payload: stringAt(request, "payload", requestId),
          targetVoters: stringsAt(request, "targetVoters", requestId),
        ),
      )
    )
  of "replicate":
    requireShape(payload, ["decision", "state", "request"], requestId)
    let request = payload["request"]
    requireShape(
      request,
      ["leaderId", "term", "supporters", "previousIndex", "entry"],
      requestId,
    )
    result = %*{"decision": "plan"}
    result["plan"] = planNode(
      planReplication(
        stateAt(payload["state"], requestId),
        ReplicationRequest(
          leaderId: stringAt(request, "leaderId", requestId),
          term: uint64At(request, "term", requestId),
          supporters: stringsAt(request, "supporters", requestId),
          previousIndex: uint64At(request, "previousIndex", requestId),
          entry: entryAt(request["entry"], requestId),
        ),
      )
    )
  of "commit":
    requireShape(payload, ["decision", "state", "request"], requestId)
    let request = payload["request"]
    requireShape(request, ["index", "leaderId", "term", "supporters"], requestId)
    result = %*{"decision": "plan"}
    result["plan"] = planNode(
      planCommit(
        stateAt(payload["state"], requestId),
        CommitRequest(
          index: uint64At(request, "index", requestId),
          leaderId: stringAt(request, "leaderId", requestId),
          term: uint64At(request, "term", requestId),
          supporters: stringsAt(request, "supporters", requestId),
        ),
      )
    )
  of "quorum":
    requireShape(payload, ["decision", "state", "request"], requestId)
    let request = payload["request"]
    requireShape(request, ["supporters"], requestId)
    let decision = checkControlQuorum(
      stateAt(payload["state"], requestId),
      stringsAt(request, "supporters", requestId),
    )
    result = %*{
      "decision": "quorum",
      "result": {
        "granted": decision.granted,
        "error": errorName(decision.error),
      },
    }
  of "apply":
    requireShape(payload, ["decision", "state"], requestId)
    result = %*{"decision": "plan"}
    result["plan"] = planNode(planApply(stateAt(payload["state"], requestId)))
  of "snapshot":
    requireShape(payload, ["decision", "state", "statePayload"], requestId)
    result = %*{"decision": "plan"}
    result["plan"] = planNode(
      planSnapshot(
        stateAt(payload["state"], requestId),
        stringAt(payload, "statePayload", requestId),
      )
    )
  of "settle":
    requireShape(payload, ["decision", "plan", "storeSucceeded"], requestId)
    result = %*{"decision": "settle"}
    result["result"] = decisionNode(
      settleControlPlan(
        planAt(payload["plan"], requestId),
        boolAt(payload, "storeSucceeded", requestId),
      )
    )
  of "recover":
    requireShape(payload, ["decision", "input"], requestId)
    let input = payload["input"]
    requireShape(
      input,
      [
        "metadataRevision", "term", "votedFor", "commitIndex", "appliedIndex",
        "initialVoters", "snapshot", "entries",
      ],
      requestId,
    )
    let recovery = recoverControlState(
      RecoveryInput(
        metadataRevision: uint64At(input, "metadataRevision", requestId),
        term: uint64At(input, "term", requestId),
        votedFor: optionalStringAt(input, "votedFor", requestId),
        commitIndex: uint64At(input, "commitIndex", requestId),
        appliedIndex: uint64At(input, "appliedIndex", requestId),
        initialVoters: stringsAt(input, "initialVoters", requestId),
        snapshot: optionalSnapshotAt(input, "snapshot", requestId),
        entries: entriesAt(input, "entries", requestId),
      )
    )
    result = %*{"decision": "recover"}
    result["result"] = %*{"error": errorName(recovery.error)}
    result["result"]["state"] = stateNode(recovery.state)
  else:
    invalid(requestId, "unknown control policy decision")
