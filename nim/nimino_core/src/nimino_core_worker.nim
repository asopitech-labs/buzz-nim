import std/json

when defined(niminoBoundaryTestHooks):
  import std/os

import nimino_core
import nimino_core/boundary/[
  admission_policy_codec,
  agent_policy_codec,
  cluster_lifecycle_codec,
  control_policy_codec,
  cli_policy_codec,
  community_policy_codec,
  dm_policy_codec,
  effect_policy_codec,
  ephemeral_policy_codec,
  event_policy_codec,
  framing,
  lease_policy_codec,
  membership_policy_codec,
  moderation_policy_codec,
  object_policy_codec,
  projection_policy_codec,
  protocol,
  sync_policy_codec,
  workflow_policy_codec,
]

proc helloResult(): JsonNode =
  result = newJObject()
  result["workerVersion"] = %NiminoCoreVersion
  result["protocolVersion"] = %BoundaryProtocolVersion
  result["maxFrameBytes"] = %BoundaryMaxFrameBytes
  result["maxInflight"] = %BoundaryMaxInflight
  result["schemaHash"] = %BoundarySchemaHash
  result["workerRole"] = %BoundaryWorkerRole
  result["capabilities"] = %*[
    "boundary.echo",
    "domain.event.policy",
    "domain.community.policy",
    "domain.membership.policy",
    "domain.dm.policy",
    "domain.moderation.policy",
    "domain.workflow.policy",
    "domain.cli.policy",
    "domain.agent.policy",
    "domain.cluster.lifecycle",
    "domain.admission.policy",
    "domain.ephemeral.policy",
    "domain.control.policy",
    "domain.lease.policy",
    "domain.effect.policy",
    "domain.object.policy",
    "domain.projection.policy",
    "domain.sync.policy",
  ]

proc execute(request: BoundaryRequest; negotiated: var bool): string =
  if request.operation.kind == boHello:
    let payload = request.operation.hello
    if
      payload.schemaHash != BoundarySchemaHash or
      payload.workerRole != BoundaryWorkerRole or
      payload.maxFrameBytes != BoundaryMaxFrameBytes or
      payload.maxInflight != BoundaryMaxInflight:
      return encodeFailure(
        request.requestId,
        request.operationName,
        "CONTRACT_MISMATCH",
        "host and worker boundary contracts do not match",
      )
    negotiated = true
    return encodeSuccess(request.requestId, request.operationName, helloResult())

  if not negotiated:
    return encodeFailure(
      request.requestId,
      request.operationName,
      "HANDSHAKE_REQUIRED",
      "system.hello must be accepted before another operation",
    )

  when defined(niminoBoundaryTestHooks):
    case request.operation.kind
    of boTestSleep:
      sleep(request.operation.milliseconds)
      return encodeSuccess(
        request.requestId,
        request.operationName,
        %*{"milliseconds": request.operation.milliseconds},
      )
    of boTestCrash:
      quit(70)
    of boTestRemoteFailure:
      return encodeFailure(
        request.requestId,
        request.operationName,
        "UNKNOWN_OPERATION",
        "operation is not supported",
      )
    of boTestGarbage:
      stdout.write("NOPE")
      stdout.flushFile()
      return encodeSuccess(request.requestId, request.operationName, newJObject())
    of boTestMalformed:
      return "{not-json"
    of boTestWrongId:
      return encodeSuccess("wrong-request-id", request.operationName,
          newJObject())
    of boTestPid:
      return encodeSuccess(
        request.requestId,
        request.operationName,
        %*{"pid": getCurrentProcessId()},
      )
    else:
      discard

  case request.operation.kind
  of boEcho:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      %*{"data": request.operation.data},
    )
  of boEventPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeEventPolicy(request.operation.data, request.requestId),
    )
  of boCommunityPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeCommunityPolicy(request.operation.data, request.requestId),
    )
  of boMembershipPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeMembershipPolicy(request.operation.data, request.requestId),
    )
  of boDmPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeDmPolicy(request.operation.data, request.requestId),
    )
  of boModerationPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeModerationPolicy(request.operation.data, request.requestId),
    )
  of boWorkflowPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeWorkflowPolicy(request.operation.data, request.requestId),
    )
  of boCliPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeCliPolicy(request.operation.data, request.requestId),
    )
  of boAgentPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeAgentPolicy(request.operation.data, request.requestId),
    )
  of boClusterLifecycle:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeClusterLifecycle(request.operation.data, request.requestId),
    )
  of boAdmissionPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeAdmissionPolicy(request.operation.data, request.requestId),
    )
  of boEphemeralPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeEphemeralPolicy(request.operation.data, request.requestId),
    )
  of boControlPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeControlPolicy(request.operation.data, request.requestId),
    )
  of boLeasePolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeLeasePolicy(request.operation.data, request.requestId),
    )
  of boEffectPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeEffectPolicy(request.operation.data, request.requestId),
    )
  of boObjectPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeObjectPolicy(request.operation.data, request.requestId),
    )
  of boProjectionPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeProjectionPolicy(request.operation.data, request.requestId),
    )
  of boSyncPolicy:
    result = encodeSuccess(
      request.requestId,
      request.operationName,
      executeSyncPolicy(request.operation.data, request.requestId),
    )
  else:
    result = encodeFailure(
      request.requestId,
      request.operationName,
      "UNKNOWN_OPERATION",
      "operation is not supported",
    )

proc runWorker*() =
  var negotiated = false
  while true:
    var frame: tuple[available: bool; payload: string]
    try:
      frame = readFrame(stdin)
    except BoundaryFrameError as error:
      writeFrame(
        stdout,
        encodeFailure("__frame__", "system.frame", error.code, error.msg),
      )
      break

    if not frame.available:
      break

    var response: string
    var responseRequestId = "__unknown__"
    var responseOperation = "system.decode"
    try:
      let request = decodeRequest(frame.payload)
      responseRequestId = request.requestId
      responseOperation = request.operationName
      try:
        response = execute(request, negotiated)
      except BoundaryProtocolError as error:
        response = encodeFailure(
          request.requestId, request.operationName, error.code, error.msg
        )
      except CatchableError:
        response = encodeFailure(
          request.requestId,
          request.operationName,
          "INTERNAL_ERROR",
          "Nimino core operation failed",
        )
    except BoundaryProtocolError as error:
      response = encodeFailure(
        error.requestId, error.operation, error.code, error.msg
      )

    if response.len > BoundaryMaxFrameBytes:
      response = encodeFailure(
        responseRequestId,
        responseOperation,
        "FRAME_TOO_LARGE",
        "response exceeds the 1 MiB contract limit",
      )
    writeFrame(stdout, response)

when isMainModule:
  runWorker()
