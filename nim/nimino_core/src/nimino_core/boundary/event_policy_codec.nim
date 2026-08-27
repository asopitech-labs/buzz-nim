## Strict JSON adapter for the typed event-policy boundary operation.

import std/[json, options]

import ../domain/event_policy
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError(
    "INVALID_REQUEST", message, requestId, "domain.event.policy"
  )

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "event policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "event policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc intAt(node: JsonNode; key, requestId: string): int64 =
  if node[key].kind != JInt:
    invalid(requestId, key & " must be an integer")
  node[key].getBiggestInt()

proc boundedIntAt(
    node: JsonNode; key, requestId: string; minimum, maximum: int64
): int64 =
  result = intAt(node, key, requestId)
  if result < minimum or result > maximum:
    invalid(requestId, key & " is out of range")

proc isHex(value: string; lowercase: bool): bool =
  if value.len != 64:
    return false
  for ch in value:
    if ch notin {'0' .. '9', 'a' .. 'f'} and
        (lowercase or ch notin {'A' .. 'F'}):
      return false
  true

proc eventIdAt(node: JsonNode; key, requestId: string): string =
  result = stringAt(node, key, requestId)
  if not isHex(result, false):
    invalid(requestId, key & " must be a 64-character hexadecimal event id")

proc authorAt(node: JsonNode; key, requestId: string): string =
  result = stringAt(node, key, requestId)
  if not isHex(result, true):
    invalid(requestId, key & " must be a canonical author key")

proc timestampAt(node: JsonNode; key, requestId: string): int64 =
  boundedIntAt(node, key, requestId, 0, int64.high)

proc boolAt(node: JsonNode; key, requestId: string): bool =
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean")
  node[key].getBool()

proc tagsAt(node: JsonNode; key, requestId: string): seq[seq[string]] =
  if node[key].kind != JArray:
    invalid(requestId, key & " must be an array of string arrays")
  for tag in node[key]:
    if tag.kind != JArray:
      invalid(requestId, key & " must be an array of string arrays")
    var parts: seq[string]
    for part in tag:
      if part.kind != JString:
        invalid(requestId, key & " must contain only strings")
      parts.add(part.getStr())
    result.add(parts)

proc stringsAt(node: JsonNode; key, requestId: string): seq[string] =
  if node[key].kind != JArray:
    invalid(requestId, key & " must be an array of strings")
  for item in node[key]:
    if item.kind != JString:
      invalid(requestId, key & " must contain only strings")
    result.add(item.getStr())

proc versionAt(node: JsonNode; requestId: string): EventVersion =
  requireShape(node, ["createdAt", "eventId"], requestId)
  EventVersion(
    createdAt: timestampAt(node, "createdAt", requestId),
    eventId: eventIdAt(node, "eventId", requestId),
  )

proc metadataAt(node: JsonNode; requestId: string): Option[ThreadMetadataFacts] =
  if node.kind == JNull:
    return none(ThreadMetadataFacts)
  requireShape(node, ["rootId", "depth"], requestId)
  some(ThreadMetadataFacts(
    rootId: eventIdAt(node, "rootId", requestId),
    depth: int(boundedIntAt(node, "depth", requestId, 0, MaxThreadDepth)),
  ))

proc parentAt(node: JsonNode; requestId: string): Option[ThreadParentFacts] =
  if node.kind == JNull:
    return none(ThreadParentFacts)
  requireShape(
    node, ["eventId", "createdAt", "channelId", "tags", "metadata"], requestId
  )
  let channelId = stringAt(node, "channelId", requestId)
  if channelId.len == 0:
    invalid(requestId, "channelId must not be empty")
  some(ThreadParentFacts(
    eventId: eventIdAt(node, "eventId", requestId),
    createdAt: timestampAt(node, "createdAt", requestId),
    channelId: channelId,
    tags: tagsAt(node, "tags", requestId),
    metadata: metadataAt(node["metadata"], requestId),
  ))

proc threadRequestAt(node: JsonNode; requestId: string): ThreadRequest =
  requireShape(
    node,
    ["eventId", "createdAt", "channelId", "tags", "parent", "rootCreatedAt"],
    requestId,
  )
  var rootCreatedAt = none(int64)
  if node["rootCreatedAt"].kind != JNull:
    rootCreatedAt = some(timestampAt(node, "rootCreatedAt", requestId))
  let channelId = stringAt(node, "channelId", requestId)
  if channelId.len == 0:
    invalid(requestId, "channelId must not be empty")
  ThreadRequest(
    eventId: eventIdAt(node, "eventId", requestId),
    createdAt: timestampAt(node, "createdAt", requestId),
    channelId: channelId,
    tags: tagsAt(node, "tags", requestId),
    parent: parentAt(node["parent"], requestId),
    rootCreatedAt: rootCreatedAt,
  )

proc deletionTargetAt(
    node: JsonNode; requestId: string
): Option[DeletionTargetFacts] =
  if node.kind == JNull:
    return none(DeletionTargetFacts)
  requireShape(
    node,
    ["eventId", "author", "createdAt", "active", "parentId", "rootId"],
    requestId,
  )
  var parentId = none(string)
  var rootId = none(string)
  if node["parentId"].kind != JNull:
    parentId = some(eventIdAt(node, "parentId", requestId))
  if node["rootId"].kind != JNull:
    rootId = some(eventIdAt(node, "rootId", requestId))
  some(DeletionTargetFacts(
    eventId: eventIdAt(node, "eventId", requestId),
    author: authorAt(node, "author", requestId),
    createdAt: timestampAt(node, "createdAt", requestId),
    active: boolAt(node, "active", requestId),
    parentId: parentId,
    rootId: rootId,
  ))

proc deletionRequestAt(node: JsonNode; requestId: string): DeletionRequest =
  requireShape(
    node, ["actor", "createdAt", "eTargets", "aTargets", "target"], requestId
  )
  let eTargets = stringsAt(node, "eTargets", requestId)
  for eventId in eTargets:
    if not isHex(eventId, false):
      invalid(requestId, "eTargets must contain only event ids")
  DeletionRequest(
    actor: authorAt(node, "actor", requestId),
    createdAt: timestampAt(node, "createdAt", requestId),
    eTargets: eTargets,
    aTargets: stringsAt(node, "aTargets", requestId),
    target: deletionTargetAt(node["target"], requestId),
  )

proc reactionRequestAt(node: JsonNode; requestId: string): ReactionRequest =
  requireShape(
    node, ["targetExists", "activeDuplicate", "content", "tags"], requestId
  )
  ReactionRequest(
    targetExists: boolAt(node, "targetExists", requestId),
    activeDuplicate: boolAt(node, "activeDuplicate", requestId),
    content: stringAt(node, "content", requestId),
    tags: tagsAt(node, "tags", requestId),
  )

proc errorName(error: EventPolicyError): string =
  case error
  of epeNone: "none"
  of epeUnsupportedKind: "unsupported_kind"
  of epeAuthNotStorable: "auth_not_storable"
  of epeDTagRequired: "d_tag_required"
  of epeDTagCardinality: "d_tag_cardinality"
  of epeDTagTooLong: "d_tag_too_long"
  of epeThreadParentMissing: "thread_parent_missing"
  of epeThreadParentMismatch: "thread_parent_mismatch"
  of epeThreadChannelMismatch: "thread_channel_mismatch"
  of epeThreadRootMismatch: "thread_root_mismatch"
  of epeThreadDepthExceeded: "thread_depth_exceeded"
  of epeDeleteTargetCardinality: "delete_target_cardinality"
  of epeDeleteTargetMissing: "delete_target_missing"
  of epeDeleteTargetMismatch: "delete_target_mismatch"
  of epeDeleteAuthorMismatch: "delete_author_mismatch"
  of epeDeleteCoordinateInvalid: "delete_coordinate_invalid"
  of epeReactionTargetMissing: "reaction_target_missing"
  of epeReactionEmojiInvalid: "reaction_emoji_invalid"

proc dispositionName(disposition: EventDisposition): string =
  case disposition
  of edRejected: "rejected"
  of edStored: "stored"
  of edEphemeral: "ephemeral"
  of edReplaceable: "replaceable"
  of edParameterized: "parameterized"

proc replacementName(action: ReplacementDecision): string =
  case action
  of rdInsert: "insert"
  of rdReplace: "replace"
  of rdDuplicate: "duplicate"
  of rdStale: "stale"

proc deletionName(action: DeletionAction): string =
  case action
  of daReject: "reject"
  of daNoop: "noop"
  of daDeleteEvent: "delete_event"
  of daDeleteCoordinate: "delete_coordinate"
  of daKeepNewer: "keep_newer"

proc reactionName(action: ReactionAction): string =
  case action
  of raReject: "reject"
  of raDuplicate: "duplicate"
  of raInsert: "insert"

proc executeEventPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "event policy decision is required")

  case payload["decision"].getStr()
  of "classify":
    requireShape(payload, ["decision", "kind", "dTagCount", "dTagLen"], requestId)
    let decision = classifyEvent(
      int(boundedIntAt(payload, "kind", requestId, 0, uint32.high.int64)),
      int(boundedIntAt(payload, "dTagCount", requestId, 0, uint16.high.int64)),
      int(boundedIntAt(payload, "dTagLen", requestId, 0, uint16.high.int64)),
    )
    %*{
      "decision": "classify",
      "disposition": dispositionName(decision.disposition),
      "error": errorName(decision.error),
    }
  of "replacement":
    requireShape(payload, ["decision", "incoming", "current"], requestId)
    var current = none(EventVersion)
    if payload["current"].kind != JNull:
      current = some(versionAt(payload["current"], requestId))
    %*{
      "decision": "replacement",
      "action": replacementName(
        decideReplacement(versionAt(payload["incoming"], requestId), current)
      ),
    }
  of "thread":
    requireShape(payload, ["decision", "request"], requestId)
    let decision = decideThread(threadRequestAt(payload["request"], requestId))
    var plan = newJNull()
    if decision.error == epeNone and decision.plan.parentId.len > 0:
      plan = %*{
        "rootId": decision.plan.rootId,
        "parentId": decision.plan.parentId,
        "rootCreatedAt": decision.plan.rootCreatedAt,
        "parentCreatedAt": decision.plan.parentCreatedAt,
        "depth": decision.plan.depth,
        "broadcast": decision.plan.broadcast,
        "parentReplyDelta": decision.plan.parentReplyDelta,
        "rootDescendantDelta": decision.plan.rootDescendantDelta,
      }
    %*{
      "decision": "thread", "error": errorName(decision.error), "plan": plan
    }
  of "deletion":
    requireShape(payload, ["decision", "request"], requestId)
    let decision = decideDeletion(deletionRequestAt(payload["request"], requestId))
    %*{
      "decision": "deletion",
      "error": errorName(decision.error),
      "action": deletionName(decision.action),
      "parentReplyDelta": decision.parentReplyDelta,
      "rootDescendantDelta": decision.rootDescendantDelta,
    }
  of "reaction":
    requireShape(payload, ["decision", "request"], requestId)
    let decision = decideReaction(reactionRequestAt(payload["request"], requestId))
    %*{
      "decision": "reaction",
      "error": errorName(decision.error),
      "action": reactionName(decision.action),
      "emoji": decision.emoji,
    }
  else:
    invalid(requestId, "unknown event policy decision")
