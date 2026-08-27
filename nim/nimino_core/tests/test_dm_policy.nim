import std/[options, unittest]

import nimino_core/domain/dm_policy

const
  CommunityA = "018f5e5a-9b7d-7c01-a7bb-46fbe46d0001"
  CommunityB = "018f5e5a-9b7d-7c01-a7bb-46fbe46d0002"

suite "Nimino DM policy":
  test "open creates or restores an immutable participant set":
    check decideDmMutation(DmMutationRequest(
      command: dcOpen,
      requestCommunity: CommunityA,
      actorIncluded: true,
      participantCount: 2,
    )).action == daCreate
    check decideDmMutation(DmMutationRequest(
      command: dcOpen,
      requestCommunity: CommunityA,
      destinationCommunity: some(CommunityA),
      destinationExists: true,
      destinationIsDm: true,
      destinationActorHidden: true,
      actorIncluded: true,
      participantCount: 2,
    )).action == daUnhide
    check decideDmMutation(DmMutationRequest(
      command: dcOpen,
      requestCommunity: CommunityA,
      actorIncluded: true,
      participantCount: 1,
    )).error == dpeParticipantCount

  test "adding a participant creates or reuses a separate expanded DM":
    check decideDmMutation(DmMutationRequest(
      command: dcAdd,
      requestCommunity: CommunityA,
      sourceCommunity: some(CommunityA),
      sourceExists: true,
      sourceIsDm: true,
      actorIsSourceParticipant: true,
      actorIncluded: true,
      participantCount: 3,
      newParticipantCount: 1,
    )).action == daCreate
    check decideDmMutation(DmMutationRequest(
      command: dcAdd,
      requestCommunity: CommunityA,
      sourceCommunity: some(CommunityA),
      sourceExists: true,
      sourceIsDm: true,
      actorIsSourceParticipant: false,
      actorIncluded: true,
      participantCount: 3,
      newParticipantCount: 1,
    )).error == dpeNotParticipant
    check decideDmMutation(DmMutationRequest(
      command: dcAdd,
      requestCommunity: CommunityA,
      sourceCommunity: some(CommunityA),
      sourceExists: true,
      sourceIsDm: true,
      actorIsSourceParticipant: true,
      actorIncluded: true,
      participantCount: 2,
    )).error == dpeNoNewParticipant
    check decideDmMutation(DmMutationRequest(
      command: dcAdd,
      requestCommunity: CommunityA,
      sourceCommunity: some(CommunityB),
      sourceExists: true,
      sourceIsDm: true,
      actorIsSourceParticipant: true,
      actorIncluded: true,
      participantCount: 3,
      newParticipantCount: 1,
    )).error == dpeTenantMismatch

  test "hide is participant-owned and idempotent":
    check decideDmMutation(DmMutationRequest(
      command: dcHide,
      requestCommunity: CommunityA,
      sourceCommunity: some(CommunityA),
      sourceExists: true,
      sourceIsDm: true,
      actorIsSourceParticipant: true,
    )).action == daHide
    check decideDmMutation(DmMutationRequest(
      command: dcHide,
      requestCommunity: CommunityA,
      sourceCommunity: some(CommunityA),
      sourceExists: true,
      sourceIsDm: true,
      actorIsSourceParticipant: true,
      sourceActorHidden: true,
    )).action == daNoop

  test "read and write require exact tenant and active participation":
    for operation in [daoRead, daoWrite]:
      check decideDmAccess(DmAccessRequest(
        operation: operation,
        requestCommunity: CommunityA,
        resourceCommunity: some(CommunityA),
        resourceExists: true,
        channelIsDm: true,
        actorIsParticipant: true,
      )) == DmAccessDecision(allowed: true, error: dpeNone)

    check decideDmAccess(DmAccessRequest(
      operation: daoRead,
      requestCommunity: CommunityA,
      resourceCommunity: some(CommunityB),
      resourceExists: true,
      channelIsDm: true,
      actorIsParticipant: true,
    )).error == dpeTenantMismatch
    check decideDmAccess(DmAccessRequest(
      operation: daoWrite,
      requestCommunity: CommunityA,
      resourceCommunity: some(CommunityA),
      resourceExists: true,
      channelIsDm: true,
    )).error == dpeNotParticipant

  test "visibility snapshots are readable only by their viewer":
    check decideDmAccess(DmAccessRequest(
      operation: daoVisibility,
      requestCommunity: CommunityA,
      resourceCommunity: some(CommunityA),
      resourceExists: true,
      actorIsViewer: true,
    )).allowed
    check decideDmAccess(DmAccessRequest(
      operation: daoVisibility,
      requestCommunity: CommunityA,
      resourceCommunity: some(CommunityA),
      resourceExists: true,
    )).error == dpeViewerMismatch
