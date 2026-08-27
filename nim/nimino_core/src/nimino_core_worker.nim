import std/json

when defined(niminoBoundaryTestHooks):
  import std/os

import nimino_core
import nimino_core/boundary/[
  community_policy_codec,
  dm_policy_codec,
  event_policy_codec,
  framing,
  membership_policy_codec,
  protocol,
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
