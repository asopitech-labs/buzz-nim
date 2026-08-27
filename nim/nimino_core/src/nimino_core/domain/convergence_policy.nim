## Deterministic canonical conflict and anti-resurrection policy.
##
## Sync supplies authenticated, digest-verified facts. This module chooses a
## winner or quarantine result; adapters persist that exact decision.

import std/[algorithm, options]

import nimino_core/domain/event_policy

type
  ConvergenceEffect* = enum
    cmeReject
    cmeInsert
    cmeKeep
    cmeReplace
    cmeDuplicate
    cmeQuarantine
    cmeMerge

  ConvergencePolicyError* = enum
    cpeNone
    cpeIdentityInvalid
    cpeDigestInvalid
    cpeVersionInvalid
    cpeScopeMismatch
    cpeIdentityCollision
    cpeFactConflict
    cpeRetentionInvalid

  IdentityObservation* = object
    communityId*: string
    recordId*: string
    contentDigest*: string

  IdentityState* = object
    communityId*: string
    recordId*: string
    digestBounds*: seq[string]
    quarantined*: bool

  IdentityDecision* = object
    effect*: ConvergenceEffect
    error*: ConvergencePolicyError
    state*: IdentityState

  ReplicaMutationKind* = enum
    rmkLive
    rmkTombstone

  TombstoneKind* = enum
    tskVersioned
    tskPermanent

  ReplicaRecord* = object
    communityId*: string
    logicalKey*: string
    recordId*: string
    logicalTime*: int64
    contentDigest*: string
    kind*: ReplicaMutationKind
    tombstoneKind*: TombstoneKind

  ReplicaMerge* = object
    effect*: ConvergenceEffect
    error*: ConvergencePolicyError
    winner*: Option[ReplicaRecord]
    quarantine*: Option[IdentityState]

  RestrictionState* = enum
    rsReleased
    rsTimedOut
    rsBanned

  RestrictionVersion* = object
    communityId*: string
    subjectId*: string
    revision*: uint64
    state*: RestrictionState
    transitionId*: string
    contentDigest*: string

  RestrictionMerge* = object
    effect*: ConvergenceEffect
    error*: ConvergencePolicyError
    winner*: Option[RestrictionVersion]
    quarantine*: Option[IdentityState]

  RetentionWatermark* = object
    communityId*: string
    scopeId*: string
    prunedThrough*: uint64
    tombstoneProtectedThrough*: uint64

  RetentionMerge* = object
    effect*: ConvergenceEffect
    error*: ConvergencePolicyError
    watermark*: RetentionWatermark

proc isSha256(value: string): bool =
  if value.len != 64:
    return false
  for character in value:
    if character notin {'0' .. '9', 'a' .. 'f'}:
      return false
  true

proc observationError(observation: IdentityObservation): ConvergencePolicyError =
  if observation.communityId.len == 0 or not isSha256(observation.recordId):
    return cpeIdentityInvalid
  if not isSha256(observation.contentDigest):
    return cpeDigestInvalid
  cpeNone

proc validIdentityState(state: IdentityState): bool =
  if observationError(
    IdentityObservation(
      communityId: state.communityId,
      recordId: state.recordId,
      contentDigest: (if state.digestBounds.len > 0: state.digestBounds[0] else: ""),
    )
  ) != cpeNone:
    return false
  if state.digestBounds.len == 1:
    return not state.quarantined
  state.digestBounds.len == 2 and state.quarantined and
    state.digestBounds[0] < state.digestBounds[1] and
    isSha256(state.digestBounds[1])

proc boundedDigests(existing: seq[string]; incoming: string): seq[string] =
  result = existing
  if incoming in result:
    return
  result.add(incoming)
  result.sort()
  # ponytail: two extrema prove an identity collision. Keep full payloads in
  # append-only evidence only if forensic demand justifies the extra storage.
  if result.len > 2:
    result = @[result[0], result[^1]]

proc observeIdentity*(
    current: Option[IdentityState]; observation: IdentityObservation
): IdentityDecision =
  let error = observationError(observation)
  if error != cpeNone:
    return IdentityDecision(effect: cmeReject, error: error)
  if current.isNone:
    return IdentityDecision(
      effect: cmeInsert,
      error: cpeNone,
      state: IdentityState(
        communityId: observation.communityId,
        recordId: observation.recordId,
        digestBounds: @[observation.contentDigest],
      ),
    )

  let existing = current.get()
  if not validIdentityState(existing):
    return IdentityDecision(effect: cmeReject, error: cpeFactConflict)
  if existing.communityId != observation.communityId or
      existing.recordId != observation.recordId:
    return IdentityDecision(effect: cmeReject, error: cpeScopeMismatch, state: existing)
  if observation.contentDigest in existing.digestBounds:
    return IdentityDecision(
      effect: (if existing.quarantined: cmeQuarantine else: cmeDuplicate),
      error: (if existing.quarantined: cpeIdentityCollision else: cpeNone),
      state: existing,
    )

  var quarantined = existing
  quarantined.digestBounds = boundedDigests(
    existing.digestBounds, observation.contentDigest
  )
  quarantined.quarantined = true
  IdentityDecision(
    effect: cmeQuarantine,
    error: cpeIdentityCollision,
    state: quarantined,
  )

proc recordError(record: ReplicaRecord): ConvergencePolicyError =
  if record.communityId.len == 0 or record.logicalKey.len == 0 or
      not isSha256(record.recordId):
    return cpeIdentityInvalid
  if not isSha256(record.contentDigest):
    return cpeDigestInvalid
  if record.logicalTime < 0 or
      (record.kind == rmkLive and record.tombstoneKind != tskVersioned):
    return cpeVersionInvalid
  cpeNone

proc quarantineIdentity(
    communityId, recordId, firstDigest, secondDigest: string
): IdentityState =
  IdentityState(
    communityId: communityId,
    recordId: recordId,
    digestBounds: boundedDigests(@[firstDigest], secondDigest),
    quarantined: true,
  )

proc incomingWins(current, incoming: ReplicaRecord): bool =
  let currentPermanent =
    current.kind == rmkTombstone and current.tombstoneKind == tskPermanent
  let incomingPermanent =
    incoming.kind == rmkTombstone and incoming.tombstoneKind == tskPermanent
  if currentPermanent != incomingPermanent:
    return incomingPermanent
  if current.logicalTime != incoming.logicalTime:
    return incoming.logicalTime > current.logicalTime
  if current.kind != incoming.kind:
    return incoming.kind == rmkTombstone
  decideReplacement(
    EventVersion(createdAt: incoming.logicalTime, eventId: incoming.recordId),
    some(EventVersion(createdAt: current.logicalTime, eventId: current.recordId)),
  ) == rdReplace

proc mergeReplicaRecord*(
    current: Option[ReplicaRecord]; incoming: ReplicaRecord
): ReplicaMerge =
  let incomingError = recordError(incoming)
  if incomingError != cpeNone:
    return ReplicaMerge(effect: cmeReject, error: incomingError, winner: current)
  if current.isNone:
    return ReplicaMerge(effect: cmeInsert, error: cpeNone, winner: some(incoming))
  let existing = current.get()
  let currentError = recordError(existing)
  if currentError != cpeNone:
    return ReplicaMerge(effect: cmeReject, error: cpeFactConflict)
  if existing.communityId != incoming.communityId:
    return ReplicaMerge(
      effect: cmeReject, error: cpeScopeMismatch, winner: current
    )
  if existing.recordId == incoming.recordId:
    if existing.contentDigest != incoming.contentDigest:
      return ReplicaMerge(
        effect: cmeQuarantine,
        error: cpeIdentityCollision,
        quarantine: some(
          quarantineIdentity(
            incoming.communityId,
            incoming.recordId,
            existing.contentDigest,
            incoming.contentDigest,
          )
        ),
      )
    if existing == incoming:
      return ReplicaMerge(effect: cmeDuplicate, error: cpeNone, winner: current)
    return ReplicaMerge(effect: cmeQuarantine, error: cpeFactConflict)
  if existing.logicalKey != incoming.logicalKey:
    return ReplicaMerge(
      effect: cmeReject, error: cpeScopeMismatch, winner: current
    )

  if incomingWins(existing, incoming):
    ReplicaMerge(effect: cmeReplace, error: cpeNone, winner: some(incoming))
  else:
    ReplicaMerge(effect: cmeKeep, error: cpeNone, winner: current)

proc restrictionError(
    restriction: RestrictionVersion
): ConvergencePolicyError =
  if restriction.communityId.len == 0 or restriction.subjectId.len == 0 or
      not isSha256(restriction.transitionId):
    return cpeIdentityInvalid
  if not isSha256(restriction.contentDigest):
    return cpeDigestInvalid
  if restriction.revision == 0:
    return cpeVersionInvalid
  cpeNone

proc mergeRestriction*(
    current: Option[RestrictionVersion]; incoming: RestrictionVersion
): RestrictionMerge =
  let incomingError = restrictionError(incoming)
  if incomingError != cpeNone:
    return RestrictionMerge(
      effect: cmeReject, error: incomingError, winner: current
    )
  if current.isNone:
    return RestrictionMerge(
      effect: cmeInsert, error: cpeNone, winner: some(incoming)
    )
  let existing = current.get()
  if restrictionError(existing) != cpeNone:
    return RestrictionMerge(effect: cmeReject, error: cpeFactConflict)
  if existing.communityId != incoming.communityId or
      existing.subjectId != incoming.subjectId:
    return RestrictionMerge(
      effect: cmeReject, error: cpeScopeMismatch, winner: current
    )
  if existing.transitionId == incoming.transitionId:
    if existing == incoming:
      return RestrictionMerge(
        effect: cmeDuplicate, error: cpeNone, winner: current
      )
    let error = if existing.contentDigest == incoming.contentDigest:
        cpeFactConflict
      else:
        cpeIdentityCollision
    return RestrictionMerge(
      effect: cmeQuarantine,
      error: error,
      quarantine: (if error == cpeIdentityCollision:
          some(
            quarantineIdentity(
              incoming.communityId,
              incoming.transitionId,
              existing.contentDigest,
              incoming.contentDigest,
            )
          )
        else:
          none(IdentityState)),
    )

  let incomingWins = incoming.revision > existing.revision or
    (incoming.revision == existing.revision and incoming.state > existing.state) or
    (incoming.revision == existing.revision and incoming.state == existing.state and
      incoming.transitionId < existing.transitionId)
  if incomingWins:
    RestrictionMerge(effect: cmeReplace, error: cpeNone, winner: some(incoming))
  else:
    RestrictionMerge(effect: cmeKeep, error: cpeNone, winner: current)

proc validWatermark(watermark: RetentionWatermark): bool =
  watermark.communityId.len > 0 and watermark.scopeId.len > 0 and
    watermark.prunedThrough <= watermark.tombstoneProtectedThrough

proc mergeRetention*(
    current, incoming: RetentionWatermark
): RetentionMerge =
  if not validWatermark(current) or not validWatermark(incoming):
    return RetentionMerge(
      effect: cmeReject, error: cpeRetentionInvalid, watermark: current
    )
  if current.communityId != incoming.communityId or
      current.scopeId != incoming.scopeId:
    return RetentionMerge(
      effect: cmeReject, error: cpeScopeMismatch, watermark: current
    )
  let merged = RetentionWatermark(
    communityId: current.communityId,
    scopeId: current.scopeId,
    prunedThrough: max(current.prunedThrough, incoming.prunedThrough),
    tombstoneProtectedThrough: max(
      current.tombstoneProtectedThrough, incoming.tombstoneProtectedThrough
    ),
  )
  RetentionMerge(
    effect: (if merged == current: cmeDuplicate else: cmeMerge),
    error: cpeNone,
    watermark: merged,
  )
