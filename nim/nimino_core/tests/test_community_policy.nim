import std/[options, unittest]

import nimino_core/domain/community_policy

suite "Nimino community lifecycle and tenant policy":
  test "create requires an operator, owner, free host, and owner capacity":
    let accepted = decideLifecycle(CommunityLifecycleRequest(
      command: ccCreate,
      state: csMissing,
      actorIsOperator: true,
      ownerProvided: true,
    ))
    check accepted == CommunityLifecycleDecision(action: caCreate, error: cpeNone)

    check decideLifecycle(CommunityLifecycleRequest(
      command: ccCreate,
      state: csMissing,
      ownerProvided: true,
    )).error == cpeNotOperator
    check decideLifecycle(CommunityLifecycleRequest(
      command: ccCreate,
      state: csMissing,
      actorIsOperator: true,
    )).error == cpeOwnerRequired
    check decideLifecycle(CommunityLifecycleRequest(
      command: ccCreate,
      state: csMissing,
      actorIsOperator: true,
      ownerProvided: true,
      ownerAtLimit: true,
    )).error == cpeOwnerLimit
    check decideLifecycle(CommunityLifecycleRequest(
      command: ccCreate,
      state: csArchived,
      actorIsOperator: true,
      ownerProvided: true,
      ownerAtLimit: true,
    )).error == cpeHostReserved

  test "archive and unarchive are owner-authorized idempotent transitions":
    for (command, state, action, error) in [
      (ccArchive, csActive, caArchive, cpeNone),
      (ccArchive, csArchived, caNoop, cpeNone),
      (ccArchive, csMissing, caReject, cpeInvalidState),
      (ccArchive, csDeleting, caReject, cpeInvalidState),
      (ccArchive, csDeleted, caReject, cpeInvalidState),
      (ccUnarchive, csActive, caNoop, cpeNone),
      (ccUnarchive, csArchived, caUnarchive, cpeNone),
      (ccUnarchive, csMissing, caReject, cpeInvalidState),
      (ccUnarchive, csDeleting, caReject, cpeInvalidState),
      (ccUnarchive, csDeleted, caReject, cpeInvalidState),
    ]:
      check decideLifecycle(CommunityLifecycleRequest(
        command: command,
        state: state,
        actorIsOperator: true,
        actorIsOwner: true,
      )) == CommunityLifecycleDecision(action: action, error: error)

    check decideLifecycle(CommunityLifecycleRequest(
      command: ccArchive,
      state: csActive,
      actorIsOperator: true,
    )).error == cpeNotOwner

  test "protected and deleting communities fail closed":
    check decideLifecycle(CommunityLifecycleRequest(
      command: ccArchive,
      state: csActive,
      actorIsOperator: true,
      actorIsOwner: true,
      protectedDeployment: true,
    )).error == cpeProtectedCommunity
    check decideLifecycle(CommunityLifecycleRequest(
      command: ccUnarchive,
      state: csDeleting,
      actorIsOperator: true,
      actorIsOwner: true,
    )).error == cpeInvalidState

  test "resource access requires an exact host-derived tenant match":
    check decideScope(CommunityScopeRequest(
      requestCommunity: "018f5e5a-9b7d-7c01-a7bb-46fbe46d0001",
      resourceCommunity: some("018f5e5a-9b7d-7c01-a7bb-46fbe46d0001"),
    )) == CommunityScopeDecision(allowed: true, error: cpeNone)

    check decideScope(CommunityScopeRequest(
      requestCommunity: "018f5e5a-9b7d-7c01-a7bb-46fbe46d0001",
      resourceCommunity: some("018f5e5a-9b7d-7c01-a7bb-46fbe46d0002"),
    )).error == cpeTenantMismatch
    check decideScope(CommunityScopeRequest(
      requestCommunity: "018f5e5a-9b7d-7c01-a7bb-46fbe46d0001",
      resourceCommunity: none(string),
    )).error == cpeResourceMissing
