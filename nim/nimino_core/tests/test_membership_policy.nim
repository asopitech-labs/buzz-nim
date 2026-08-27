import std/[options, unittest]

import nimino_core/domain/membership_policy

suite "Nimino membership policy":
  test "role capability matrix is explicit":
    for (role, expected) in [
      (mrNone, RoleCapabilities()),
      (mrOwner, RoleCapabilities(
        channelActive: true,
        channelElevated: true,
        relayMember: true,
        relayAdmin: true,
        relayOwner: true,
      )),
      (mrAdmin, RoleCapabilities(
        channelActive: true,
        channelElevated: true,
        relayMember: true,
        relayAdmin: true,
      )),
      (mrMember, RoleCapabilities(channelActive: true, relayMember: true)),
      (mrGuest, RoleCapabilities(channelActive: true)),
      (mrBot, RoleCapabilities(channelActive: true)),
    ]:
      check roleCapabilities(role) == expected

  test "channel join and leave protect private access and the last owner":
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcJoin,
      visibility: cvOpen,
      actorIsTarget: true,
    )) == MembershipDecision(
      action: maInsert,
      error: mpeNone,
      effectiveRole: mrMember,
    )
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcJoin,
      visibility: cvPrivate,
      actorIsTarget: true,
    )).error == mpeInviteRequired
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcLeave,
      actorRole: mrOwner,
      targetRole: mrOwner,
      actorIsTarget: true,
      ownerCount: 1,
    )).error == mpeLastOwner
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcJoin,
      visibility: cvOpen,
      actorRole: mrMember,
      targetRole: mrMember,
      actorIsTarget: true,
    )).action == maNoop
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcLeave,
      actorRole: mrMember,
      targetRole: mrMember,
      actorIsTarget: true,
      ownerCount: 1,
    )).action == maRemove
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcLeave,
      targetRole: mrNone,
      actorIsTarget: true,
    )).error == mpeNotMember
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcLeave,
      actorRole: mrMember,
      targetRole: mrOwner,
      actorIsTarget: true,
      ownerCount: 2,
    )).error == mpeRoleInvalid

  test "channel admin and agent-owner decisions fail closed":
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcAdd,
      visibility: cvPrivate,
      actorRole: mrMember,
      requestedRole: mrAdmin,
      targetAddPolicy: apAnyone,
    )).error == mpeNotAuthorized
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcRemove,
      actorRole: mrMember,
      targetRole: mrBot,
      actorOwnsTargetAgent: true,
      targetIsAgent: true,
      ownerCount: 1,
    )).action == maRemove
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcRemove,
      actorRole: mrNone,
      targetRole: mrBot,
      actorOwnsTargetAgent: true,
      targetIsAgent: true,
      ownerCount: 1,
    )).error == mpeNotAuthorized
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcAdd,
      visibility: cvPrivate,
      actorRole: mrMember,
      requestedRole: mrMember,
      targetAddPolicy: apAnyone,
    )).action == maInsert
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcAdd,
      visibility: cvOpen,
      actorRole: mrOwner,
      requestedRole: mrBot,
      targetIsAgent: true,
      targetAddPolicy: apOwnerOnly,
    )).error == mpeAgentAddDenied
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcAdd,
      visibility: cvOpen,
      actorRole: mrMember,
      requestedRole: mrBot,
      actorOwnsTargetAgent: true,
      targetIsAgent: true,
      targetAddPolicy: apOwnerOnly,
    )).action == maInsert
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcChangeRole,
      actorRole: mrAdmin,
      targetRole: mrOwner,
      requestedRole: mrMember,
      ownerCount: 1,
    )).error == mpeLastOwner
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcChangeRole,
      actorRole: mrAdmin,
      targetRole: mrMember,
      requestedRole: mrOwner,
      ownerCount: 1,
    )).action == maUpdateRole
    check decideChannelMembership(ChannelMembershipRequest(
      command: mcRemove,
      actorRole: mrAdmin,
      targetRole: mrOwner,
      ownerCount: 1,
    )).error == mpeLastOwner

  test "relay owner and admin delegation follows the role matrix":
    check decideRelayMembership(RelayMembershipRequest(
      command: mcAdd,
      actorRole: mrAdmin,
      requestedRole: mrMember,
    )).action == maInsert
    check decideRelayMembership(RelayMembershipRequest(
      command: mcAdd,
      actorRole: mrAdmin,
      requestedRole: mrAdmin,
    )).error == mpeNotAuthorized
    check decideRelayMembership(RelayMembershipRequest(
      command: mcChangeRole,
      actorRole: mrOwner,
      targetRole: mrAdmin,
      requestedRole: mrMember,
    )).action == maUpdateRole
    check decideRelayMembership(RelayMembershipRequest(
      command: mcRemove,
      actorRole: mrOwner,
      targetRole: mrOwner,
    )).error == mpeOwnerProtected
    check decideRelayMembership(RelayMembershipRequest(
      command: mcAdd,
      actorRole: mrOwner,
      targetRole: mrAdmin,
      requestedRole: mrMember,
    )) == MembershipDecision(
      action: maNoop,
      error: mpeNone,
      effectiveRole: mrAdmin,
    )
    check decideRelayMembership(RelayMembershipRequest(
      command: mcRemove,
      actorRole: mrAdmin,
      targetRole: mrAdmin,
    )).error == mpeNotAuthorized
    check decideRelayMembership(RelayMembershipRequest(
      command: mcChangeRole,
      actorRole: mrAdmin,
      targetRole: mrMember,
      requestedRole: mrAdmin,
    )).error == mpeNotAuthorized
    check decideRelayMembership(RelayMembershipRequest(
      command: mcChangeRole,
      actorRole: mrOwner,
      targetRole: mrMember,
      requestedRole: mrMember,
    )).action == maNoop
    check decideRelayMembership(RelayMembershipRequest(
      command: mcAdd,
      actorRole: mrOwner,
      targetRole: mrNone,
      requestedRole: mrMember,
      actorIsTarget: true,
    )).error == mpeRoleInvalid

  test "invite expiry and policy acceptance fail closed":
    check decideInvite(InvitePolicyRequest(
      command: icMint,
      actorRole: mrOwner,
      ttlSeconds: 60,
      maxUses: some(1),
    )).action == maMint
    check decideInvite(InvitePolicyRequest(
      command: icClaim,
      inviteState: isExpired,
      policyRequired: true,
      policyAccepted: true,
    )).error == mpeInviteExpired
    check decideInvite(InvitePolicyRequest(
      command: icClaim,
      inviteState: isValid,
      policyRequired: true,
    )).error == mpePolicyRequired
    for request in [
      InvitePolicyRequest(
        command: icMint,
        actorRole: mrOwner,
        ttlSeconds: MinInviteTtlSeconds - 1,
      ),
      InvitePolicyRequest(
        command: icMint,
        actorRole: mrOwner,
        ttlSeconds: MaxInviteTtlSeconds + 1,
      ),
      InvitePolicyRequest(
        command: icMint,
        actorRole: mrAdmin,
        ttlSeconds: 60,
        maxUses: some(0),
      ),
    ]:
      check decideInvite(request).error == mpeMintBounds
    check decideInvite(InvitePolicyRequest(
      command: icMint,
      actorRole: mrMember,
      ttlSeconds: 60,
    )).error == mpeNotAuthorized
    check decideInvite(InvitePolicyRequest(
      command: icClaim,
      inviteState: isExhausted,
      alreadyMember: true,
    )).action == maNoop
    check decideInvite(InvitePolicyRequest(
      command: icClaim,
      inviteState: isExhausted,
    )).error == mpeInviteExhausted
    check decideInvite(InvitePolicyRequest(
      command: icClaim,
      inviteState: isInvalid,
    )).error == mpeInviteInvalid
    check decideInvite(InvitePolicyRequest(
      command: icClaim,
      inviteState: isValid,
    )).action == maJoin

  test "ownership transfer requires an operator and current owner evidence":
    check decideOwnershipTransfer(OwnershipTransferRequest(
      actorIsOperator: true,
      ownerPresent: true,
      expectedOwnerMatches: true,
    )).action == maTransfer
    check decideOwnershipTransfer(OwnershipTransferRequest(
      actorIsOperator: true,
      ownerPresent: true,
    )).error == mpeOwnerConflict
    check decideOwnershipTransfer(OwnershipTransferRequest(
      ownerPresent: true,
      expectedOwnerMatches: true,
    )).error == mpeNotOperator
    check decideOwnershipTransfer(OwnershipTransferRequest(
      actorIsOperator: true,
    )).error == mpeOwnerMissing
    check decideOwnershipTransfer(OwnershipTransferRequest(
      actorIsOperator: true,
      ownerPresent: true,
      expectedOwnerMatches: true,
      newOwnerIsCurrentOwner: true,
      newOwnerAtLimit: true,
    )).action == maNoop
    check decideOwnershipTransfer(OwnershipTransferRequest(
      actorIsOperator: true,
      ownerPresent: true,
      expectedOwnerMatches: true,
      newOwnerAtLimit: true,
    )).error == mpeOwnerLimit
