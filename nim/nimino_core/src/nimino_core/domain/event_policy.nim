## Pure event acceptance and message mutation policy.
##
## The caller supplies verified event/storage facts. This module owns the
## deterministic decisions; cryptography, authorization, and persistence stay
## behind adapters.

import std/[options, strutils, unicode]

const
  MaxDTagLen* = 1_024
  MaxThreadDepth* = 100
  SupportedKinds* = [
    0, 1, 3, 5, 7, 9, 41, 1059, 1063, 1617, 1618, 1619, 1621, 1630, 1631,
    1632, 1633, 1984, 8000, 8001, 8002, 8003, 9000, 9001, 9002, 9005, 9007,
    9008, 9009, 9021, 9022, 9030, 9031, 9032, 9033, 9035, 9036, 9040, 9041,
    9042, 9043, 9044, 10000, 10001, 10002, 10003, 10030, 10100, 13534,
    13535, 20001, 20002, 22242, 24134, 24200, 24242, 24243, 24810, 27235,
    28936, 30000, 30003, 30023, 30030, 30078, 30174, 30175, 30176, 30177,
    30178, 30179, 30300, 30315, 30617, 30618, 30620, 30621, 30622, 39000,
    39001, 39002, 39003, 39005, 39006, 40002, 40003, 40004, 40005, 40006,
    40007, 40008, 40099, 40100, 40901, 40902, 41001, 41010, 41011, 41012,
    42000, 43001, 43002, 43003, 43004, 43005, 43006, 44100, 44101, 44200,
    45001, 45002, 45003, 46001, 46002, 46003, 46004, 46005, 46006, 46007,
    46010, 46011, 46012, 46020, 46030, 46031, 48001, 48100, 48101, 48102,
    48103, 48106, 49001,
  ]

type
  EventDisposition* = enum
    edRejected
    edStored
    edEphemeral
    edReplaceable
    edParameterized

  EventPolicyError* = enum
    epeNone
    epeUnsupportedKind
    epeAuthNotStorable
    epeDTagRequired
    epeDTagCardinality
    epeDTagTooLong
    epeThreadParentMissing
    epeThreadParentMismatch
    epeThreadChannelMismatch
    epeThreadRootMismatch
    epeThreadDepthExceeded
    epeDeleteTargetCardinality
    epeDeleteTargetMissing
    epeDeleteTargetMismatch
    epeDeleteAuthorMismatch
    epeDeleteCoordinateInvalid
    epeReactionTargetMissing
    epeReactionEmojiInvalid

  EventClassification* = object
    disposition*: EventDisposition
    error*: EventPolicyError

  EventVersion* = object
    createdAt*: int64
    eventId*: string

  ReplacementDecision* = enum
    rdInsert
    rdReplace
    rdDuplicate
    rdStale

  ThreadMetadataFacts* = object
    rootId*: string
    depth*: int

  ThreadParentFacts* = object
    eventId*: string
    createdAt*: int64
    channelId*: string
    tags*: seq[seq[string]]
    metadata*: Option[ThreadMetadataFacts]

  ThreadRequest* = object
    eventId*: string
    createdAt*: int64
    channelId*: string
    tags*: seq[seq[string]]
    parent*: Option[ThreadParentFacts]
    rootCreatedAt*: Option[int64]

  ThreadPlan* = object
    rootId*: string
    parentId*: string
    rootCreatedAt*: int64
    parentCreatedAt*: int64
    depth*: int
    broadcast*: bool
    parentReplyDelta*: int
    rootDescendantDelta*: int

  ThreadDecision* = object
    error*: EventPolicyError
    plan*: ThreadPlan

  DeletionTargetFacts* = object
    eventId*: string
    author*: string
    createdAt*: int64
    active*: bool
    parentId*: Option[string]
    rootId*: Option[string]

  DeletionRequest* = object
    actor*: string
    createdAt*: int64
    eTargets*: seq[string]
    aTargets*: seq[string]
    target*: Option[DeletionTargetFacts]

  DeletionAction* = enum
    daReject
    daNoop
    daDeleteEvent
    daDeleteCoordinate
    daKeepNewer

  DeletionDecision* = object
    error*: EventPolicyError
    action*: DeletionAction
    parentReplyDelta*: int
    rootDescendantDelta*: int

  ReactionRequest* = object
    targetExists*: bool
    activeDuplicate*: bool
    content*: string
    tags*: seq[seq[string]]

  ReactionAction* = enum
    raReject
    raDuplicate
    raInsert

  ReactionDecision* = object
    error*: EventPolicyError
    action*: ReactionAction
    emoji*: string

proc isSupported(kind: int): bool =
  # ponytail: a 132-entry scan is below IPC cost; sort/search only if profiling says otherwise.
  for candidate in SupportedKinds:
    if candidate == kind:
      return true

proc classifyEvent*(kind, dTagCount, dTagLen: int): EventClassification =
  if not isSupported(kind):
    return EventClassification(error: epeUnsupportedKind)
  if kind == 22242:
    return EventClassification(error: epeAuthNotStorable)
  if kind >= 30000 and kind <= 39999:
    if dTagCount == 0:
      return EventClassification(error: epeDTagRequired)
    if dTagCount != 1:
      return EventClassification(error: epeDTagCardinality)
    if dTagLen > MaxDTagLen:
      return EventClassification(error: epeDTagTooLong)
    return EventClassification(disposition: edParameterized, error: epeNone)
  if kind >= 20000 and kind <= 29999:
    return EventClassification(disposition: edEphemeral, error: epeNone)
  if kind == 0 or kind == 3 or kind == 41 or
      (kind >= 10000 and kind <= 19999):
    return EventClassification(disposition: edReplaceable, error: epeNone)
  EventClassification(disposition: edStored, error: epeNone)

proc decideReplacement*(
    incoming: EventVersion, current: Option[EventVersion]
): ReplacementDecision =
  if current.isNone:
    return rdInsert
  let existing = current.get()
  let incomingId = incoming.eventId.toLowerAscii()
  let existingId = existing.eventId.toLowerAscii()
  if incoming.createdAt > existing.createdAt or
      (incoming.createdAt == existing.createdAt and incomingId < existingId):
    rdReplace
  elif incoming.createdAt == existing.createdAt and incomingId == existingId:
    rdDuplicate
  else:
    rdStale

proc isEventId(value: string): bool =
  if value.len != 64:
    return false
  for ch in value:
    if ch notin {'0' .. '9', 'a' .. 'f', 'A' .. 'F'}:
      return false
  true

proc markers(tags: seq[seq[string]]): tuple[root, reply: Option[string]] =
  for tag in tags:
    if tag.len >= 4 and tag[0] == "e" and isEventId(tag[1]):
      if tag[3] == "root":
        result.root = some(tag[1].toLowerAscii())
      elif tag[3] == "reply":
        result.reply = some(tag[1].toLowerAscii())

proc decideThread*(request: ThreadRequest): ThreadDecision =
  let ownMarkers = markers(request.tags)
  if ownMarkers.reply.isNone:
    return ThreadDecision(error: epeNone)
  if request.parent.isNone:
    return ThreadDecision(error: epeThreadParentMissing)

  let parent = request.parent.get()
  let parentId = ownMarkers.reply.get()
  if parent.eventId.toLowerAscii() != parentId:
    return ThreadDecision(error: epeThreadParentMismatch)
  if parent.channelId != request.channelId:
    return ThreadDecision(error: epeThreadChannelMismatch)

  var rootId: string
  var depth: int
  if parent.metadata.isSome:
    let metadata = parent.metadata.get()
    rootId = metadata.rootId.toLowerAscii()
    depth = metadata.depth + 1
  else:
    let parentMarkers = markers(parent.tags)
    if parentMarkers.reply.isSome:
      rootId = if parentMarkers.root.isSome: parentMarkers.root.get() else: parentMarkers.reply.get()
    else:
      rootId = parent.eventId.toLowerAscii()
    depth = if rootId == parent.eventId.toLowerAscii(): 1 else: 2

  let clientRoot = if ownMarkers.root.isSome: ownMarkers.root.get() else: parentId
  if clientRoot != rootId:
    return ThreadDecision(error: epeThreadRootMismatch)
  if depth > MaxThreadDepth:
    return ThreadDecision(error: epeThreadDepthExceeded)

  var broadcast = false
  for tag in request.tags:
    if tag.len >= 2 and tag[0] == "broadcast" and tag[1] == "1":
      broadcast = true
      break

  ThreadDecision(
    error: epeNone,
    plan: ThreadPlan(
      rootId: rootId,
      parentId: parentId,
      rootCreatedAt: request.rootCreatedAt.get(parent.createdAt),
      parentCreatedAt: parent.createdAt,
      depth: depth,
      broadcast: broadcast,
      parentReplyDelta: 1,
      rootDescendantDelta: 1,
    ),
  )

proc validCoordinate(value, actor: string): bool =
  let first = value.find(':')
  let second = if first >= 0: value.find(':', first + 1) else: -1
  if first != 5 or second <= first + 1 or value.len - second - 1 > MaxDTagLen:
    return false
  var kind = 0
  for ch in value[0 ..< first]:
    if ch notin {'0' .. '9'}:
      return false
    kind = kind * 10 + ord(ch) - ord('0')
  kind >= 30000 and kind <= 39999 and
    value[first + 1 ..< second].toLowerAscii() == actor.toLowerAscii()

proc decideDeletion*(request: DeletionRequest): DeletionDecision =
  if request.eTargets.len + request.aTargets.len != 1:
    return DeletionDecision(error: epeDeleteTargetCardinality)
  if request.aTargets.len == 1:
    if not validCoordinate(request.aTargets[0], request.actor):
      return DeletionDecision(error: epeDeleteCoordinateInvalid)
    return DeletionDecision(error: epeNone, action: daDeleteCoordinate)
  if not isEventId(request.eTargets[0]) or request.target.isNone:
    return DeletionDecision(error: epeDeleteTargetMissing)

  let target = request.target.get()
  if target.eventId.toLowerAscii() != request.eTargets[0].toLowerAscii():
    return DeletionDecision(error: epeDeleteTargetMismatch)
  if target.author != request.actor:
    return DeletionDecision(error: epeDeleteAuthorMismatch)
  if not target.active:
    return DeletionDecision(error: epeNone, action: daNoop)
  if target.createdAt > request.createdAt:
    return DeletionDecision(error: epeNone, action: daKeepNewer)

  DeletionDecision(
    error: epeNone,
    action: daDeleteEvent,
    parentReplyDelta: (if target.parentId.isSome: -1 else: 0),
    rootDescendantDelta: (if target.rootId.isSome: -1 else: 0),
  )

proc isCanonicalShortcode(value: string): bool =
  if value.len == 0 or value.len > 64:
    return false
  for ch in value:
    if ch notin {'a' .. 'z', '0' .. '9', '_', '-'}:
      return false
  true

proc decideReaction*(request: ReactionRequest): ReactionDecision =
  if not request.targetExists:
    return ReactionDecision(error: epeReactionTargetMissing)
  if request.activeDuplicate:
    return ReactionDecision(error: epeNone, action: raDuplicate)

  let emoji = if request.content.len == 0: "+" else: request.content
  if emoji.runeLen > 64:
    if emoji.len < 3 or emoji[0] != ':' or emoji[^1] != ':':
      return ReactionDecision(error: epeReactionEmojiInvalid)
    let shortcode = emoji[1 .. ^2]
    if not isCanonicalShortcode(shortcode) or emoji.runeLen > 66:
      return ReactionDecision(error: epeReactionEmojiInvalid)
    var matched = false
    for tag in request.tags:
      if tag.len >= 2 and tag[0] == "emoji" and tag[1] == shortcode:
        matched = true
        break
    if not matched:
      return ReactionDecision(error: epeReactionEmojiInvalid)

  ReactionDecision(error: epeNone, action: raInsert, emoji: emoji)
