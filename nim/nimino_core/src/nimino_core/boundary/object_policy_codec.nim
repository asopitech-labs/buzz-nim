## Strict JSON adapter for content-addressed object policy.

import std/[json, options]

import ../domain/object_sync_policy
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError("INVALID_REQUEST", message, requestId, "domain.object.policy")

proc shape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "object policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "object policy payload is missing " & key)

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

proc u16(node: JsonNode; key, requestId: string): uint16 =
  let value = u64(node, key, requestId)
  if value > uint64(high(uint16)):
    invalid(requestId, key & " exceeds uint16")
  uint16(value)

proc strings(node: JsonNode; key, requestId: string): seq[string] =
  if node[key].kind != JArray:
    invalid(requestId, key & " must be an array")
  for item in node[key].items:
    if item.kind != JString:
      invalid(requestId, key & " must contain strings")
    result.add(item.getStr())

proc kind(value, requestId: string): ObjectKind =
  case value
  of "media": okMedia
  of "git-pack": okGitPack
  of "git-manifest": okGitManifest
  else: invalid(requestId, "kind is not an object kind")

proc descriptor(node: JsonNode; requestId: string): ObjectDescriptor =
  shape(node, ["digest", "size", "kind"], requestId)
  ObjectDescriptor(
    digest: text(node, "digest", requestId),
    size: u64(node, "size", requestId),
    kind: kind(text(node, "kind", requestId), requestId),
  )

proc manifest(node: JsonNode; requestId: string): ObjectManifest =
  shape(node, ["communityId", "manifestId", "generation", "objects"], requestId)
  result = ObjectManifest(
    communityId: text(node, "communityId", requestId),
    manifestId: text(node, "manifestId", requestId),
    generation: u64(node, "generation", requestId),
  )
  if node["objects"].kind != JArray:
    invalid(requestId, "objects must be an array")
  for item in node["objects"].items:
    result.objects.add(descriptor(item, requestId))

proc localFact(node: JsonNode; requestId: string): ObjectLocalFact =
  shape(
    node,
    [
      "digest", "size", "present", "verified", "partial", "partialOffset",
      "unreferencedSinceEpoch",
    ],
    requestId,
  )
  result = ObjectLocalFact(
    digest: text(node, "digest", requestId),
    size: u64(node, "size", requestId),
    present: flag(node, "present", requestId),
    verified: flag(node, "verified", requestId),
    partial: flag(node, "partial", requestId),
    partialOffset: u64(node, "partialOffset", requestId),
  )
  result.unreferencedSinceEpoch = if node["unreferencedSinceEpoch"].kind == JNull:
      none(uint64)
    else:
      some(u64(node, "unreferencedSinceEpoch", requestId))

proc origin(node: JsonNode; requestId: string): ObjectOriginFact =
  shape(node, ["nodeId", "available", "digests"], requestId)
  ObjectOriginFact(
    nodeId: text(node, "nodeId", requestId),
    available: flag(node, "available", requestId),
    digests: strings(node, "digests", requestId),
  )

proc syncRequest(node: JsonNode; requestId: string): ObjectSyncRequest =
  shape(
    node,
    [
      "communityId", "manifest", "manifestDigestVerified", "lifecycleAllowsSync",
      "cancelled", "mode", "requestedDigest", "pinnedDigests", "localFacts",
      "origins", "maxFetches",
    ],
    requestId,
  )
  let mode = case text(node, "mode", requestId)
    of "eager": ofmEager
    of "lazy": ofmLazy
    else: invalid(requestId, "mode is not an object fetch mode")
  result = ObjectSyncRequest(
    communityId: text(node, "communityId", requestId),
    manifest: manifest(node["manifest"], requestId),
    manifestDigestVerified: flag(node, "manifestDigestVerified", requestId),
    lifecycleAllowsSync: flag(node, "lifecycleAllowsSync", requestId),
    cancelled: flag(node, "cancelled", requestId),
    mode: mode,
    requestedDigest: text(node, "requestedDigest", requestId),
    pinnedDigests: strings(node, "pinnedDigests", requestId),
    maxFetches: u16(node, "maxFetches", requestId),
  )
  if node["localFacts"].kind != JArray or node["origins"].kind != JArray:
    invalid(requestId, "localFacts and origins must be arrays")
  for item in node["localFacts"].items:
    result.localFacts.add(localFact(item, requestId))
  for item in node["origins"].items:
    result.origins.add(origin(item, requestId))

proc pinState(node: JsonNode; requestId: string): PinState =
  shape(node, ["valid", "communityId", "revision", "digests"], requestId)
  PinState(
    valid: flag(node, "valid", requestId),
    communityId: text(node, "communityId", requestId),
    revision: u64(node, "revision", requestId),
    digests: strings(node, "digests", requestId),
  )

proc effectName(value: ObjectEffect): string =
  case value
  of oaeReject: "reject"
  of oaeComplete: "complete"
  of oaeFetch: "fetch"
  of oaePin: "pin"
  of oaeUnpin: "unpin"
  of oaeNoop: "noop"
  of oaeDelete: "delete"

proc errorName(value: ObjectPolicyError): string =
  case value
  of opeNone: "none"
  of opeInvalidManifest: "invalid_manifest"
  of opeManifestDigestMismatch: "manifest_digest_mismatch"
  of opeScopeMismatch: "scope_mismatch"
  of opeLifecycleDenied: "lifecycle_denied"
  of opeCancelled: "cancelled"
  of opeDigestInvalid: "digest_invalid"
  of opeObjectUnknown: "object_unknown"
  of opeLocalFactInvalid: "local_fact_invalid"
  of opeLocalChecksumMismatch: "local_checksum_mismatch"
  of opeMissingOrigin: "missing_origin"
  of opeFetchLimitInvalid: "fetch_limit_invalid"
  of opePinRevisionConflict: "pin_revision_conflict"
  of opePinRevisionOverflow: "pin_revision_overflow"
  of opeGcInvalid: "gc_invalid"

proc kindName(value: ObjectKind): string =
  case value
  of okMedia: "media"
  of okGitPack: "git-pack"
  of okGitManifest: "git-manifest"

proc stringsNode(values: seq[string]): JsonNode =
  result = newJArray()
  for value in values:
    result.add(%value)

proc executeObjectPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "object policy decision is required")
  case payload["decision"].getStr()
  of "sync":
    shape(payload, ["decision", "request"], requestId)
    let plan = planObjectSync(syncRequest(payload["request"], requestId))
    result = %*{
      "decision": "sync",
      "result": {
        "effect": effectName(plan.effect),
        "error": errorName(plan.error),
        "actions": [],
      },
    }
    for action in plan.actions:
      result["result"]["actions"].add(
        %*{
          "digest": action.digest,
          "size": action.size,
          "kind": kindName(action.kind),
          "sourceNodeId": action.sourceNodeId,
          "resumeOffset": action.resumeOffset,
        }
      )
  of "pin":
    shape(payload, ["decision", "state", "request"], requestId)
    let request = payload["request"]
    shape(request, ["communityId", "expectedRevision", "digest", "pin"], requestId)
    let decision = decidePin(
      pinState(payload["state"], requestId),
      PinRequest(
        communityId: text(request, "communityId", requestId),
        expectedRevision: u64(request, "expectedRevision", requestId),
        digest: text(request, "digest", requestId),
        pin: flag(request, "pin", requestId),
      ),
    )
    result = %*{
      "decision": "pin",
      "result": {
        "effect": effectName(decision.effect),
        "error": errorName(decision.error),
        "state": {
          "valid": decision.state.valid,
          "communityId": decision.state.communityId,
          "revision": decision.state.revision,
          "digests": stringsNode(decision.state.digests),
        },
      },
    }
  of "gc":
    shape(payload, ["decision", "request"], requestId)
    let request = payload["request"]
    shape(
      request,
      [
        "communityId", "currentEpoch", "graceEpochs", "referencedDigests",
        "pinnedDigests", "objects", "maxDeletes",
      ],
      requestId,
    )
    var facts = ObjectGcRequest(
      communityId: text(request, "communityId", requestId),
      currentEpoch: u64(request, "currentEpoch", requestId),
      graceEpochs: u64(request, "graceEpochs", requestId),
      referencedDigests: strings(request, "referencedDigests", requestId),
      pinnedDigests: strings(request, "pinnedDigests", requestId),
      maxDeletes: u16(request, "maxDeletes", requestId),
    )
    if request["objects"].kind != JArray:
      invalid(requestId, "objects must be an array")
    for item in request["objects"].items:
      facts.objects.add(localFact(item, requestId))
    let plan = planObjectGc(facts)
    result = %*{
      "decision": "gc",
      "result": {
        "effect": effectName(plan.effect),
        "error": errorName(plan.error),
        "deleteDigests": stringsNode(plan.deleteDigests),
      },
    }
  else:
    invalid(requestId, "unknown object policy decision")
