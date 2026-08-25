use nimino_boundary::{
    BoundaryFault, BoundaryRequest, BoundaryResponse, BoundaryResult, EchoPayload, RemoteErrorCode,
    RetryDisposition, HOST_ERROR_CODES, PROTOCOL_NAME, PROTOCOL_VERSION,
};
use serde_json::json;

#[test]
fn request_fixture_round_trips_without_losing_contract_fields() {
    let fixture =
        include_str!("../../../contracts/nim-rust-boundary/v1/fixtures/echo.request.json");
    let request: BoundaryRequest = serde_json::from_str(fixture).expect("valid request fixture");

    assert_eq!(request.protocol(), PROTOCOL_NAME);
    assert_eq!(request.version(), PROTOCOL_VERSION);
    assert_eq!(request.request_id(), "018f5e5a-9b7d-7c01-a7bb-46fbe46d0001");
    assert_eq!(request.operation_name(), "boundary.echo");
    assert_eq!(
        request.echo_data(),
        Some(&json!({"message": "hello from Rust"}))
    );
}

#[test]
fn response_fixtures_are_typed_success_or_failure() {
    let success: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/echo.response.json"
    ))
    .expect("valid success fixture");
    assert_eq!(
        success.into_result().expect("success response"),
        BoundaryResult::Echo(EchoPayload {
            data: json!({"message": "hello from Rust"})
        })
    );

    let failure: BoundaryResponse = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/fixtures/unknown-operation.response.json"
    ))
    .expect("valid failure fixture");
    assert_eq!(
        failure.into_result().expect_err("remote failure"),
        BoundaryFault {
            code: RemoteErrorCode::UnknownOperation,
            message: "operation is not supported".to_owned(),
            retry: RetryDisposition::Never,
        }
    );
}

#[test]
fn malformed_response_metadata_and_fault_bounds_are_rejected() {
    let wrong_version = json!({
        "protocol": PROTOCOL_NAME,
        "version": 2,
        "requestId": "request-1",
        "operation": "boundary.echo",
        "status": "ok",
        "result": {}
    });
    assert!(serde_json::from_value::<BoundaryResponse>(wrong_version).is_err());

    let oversized_message = json!({
        "protocol": PROTOCOL_NAME,
        "version": PROTOCOL_VERSION,
        "requestId": "request-1",
        "operation": "boundary.echo",
        "status": "error",
        "error": {
            "code": "INTERNAL_ERROR",
            "message": "x".repeat(1_025),
            "retry": "idempotent_only"
        }
    });
    assert!(serde_json::from_value::<BoundaryResponse>(oversized_message).is_err());

    let invalid_retry = json!({
        "protocol": PROTOCOL_NAME,
        "version": PROTOCOL_VERSION,
        "requestId": "request-1",
        "operation": "unknown.operation",
        "status": "error",
        "error": {
            "code": "UNKNOWN_OPERATION",
            "message": "not supported",
            "retry": "after_refresh"
        }
    });
    assert!(serde_json::from_value::<BoundaryResponse>(invalid_retry).is_err());
}

#[test]
fn unknown_contract_fields_are_rejected() {
    let malformed = json!({
        "protocol": PROTOCOL_NAME,
        "version": PROTOCOL_VERSION,
        "requestId": "request-1",
        "operation": "boundary.echo",
        "payload": {"data": {}},
        "legacyMode": true
    });

    assert!(serde_json::from_value::<BoundaryRequest>(malformed).is_err());

    let duplicate = r#"{
        "protocol":"nimino.core.boundary",
        "version":1,
        "requestId":"request-1",
        "requestId":"request-2",
        "operation":"boundary.echo",
        "payload":{"data":{}}
    }"#;
    assert!(serde_json::from_str::<BoundaryRequest>(duplicate).is_err());
}

#[test]
fn host_error_inventory_matches_the_versioned_manifest() {
    let manifest: serde_json::Value = serde_json::from_str(include_str!(
        "../../../contracts/nim-rust-boundary/v1/errors.json"
    ))
    .expect("valid error manifest");
    let host_codes: Vec<&str> = manifest["host"]
        .as_array()
        .expect("host error array")
        .iter()
        .map(|entry| entry["code"].as_str().expect("host error code"))
        .collect();
    assert_eq!(host_codes, HOST_ERROR_CODES);
}
