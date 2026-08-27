## Content-addressed object fetch, pin, and GC policy owned by Nimino.
##
## Rust streams chunks, verifies SHA-256, and atomically installs bytes. This
## module decides what to fetch, retain, or delete for media and Git objects.

import std/[algorithm, options, sets]

const
  MaxObjectBytes* = 68_719_476_736'u64
  MaxObjectFetches* = 64'u16
  MaxObjectDeletes* = 1_000'u16

type
  ObjectKind* = enum
    okMedia
    okGitPack
    okGitManifest

  ObjectFetchMode* = enum
    ofmEager
    ofmLazy

  ObjectEffect* = enum
    oaeReject
    oaeComplete
    oaeFetch
    oaePin
    oaeUnpin
    oaeNoop
    oaeDelete

  ObjectPolicyError* = enum
    opeNone
    opeInvalidManifest
    opeManifestDigestMismatch
    opeScopeMismatch
    opeLifecycleDenied
    opeCancelled
    opeDigestInvalid
    opeObjectUnknown
    opeLocalFactInvalid
    opeLocalChecksumMismatch
    opeMissingOrigin
    opeFetchLimitInvalid
    opePinRevisionConflict
    opePinRevisionOverflow
    opeGcInvalid

  ObjectDescriptor* = object
    digest*: string
    size*: uint64
    kind*: ObjectKind

  ObjectManifest* = object
    communityId*: string
    manifestId*: string
    generation*: uint64
    objects*: seq[ObjectDescriptor]

  ObjectLocalFact* = object
    digest*: string
    size*: uint64
    present*: bool
    verified*: bool
    partial*: bool
    partialOffset*: uint64
    unreferencedSinceEpoch*: Option[uint64]

  ObjectOriginFact* = object
    nodeId*: string
    available*: bool
    digests*: seq[string]

  ObjectSyncRequest* = object
    communityId*: string
    manifest*: ObjectManifest
    manifestDigestVerified*: bool
    lifecycleAllowsSync*: bool
    cancelled*: bool
    mode*: ObjectFetchMode
    requestedDigest*: string
    pinnedDigests*: seq[string]
    localFacts*: seq[ObjectLocalFact]
    origins*: seq[ObjectOriginFact]
    maxFetches*: uint16

  ObjectFetchAction* = object
    digest*: string
    size*: uint64
    kind*: ObjectKind
    sourceNodeId*: string
    resumeOffset*: uint64

  ObjectSyncPlan* = object
    effect*: ObjectEffect
    error*: ObjectPolicyError
    actions*: seq[ObjectFetchAction]

  PinState* = object
    valid*: bool
    communityId*: string
    revision*: uint64
    digests*: seq[string]

  PinRequest* = object
    communityId*: string
    expectedRevision*: uint64
    digest*: string
    pin*: bool

  PinDecision* = object
    effect*: ObjectEffect
    error*: ObjectPolicyError
    state*: PinState

  ObjectGcRequest* = object
    communityId*: string
    currentEpoch*: uint64
    graceEpochs*: uint64
    ## Complete cross-community reference and pin snapshots. Physical CAS bytes
    ## are shared, so a tenant-local snapshot is never sufficient for deletion.
    referencedDigests*: seq[string]
    pinnedDigests*: seq[string]
    objects*: seq[ObjectLocalFact]
    maxDeletes*: uint16

  ObjectGcPlan* = object
    effect*: ObjectEffect
    error*: ObjectPolicyError
    deleteDigests*: seq[string]

proc isSha256(value: string): bool =
  if value.len != 64:
    return false
  for character in value:
    if character notin {'0' .. '9', 'a' .. 'f'}:
      return false
  true

proc manifestError(manifest: ObjectManifest): ObjectPolicyError =
  if manifest.communityId.len == 0 or not isSha256(manifest.manifestId) or
      manifest.generation == 0 or manifest.objects.len == 0:
    return opeInvalidManifest
  var digests = initHashSet[string]()
  for item in manifest.objects:
    if not isSha256(item.digest):
      return opeDigestInvalid
    if item.size == 0 or item.size > MaxObjectBytes or item.digest in digests:
      return opeInvalidManifest
    digests.incl(item.digest)
  opeNone

proc localFactError(fact: ObjectLocalFact): ObjectPolicyError =
  if not isSha256(fact.digest) or fact.size == 0 or fact.size > MaxObjectBytes or
      fact.partialOffset > fact.size or (fact.verified and not fact.present) or
      (not fact.partial and fact.partialOffset != 0):
    return opeLocalFactInvalid
  opeNone

proc descriptorFor(
    manifest: ObjectManifest; digest: string
): Option[ObjectDescriptor] =
  for item in manifest.objects:
    if item.digest == digest:
      return some(item)
  none(ObjectDescriptor)

proc localFactFor(
    facts: seq[ObjectLocalFact]; digest: string
): Option[ObjectLocalFact] =
  for fact in facts:
    if fact.digest == digest:
      return some(fact)
  none(ObjectLocalFact)

proc normalizedDigests(
    digests: seq[string]
): tuple[valid: bool, values: seq[string]] =
  var seen = initHashSet[string]()
  for digest in digests:
    if not isSha256(digest):
      return
    if digest notin seen:
      seen.incl(digest)
      result.values.add(digest)
  result.values.sort()
  result.valid = true

proc selectOrigin(origins: seq[ObjectOriginFact]; digest: string): Option[string] =
  for origin in origins:
    if origin.available and digest in origin.digests:
      if result.isNone or origin.nodeId < result.get():
        result = some(origin.nodeId)

proc rejectSync(error: ObjectPolicyError): ObjectSyncPlan =
  ObjectSyncPlan(effect: oaeReject, error: error)

proc planObjectSync*(request: ObjectSyncRequest): ObjectSyncPlan =
  let manifestValidation = manifestError(request.manifest)
  if manifestValidation != opeNone:
    return rejectSync(manifestValidation)
  if request.communityId != request.manifest.communityId:
    return rejectSync(opeScopeMismatch)
  if not request.manifestDigestVerified:
    return rejectSync(opeManifestDigestMismatch)
  if not request.lifecycleAllowsSync:
    return rejectSync(opeLifecycleDenied)
  if request.cancelled:
    return rejectSync(opeCancelled)
  if request.maxFetches == 0 or request.maxFetches > MaxObjectFetches:
    return rejectSync(opeFetchLimitInvalid)

  var seenFacts = initHashSet[string]()
  for fact in request.localFacts:
    let error = localFactError(fact)
    if error != opeNone or fact.digest in seenFacts:
      return rejectSync(opeLocalFactInvalid)
    seenFacts.incl(fact.digest)
  var seenOrigins = initHashSet[string]()
  for origin in request.origins:
    let digests = normalizedDigests(origin.digests)
    if origin.nodeId.len == 0 or origin.nodeId in seenOrigins or not digests.valid:
      return rejectSync(opeLocalFactInvalid)
    seenOrigins.incl(origin.nodeId)

  let pins = normalizedDigests(request.pinnedDigests)
  if not pins.valid:
    return rejectSync(opeDigestInvalid)
  var required = pins.values
  case request.mode
  of ofmEager:
    for item in request.manifest.objects:
      if item.digest notin required:
        required.add(item.digest)
  of ofmLazy:
    if request.requestedDigest.len > 0:
      if not isSha256(request.requestedDigest):
        return rejectSync(opeDigestInvalid)
      if request.requestedDigest notin required:
        required.add(request.requestedDigest)
  required.sort()

  for digest in required:
    let descriptor = descriptorFor(request.manifest, digest)
    if descriptor.isNone:
      return rejectSync(opeObjectUnknown)
    let item = descriptor.get()
    let local = localFactFor(request.localFacts, digest)
    if local.isSome and local.get().size != item.size:
      return rejectSync(opeLocalChecksumMismatch)
    if local.isSome and local.get().present:
      if not local.get().verified:
        return rejectSync(opeLocalChecksumMismatch)
      continue
    if result.actions.len >= int(request.maxFetches):
      break
    let source = selectOrigin(request.origins, digest)
    if source.isNone:
      return rejectSync(opeMissingOrigin)
    result.actions.add(
      ObjectFetchAction(
        digest: digest,
        size: item.size,
        kind: item.kind,
        sourceNodeId: source.get(),
        resumeOffset: (if local.isSome and local.get().partial:
            local.get().partialOffset
          else:
            0),
      )
    )
  result.effect = if result.actions.len == 0: oaeComplete else: oaeFetch
  result.error = opeNone

proc validPinState(state: PinState): bool =
  let normalized = normalizedDigests(state.digests)
  state.valid and state.communityId.len > 0 and normalized.valid and
    normalized.values == state.digests

proc initPinState*(communityId: string): PinState =
  PinState(valid: communityId.len > 0, communityId: communityId)

proc decidePin*(state: PinState; request: PinRequest): PinDecision =
  if not validPinState(state):
    return PinDecision(effect: oaeReject, error: opeLocalFactInvalid, state: state)
  if request.communityId != state.communityId:
    return PinDecision(effect: oaeReject, error: opeScopeMismatch, state: state)
  if not isSha256(request.digest):
    return PinDecision(effect: oaeReject, error: opeDigestInvalid, state: state)
  if request.expectedRevision != state.revision:
    return PinDecision(
      effect: oaeReject, error: opePinRevisionConflict, state: state
    )
  let alreadyPinned = request.digest in state.digests
  if alreadyPinned == request.pin:
    return PinDecision(effect: oaeNoop, error: opeNone, state: state)
  if state.revision == high(uint64):
    return PinDecision(
      effect: oaeReject, error: opePinRevisionOverflow, state: state
    )
  var next = state
  next.revision += 1
  if request.pin:
    next.digests.add(request.digest)
    next.digests.sort()
  else:
    next.digests.delete(next.digests.find(request.digest))
  PinDecision(
    effect: (if request.pin: oaePin else: oaeUnpin),
    error: opeNone,
    state: next,
  )

proc planObjectGc*(request: ObjectGcRequest): ObjectGcPlan =
  if request.communityId.len == 0 or request.graceEpochs == 0 or
      request.maxDeletes == 0 or request.maxDeletes > MaxObjectDeletes:
    return ObjectGcPlan(effect: oaeReject, error: opeGcInvalid)
  let references = normalizedDigests(request.referencedDigests)
  let pins = normalizedDigests(request.pinnedDigests)
  if not references.valid or not pins.valid:
    return ObjectGcPlan(effect: oaeReject, error: opeDigestInvalid)
  var seen = initHashSet[string]()
  for item in request.objects:
    if localFactError(item) != opeNone or item.digest in seen:
      return ObjectGcPlan(effect: oaeReject, error: opeLocalFactInvalid)
    seen.incl(item.digest)
    if not item.present or not item.verified or item.partial or
        item.digest in references.values or item.digest in pins.values or
        item.unreferencedSinceEpoch.isNone:
      continue
    let since = item.unreferencedSinceEpoch.get()
    if since <= request.currentEpoch and
        request.currentEpoch - since >= request.graceEpochs:
      result.deleteDigests.add(item.digest)
  result.deleteDigests.sort()
  if result.deleteDigests.len > int(request.maxDeletes):
    result.deleteDigests.setLen(int(request.maxDeletes))
  result.effect = if result.deleteDigests.len == 0: oaeNoop else: oaeDelete
  result.error = opeNone
