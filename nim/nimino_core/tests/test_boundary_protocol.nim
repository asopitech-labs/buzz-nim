import std/[json, strutils, unicode, unittest]
import nimino_core/boundary/[community_policy_codec, event_policy_codec, protocol]

suite "Nim/Rust boundary protocol v1":
  test "decodes the canonical request envelope":
    let request = decodeRequest("""{
      "protocol":"nimino.core.boundary",
      "version":1,
      "requestId":"request-1",
      "operation":"boundary.echo",
      "payload":{"data":{"message":"hello from Nim"}}
    }""")

    check request.protocol == BoundaryProtocolName
    check request.version == BoundaryProtocolVersion
    check request.requestId == "request-1"
    check request.operationName == "boundary.echo"
    check request.operation.kind == boEcho
    check request.operation.data == %*{"message": "hello from Nim"}

  test "rejects unsupported versions with a stable code":
    expect BoundaryProtocolError:
      discard decodeRequest("""{
        "protocol":"nimino.core.boundary",
        "version":2,
        "requestId":"request-2",
        "operation":"boundary.echo",
        "payload":{}
      }""")

    try:
      discard decodeRequest("""{
        "protocol":"nimino.core.boundary",
        "version":2,
        "requestId":"request-2",
        "operation":"boundary.echo",
        "payload":{}
      }""")
    except BoundaryProtocolError as error:
      check error.code == "UNSUPPORTED_VERSION"

  test "encodes typed success and failure envelopes":
    let success = parseJson(
      encodeSuccess("request-3", "boundary.echo", %*{"data": {"ok": true}})
    )
    check success["status"].getStr() == "ok"
    check success["operation"].getStr() == "boundary.echo"
    check success["result"] == %*{"data": {"ok": true}}

    let failure = parseJson(
      encodeFailure(
        "request-4",
        "unknown.operation",
        "UNKNOWN_OPERATION",
        "not supported",
      )
    )
    check failure["status"].getStr() == "error"
    check failure["error"]["code"].getStr() == "UNKNOWN_OPERATION"
    check failure["error"]["retry"].getStr() == "never"

    let bounded = parseJson(
      encodeFailure(
        "request-5",
        "boundary.echo",
        "INVALID_REQUEST",
        repeat("界", 1_025),
      )
    )
    check bounded["error"]["message"].getStr().runeLen == 1_024

    let sanitized = parseJson(
      encodeFailure("", repeat("界", 129), "INVALID_REQUEST", "invalid")
    )
    check sanitized["requestId"].getStr() == "__unknown__"
    check sanitized["operation"].getStr() == "system.decode"

  test "rejects unknown and duplicate envelope fields":
    for malformed in [
      """{"protocol":"nimino.core.boundary","version":1,"requestId":"request-5","operation":"boundary.echo","payload":{"data":{}},"legacyMode":true}""",
      """{"protocol":"nimino.core.boundary","version":1,"requestId":"request-5","requestId":"request-6","operation":"boundary.echo","payload":{"data":{}}}""",
    ]:
      expect BoundaryProtocolError:
        discard decodeRequest(malformed)

  test "rejects operation payloads that do not match their typed shape":
    for malformed in [
      """{"protocol":"nimino.core.boundary","version":1,"requestId":"request-7","operation":"boundary.echo","payload":{"message":"missing data field"}}""",
      """{"protocol":"nimino.core.boundary","version":1,"requestId":"request-8","operation":"system.hello","payload":{"schemaHash":"abc"}}""",
    ]:
      expect BoundaryProtocolError:
        discard decodeRequest(malformed)

  test "routes typed event policy payloads and rejects unknown fields":
    let request = decodeRequest("""{
      "protocol":"nimino.core.boundary",
      "version":1,
      "requestId":"request-31",
      "operation":"domain.event.policy",
      "payload":{"decision":"classify","kind":30023,"dTagCount":1,"dTagLen":4}
    }""")
    check request.operation.kind == boEventPolicy
    check executeEventPolicy(request.operation.data, request.requestId) == %*{
      "decision": "classify", "disposition": "parameterized", "error": "none"
    }

    expect BoundaryProtocolError:
      discard executeEventPolicy(
        %*{
          "decision": "classify",
          "kind": 9,
          "dTagCount": 0,
          "dTagLen": 0,
          "legacyMode": true,
        },
        "request-31-invalid",
      )

  test "routes typed community policy payloads and rejects unknown fields":
    let request = decodeRequest("""{
      "protocol":"nimino.core.boundary",
      "version":1,
      "requestId":"request-85",
      "operation":"domain.community.policy",
      "payload":{"decision":"scope","request":{"requestCommunity":"018f5e5a-9b7d-7c01-a7bb-46fbe46d0001","resourceCommunity":null}}
    }""")
    check request.operation.kind == boCommunityPolicy
    check executeCommunityPolicy(request.operation.data, request.requestId) == %*{
      "decision": "scope", "allowed": false, "error": "resource_missing"
    }

    expect BoundaryProtocolError:
      discard executeCommunityPolicy(
        %*{
          "decision": "scope",
          "request": {
            "requestCommunity": "018f5e5a-9b7d-7c01-a7bb-46fbe46d0001",
            "resourceCommunity": nil,
          },
          "legacyMode": true,
        },
        "request-85-invalid",
      )
