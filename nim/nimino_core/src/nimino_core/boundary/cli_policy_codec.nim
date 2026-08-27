## Strict JSON adapter for the CLI command and failure policy.

import std/json

import ../domain/cli_policy
import ./protocol

proc invalid(requestId, message: string) {.noreturn.} =
  raiseProtocolError("INVALID_REQUEST", message, requestId, "domain.cli.policy")

proc requireShape(node: JsonNode; keys: openArray[string]; requestId: string) =
  if node.kind != JObject or node.len != keys.len:
    invalid(requestId, "CLI policy payload shape is invalid")
  for key in keys:
    if not node.hasKey(key):
      invalid(requestId, "CLI policy payload is missing " & key)

proc stringAt(node: JsonNode; key, requestId: string): string =
  if node[key].kind != JString:
    invalid(requestId, key & " must be a string")
  node[key].getStr()

proc boolAt(node: JsonNode; key, requestId: string): bool =
  if node[key].kind != JBool:
    invalid(requestId, key & " must be a boolean")
  node[key].getBool()

proc statusAt(node: JsonNode; requestId: string): int =
  if node["status"].kind != JInt:
    invalid(requestId, "status must be an integer")
  result = node["status"].getInt()
  if result < 0 or result > 999:
    invalid(requestId, "status must be between 0 and 999")

proc failureKindAt(node: JsonNode; requestId: string): CliFailureKind =
  case stringAt(node, "kind", requestId)
  of "usage": cfkUsage
  of "relay": cfkRelay
  of "network": cfkNetwork
  of "auth": cfkAuth
  of "key": cfkKey
  of "conflict": cfkConflict
  of "not_found": cfkNotFound
  of "delivery_unknown": cfkDeliveryUnknown
  of "other": cfkOther
  else: invalid(requestId, "kind is not a CLI failure kind")

proc ioName(value: CliIoMode): string =
  case value
  of cimLocal: "local"
  of cimRelayRead: "relay_read"
  of cimRelayWrite: "relay_write"

proc targetName(value: CliPolicyTarget): string =
  case value
  of cptNone: "none"
  of cptEvent: "domain.event.policy"
  of cptCommunity: "domain.community.policy"
  of cptMembership: "domain.membership.policy"
  of cptDm: "domain.dm.policy"
  of cptWorkflow: "domain.workflow.policy"
  of cptModeration: "domain.moderation.policy"

proc errorName(value: CliCommandError): string =
  case value
  of cceNone: "none"
  of cceUnknownCommand: "unknown_command"

proc commandResult(value: CliCommandDecision): JsonNode =
  %*{
    "decision": "command",
    "accepted": value.accepted,
    "error": errorName(value.error),
    "ioMode": ioName(value.ioMode),
    "requiresAuth": value.requiresAuth,
    "outputContract": value.outputContract,
    "policyOperation": targetName(value.policyTarget),
  }

proc failureResult(value: CliFailureDecision): JsonNode =
  %*{
    "decision": "failure",
    "category": value.category,
    "exitCode": value.exitCode,
    "retryable": value.retryable,
  }

proc executeCliPolicy*(payload: JsonNode; requestId: string): JsonNode =
  if payload.kind != JObject or not payload.hasKey("decision") or
      payload["decision"].kind != JString:
    invalid(requestId, "CLI policy decision is required")
  case payload["decision"].getStr()
  of "command":
    requireShape(payload, ["decision", "path"], requestId)
    commandResult(decideCliCommand(stringAt(payload, "path", requestId)))
  of "failure":
    requireShape(
      payload, ["decision", "kind", "status", "transportRetryable"], requestId
    )
    failureResult(
      decideCliFailure(
        failureKindAt(payload, requestId),
        statusAt(payload, requestId),
        boolAt(payload, "transportRetryable", requestId),
      )
    )
  else:
    invalid(requestId, "unknown CLI policy decision")
