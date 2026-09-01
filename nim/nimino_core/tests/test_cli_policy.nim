import std/[sets, unittest]

import nimino_core/domain/cli_policy

suite "Nimino CLI policy":
  test "the canonical grammar has unique command paths":
    var seen = initHashSet[string]()
    for path in CliCommandPaths:
      check path notin seen
      seen.incl(path)
      check decideCliCommand(path).accepted
    check seen.len == 115

  test "unknown and old command paths fail closed":
    for path in ["", "nimino", "nimino.channels.list", "workflows.retry"]:
      let decision = decideCliCommand(path)
      check not decision.accepted
      check decision.error == cceUnknownCommand

  test "local reads and relay writes have explicit adapter plans":
    let local = decideCliCommand("pack.validate")
    check local.ioMode == cimLocal
    check not local.requiresAuth
    check local.policyTarget == cptNone

    let read = decideCliCommand("messages.search")
    check read.ioMode == cimRelayRead
    check read.requiresAuth
    check read.policyTarget == cptEvent

    let write = decideCliCommand("messages.send")
    check write.ioMode == cimRelayWrite
    check write.requiresAuth
    check write.policyTarget == cptEvent

    check decideCliCommand("dms.open").ioMode == cimRelayWrite
    check decideCliCommand("channels.create").policyTarget == cptEvent
    check decideCliCommand("channels.add-member").policyTarget == cptMembership

  test "workflow commands delegate domain decisions to the Nim core":
    for action in ["list", "get", "create", "update", "trigger", "runs", "approve"]:
      check decideCliCommand("workflows." & action).policyTarget == cptWorkflow
    check decideCliCommand("workflows.delete").policyTarget == cptEvent

  test "exit and retry decisions are stable":
    check decideCliFailure(cfkUsage, 0, false) ==
      CliFailureDecision(category: "user_error", exitCode: 1, retryable: false)
    check decideCliFailure(cfkRelay, 401, true).exitCode == 3
    check decideCliFailure(cfkRelay, 503, false).retryable
    check not decideCliFailure(cfkRelay, 422, true).retryable
    check decideCliFailure(cfkNetwork, 0, true).retryable
    check decideCliFailure(cfkConflict, 0, false).exitCode == 5
    check decideCliFailure(cfkDeliveryUnknown, 0, true) ==
      CliFailureDecision(
        category: "delivery_unknown", exitCode: 2, retryable: false
      )
