import std/[json, parsejson, sets, streams, unicode]

const
  BoundaryProtocolName* = "nimino.core.boundary"
  BoundaryProtocolVersion* = 1
  BoundaryMaxFrameBytes* = 1_048_576
  BoundaryMaxInflight* = 1
  BoundaryWorkerRole* = "nimino-core"

when defined(niminoBoundaryWrongSchema):
  const BoundarySchemaHash* = "0000000000000000000000000000000000000000000000000000000000000000"
else:
  const BoundarySchemaHash* = "8b2ddbbb0fd4b20aa227511187428c03d2334f0a0b2f666c98a3a1db17172182"

type
  BoundaryOperationKind* = enum
    boHello,
    boEcho,
    boEventPolicy,
    boCommunityPolicy,
    boTestSleep,
    boTestCrash,
    boTestRemoteFailure,
    boTestGarbage,
    boTestMalformed,
    boTestWrongId,
    boTestPid,
    boUnknown

  BoundaryHelloPayload* = object
    schemaHash*: string
    workerRole*: string
    maxFrameBytes*: int
    maxInflight*: int

  BoundaryOperation* = object
    kind*: BoundaryOperationKind
    data*: JsonNode
    hello*: BoundaryHelloPayload
    milliseconds*: int

  BoundaryRequest* = object
    protocol*: string
    version*: int
    requestId*: string
    operationName*: string
    operation*: BoundaryOperation

  BoundaryProtocolError* = object of CatchableError
    code*: string
    requestId*: string
    operation*: string

proc raiseProtocolError*(
    code, message: string;
    requestId = "__unknown__";
    operation = "system.decode"
) =
  var error = newException(BoundaryProtocolError, message)
  error.code = code
  error.requestId = requestId
  error.operation = operation
  raise error

proc consumeStrictJsonValue(parser: var JsonParser) =
  case parser.kind
  of jsonString, jsonInt, jsonFloat, jsonTrue, jsonFalse, jsonNull:
    parser.next()
  of jsonArrayStart:
    parser.next()
    while parser.kind != jsonArrayEnd:
      if parser.kind in {jsonError, jsonEof}:
        raiseProtocolError("INVALID_REQUEST", "array value is malformed")
      consumeStrictJsonValue(parser)
    parser.next()
  of jsonObjectStart:
    var keys = initHashSet[string]()
    parser.next()
    while parser.kind != jsonObjectEnd:
      if parser.kind != jsonString:
        raiseProtocolError("INVALID_REQUEST", "object key is malformed")
      let key = parser.str()
      if key in keys:
        raiseProtocolError("INVALID_REQUEST", "duplicate JSON field: " & key)
      keys.incl(key)
      parser.next()
      consumeStrictJsonValue(parser)
    parser.next()
  of jsonError:
    raiseProtocolError("INVALID_REQUEST", parser.errorMsg())
  of jsonEof, jsonObjectEnd, jsonArrayEnd:
    raiseProtocolError("INVALID_REQUEST", "JSON value is incomplete")

proc rejectDuplicateKeys(raw: string) =
  var parser: JsonParser
  parser.open(newStringStream(raw), "boundary request")
  defer:
    parser.close()
  parser.next()
  consumeStrictJsonValue(parser)
  if parser.kind != jsonEof:
    raiseProtocolError("INVALID_REQUEST", "request contains trailing JSON")

proc requireString(node: JsonNode; key, requestId: string): string =
  if not node.hasKey(key) or node[key].kind != JString:
    raiseProtocolError("INVALID_REQUEST", key & " must be a string", requestId)
  result = node[key].getStr()

proc requireInt(node: JsonNode; key, requestId, operation: string): int =
  if not node.hasKey(key) or node[key].kind != JInt:
    raiseProtocolError(
      "INVALID_REQUEST", key & " must be an integer", requestId, operation
    )
  result = node[key].getInt()

proc decodeOperation(
    name: string; payload: JsonNode; requestId: string
): BoundaryOperation =
  case name
  of "system.hello":
    if payload.len != 4:
      raiseProtocolError(
        "INVALID_REQUEST", "system.hello payload shape is invalid", requestId, name
      )
    result.kind = boHello
    result.hello.schemaHash = requireString(payload, "schemaHash", requestId)
    result.hello.workerRole = requireString(payload, "workerRole", requestId)
    result.hello.maxFrameBytes = requireInt(payload, "maxFrameBytes", requestId, name)
    result.hello.maxInflight = requireInt(payload, "maxInflight", requestId, name)
  of "boundary.echo":
    if payload.len != 1 or not payload.hasKey("data"):
      raiseProtocolError(
        "INVALID_REQUEST",
        "boundary.echo payload must contain only data",
        requestId,
        name,
      )
    result.kind = boEcho
    result.data = payload["data"]
  of "domain.event.policy":
    result.kind = boEventPolicy
    result.data = payload
  of "domain.community.policy":
    result.kind = boCommunityPolicy
    result.data = payload
  of "boundary.test.sleep":
    if payload.len != 1:
      raiseProtocolError(
        "INVALID_REQUEST", "test sleep payload shape is invalid", requestId, name
      )
    result.kind = boTestSleep
    result.milliseconds = requireInt(payload, "milliseconds", requestId, name)
    if result.milliseconds < 0 or result.milliseconds > 60_000:
      raiseProtocolError(
        "INVALID_REQUEST", "milliseconds is out of range", requestId, name
      )
  of "boundary.test.crash":
    result.kind = boTestCrash
  of "boundary.test.remote_failure":
    result.kind = boTestRemoteFailure
  of "boundary.test.garbage":
    result.kind = boTestGarbage
  of "boundary.test.malformed":
    result.kind = boTestMalformed
  of "boundary.test.wrong_id":
    result.kind = boTestWrongId
  of "boundary.test.pid":
    result.kind = boTestPid
  else:
    result.kind = boUnknown
    result.data = payload

  if result.kind in {
    boTestCrash,
    boTestRemoteFailure,
    boTestGarbage,
    boTestMalformed,
    boTestWrongId,
    boTestPid,
  } and payload.len != 0:
    raiseProtocolError(
      "INVALID_REQUEST", "test operation payload must be empty", requestId, name
    )

proc decodeRequest*(raw: string): BoundaryRequest =
  rejectDuplicateKeys(raw)
  var node: JsonNode
  try:
    node = parseJson(raw)
  except JsonParsingError as error:
    raiseProtocolError("INVALID_REQUEST", "request is not valid JSON: " & error.msg)

  if node.kind != JObject:
    raiseProtocolError("INVALID_REQUEST", "request must be a JSON object")

  var requestId = "__unknown__"
  if node.hasKey("requestId") and node["requestId"].kind == JString:
    requestId = node["requestId"].getStr()

  const allowedKeys = ["protocol", "version", "requestId", "operation", "payload"]
  for key in node.keys:
    if key notin allowedKeys:
      raiseProtocolError("INVALID_REQUEST", "unknown request field: " & key, requestId)

  result.protocol = requireString(node, "protocol", requestId)
  result.requestId = requireString(node, "requestId", requestId)
  result.operationName = requireString(node, "operation", requestId)

  if not node.hasKey("version") or node["version"].kind != JInt:
    raiseProtocolError("INVALID_REQUEST", "version must be an integer", requestId)
  result.version = node["version"].getInt()

  if not node.hasKey("payload") or node["payload"].kind != JObject:
    raiseProtocolError("INVALID_REQUEST", "payload must be an object", requestId)
  let payload = node["payload"]

  if result.protocol != BoundaryProtocolName:
    raiseProtocolError("INVALID_REQUEST", "protocol name is not supported", requestId)
  if result.version != BoundaryProtocolVersion:
    raiseProtocolError("UNSUPPORTED_VERSION", "protocol version is not supported", requestId)
  if result.requestId.runeLen == 0 or result.requestId.runeLen > 128:
    raiseProtocolError("INVALID_REQUEST", "requestId length is invalid", requestId)
  if result.operationName.runeLen == 0 or result.operationName.runeLen > 128:
    raiseProtocolError("INVALID_REQUEST", "operation length is invalid", requestId)
  result.operation = decodeOperation(result.operationName, payload, requestId)

proc baseResponse(requestId, operation, status: string): JsonNode =
  result = newJObject()
  result["protocol"] = %BoundaryProtocolName
  result["version"] = %BoundaryProtocolVersion
  result["requestId"] = %requestId
  result["operation"] = %operation
  result["status"] = %status

proc encodeSuccess*(requestId, operation: string; value: JsonNode): string =
  var response = baseResponse(requestId, operation, "ok")
  response["result"] = value
  result = $response

proc retryForCode(code: string): string =
  case code
  of "HANDSHAKE_REQUIRED":
    result = "after_refresh"
  of "INTERNAL_ERROR":
    result = "idempotent_only"
  of
    "INVALID_REQUEST",
    "UNSUPPORTED_VERSION",
    "CONTRACT_MISMATCH",
    "UNKNOWN_OPERATION",
    "FRAME_TOO_LARGE":
    result = "never"
  else:
    raise newException(ValueError, "unknown boundary fault code: " & code)

proc validEnvelopeField(value, fallback: string): string =
  let length = value.runeLen
  if length >= 1 and length <= 128:
    value
  else:
    fallback

proc encodeFailure*(requestId, operation, code, message: string): string =
  var response = baseResponse(
    validEnvelopeField(requestId, "__unknown__"),
    validEnvelopeField(operation, "system.decode"),
    "error",
  )
  var fault = newJObject()
  fault["code"] = %code
  fault["message"] = %message.runeSubStr(0, 1_024)
  fault["retry"] = %retryForCode(code)
  response["error"] = fault
  result = $response
