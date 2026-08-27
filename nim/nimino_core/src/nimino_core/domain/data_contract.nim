## Pure, versioned storage contract owned by Nimino.
##
## Domain services produce canonical writes. Storage adapters commit or query
## these contract entities. Cache rows are replaceable projections; log rows
## are append-only evidence and never business truth.

import std/[json, options, sets, strutils]

const
  DataProtocolName* = "nimino.data"
  DataProtocolVersion* = 1
  MaxQueryLimit* = 1_000'u16

  CanonicalRecordTypes* = [
    "community",
    "channel",
    "membership",
    "profile",
    "event",
    "subscription",
    "workflow",
    "workflow_run",
    "workflow_approval",
    "scheduled_workflow_fire",
    "api_token",
    "allowlist_entry",
    "relay_member",
    "join_policy_acceptance",
    "relay_invite",
    "identity_archive",
    "moderation_case",
    "restriction",
    "repository_name",
    "retention_watermark",
    "product_feedback",
    "deletion_job",
    "deletion_approval",
    "deletion_checkpoint",
    "deletion_manifest_key",
  ]
  CacheRecordTypes* = [
    "mention_index",
    "thread_index",
    "reaction_index",
    "search_index",
    "feed_index",
    "replica_heartbeat",
    "serving_write_lease",
    "executor_heartbeat",
  ]
  LogRecordTypes* = [
    "delivery_attempt",
    "rate_limit_violation",
    "audit_entry",
    "moderation_action",
    "workflow_execution_entry",
    "taxonomy_sweep",
  ]

type
  DataClass* = enum
    dcCanonical
    dcCache
    dcLog

  WriteIntentKind* = enum
    wiCanonicalCommit
    wiCacheReplace
    wiLogAppend

  QueryIntentKind* = enum
    qiByKey
    qiPage
    qiChanges

  ProjectionKind* = enum
    pkSearch
    pkThread
    pkFeed

  DataContractError* = enum
    dceNone
    dceIntentRequired
    dceCommunityRequired
    dceKeyRequired
    dceValueRequired
    dceWritesRequired
    dceUnknownRecordType
    dceClassMismatch
    dceCheckpointRequired
    dceCheckpointForbidden
    dceDeleteForbidden
    dceLimitInvalid
    dceQueryShape
    dceSequenceOrder
    dceDigestInvalid
    dceProjectionMismatch
    dceDuplicateKey

  RecordWrite* = object
    recordType*: string
    key*: string
    deleted*: bool
    value*: JsonNode

  WriteIntent* = object
    kind*: WriteIntentKind
    intentId*: string
    communityId*: string
    ## Optimistic canonical revision or cache source checkpoint. Log appends
    ## forbid it because logs do not replace prior state.
    checkpoint*: Option[uint64]
    writes*: seq[RecordWrite]

  QueryIntent* = object
    kind*: QueryIntentKind
    communityId*: string
    recordType*: string
    key*: string
    after*: Option[uint64]
    limit*: uint16

  CanonicalRecord* = object
    sequence*: uint64
    recordType*: string
    key*: string
    deleted*: bool
    value*: JsonNode

  ProjectionRebuildInput* = object
    projection*: ProjectionKind
    communityId*: string
    throughSequence*: uint64
    sourceDigest*: string
    records*: seq[CanonicalRecord]

  ProjectionRow* = object
    recordType*: string
    key*: string
    value*: JsonNode

  ProjectionRebuildOutput* = object
    projection*: ProjectionKind
    communityId*: string
    throughSequence*: uint64
    sourceDigest*: string
    rows*: seq[ProjectionRow]

proc classifyRecord*(recordType: string): Option[DataClass] =
  if recordType in CanonicalRecordTypes:
    some(dcCanonical)
  elif recordType in CacheRecordTypes:
    some(dcCache)
  elif recordType in LogRecordTypes:
    some(dcLog)
  else:
    none(DataClass)

proc projectionRecordType*(projection: ProjectionKind): string =
  case projection
  of pkSearch: "search_index"
  of pkThread: "thread_index"
  of pkFeed: "feed_index"

proc validateRecord(recordType, key: string, value: JsonNode): DataContractError =
  if key.len == 0:
    return dceKeyRequired
  if value.isNil:
    return dceValueRequired
  if classifyRecord(recordType).isNone:
    return dceUnknownRecordType
  dceNone

proc validate*(intent: WriteIntent): DataContractError =
  if intent.intentId.len == 0:
    return dceIntentRequired
  if intent.communityId.len == 0:
    return dceCommunityRequired

  let expectedClass = case intent.kind
    of wiCanonicalCommit: dcCanonical
    of wiCacheReplace: dcCache
    of wiLogAppend: dcLog

  case intent.kind
  of wiCanonicalCommit:
    if intent.checkpoint.isNone:
      return dceCheckpointRequired
    if intent.writes.len == 0:
      return dceWritesRequired
  of wiCacheReplace:
    if intent.checkpoint.isNone:
      return dceCheckpointRequired
  of wiLogAppend:
    if intent.checkpoint.isSome:
      return dceCheckpointForbidden
    if intent.writes.len == 0:
      return dceWritesRequired

  var keys = initHashSet[string]()
  for write in intent.writes:
    let error = validateRecord(write.recordType, write.key, write.value)
    if error != dceNone:
      return error
    if classifyRecord(write.recordType).get() != expectedClass:
      return dceClassMismatch
    if write.deleted and intent.kind != wiCanonicalCommit:
      return dceDeleteForbidden
    let typedKey = write.recordType & "\x1f" & write.key
    if typedKey in keys:
      return dceDuplicateKey
    keys.incl(typedKey)
  dceNone

proc validate*(intent: QueryIntent): DataContractError =
  if intent.communityId.len == 0:
    return dceCommunityRequired
  if intent.limit == 0 or intent.limit > MaxQueryLimit:
    return dceLimitInvalid

  case intent.kind
  of qiByKey:
    if classifyRecord(intent.recordType).isNone:
      return dceUnknownRecordType
    if intent.key.len == 0 or intent.limit != 1 or intent.after.isSome:
      return dceQueryShape
  of qiPage:
    if classifyRecord(intent.recordType).isNone:
      return dceUnknownRecordType
    if intent.key.len > 0:
      return dceQueryShape
  of qiChanges:
    if intent.recordType.len > 0 or intent.key.len > 0:
      return dceQueryShape
  dceNone

proc isSha256(value: string): bool =
  value.len == 64 and value.allCharsInSet({'0' .. '9', 'a' .. 'f'})

proc validate*(input: ProjectionRebuildInput): DataContractError =
  if input.communityId.len == 0:
    return dceCommunityRequired
  if not isSha256(input.sourceDigest):
    return dceDigestInvalid

  var previous = 0'u64
  var first = true
  for record in input.records:
    let error = validateRecord(record.recordType, record.key, record.value)
    if error != dceNone:
      return error
    if classifyRecord(record.recordType).get() != dcCanonical:
      return dceClassMismatch
    if record.sequence > input.throughSequence or
        (not first and record.sequence <= previous):
      return dceSequenceOrder
    previous = record.sequence
    first = false
  dceNone

proc validate*(
    input: ProjectionRebuildInput, output: ProjectionRebuildOutput
): DataContractError =
  let inputError = validate(input)
  if inputError != dceNone:
    return inputError
  if output.projection != input.projection or
      output.communityId != input.communityId or
      output.throughSequence != input.throughSequence or
      output.sourceDigest != input.sourceDigest:
    return dceProjectionMismatch

  let expectedType = projectionRecordType(input.projection)
  var keys = initHashSet[string]()
  for row in output.rows:
    let error = validateRecord(row.recordType, row.key, row.value)
    if error != dceNone:
      return error
    if row.recordType != expectedType:
      return dceProjectionMismatch
    if row.key in keys:
      return dceDuplicateKey
    keys.incl(row.key)
  dceNone
