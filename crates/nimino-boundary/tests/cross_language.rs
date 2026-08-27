#![cfg(feature = "test-hooks")]

use std::{path::PathBuf, process::Stdio, time::Duration};

use nimino_boundary::{
    BoundaryConfig, BoundaryError, BoundaryRequest, BoundaryResponse, BoundaryResult,
    BoundaryRuntime, CallContext, EchoPayload, EventPolicyRequest, EventPolicyResult,
    RemoteErrorCode, MAX_FRAME_BYTES, MAX_INFLIGHT, PROTOCOL_NAME, PROTOCOL_VERSION, SCHEMA_HASH,
    WORKER_ROLE,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

fn worker_path() -> PathBuf {
    std::env::var_os("NIMINO_BOUNDARY_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_WORKER must point to the test worker")
}

fn mismatch_worker_path() -> PathBuf {
    std::env::var_os("NIMINO_BOUNDARY_MISMATCH_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_MISMATCH_WORKER must point to the mismatch worker")
}

fn production_worker_path() -> PathBuf {
    std::env::var_os("NIMINO_BOUNDARY_PRODUCTION_WORKER")
        .map(PathBuf::from)
        .expect("NIMINO_BOUNDARY_PRODUCTION_WORKER must point to the production worker")
}

async fn runtime(queue_capacity: usize) -> BoundaryRuntime {
    BoundaryRuntime::start(BoundaryConfig::new(worker_path()).with_queue_capacity(queue_capacity))
        .await
        .expect("worker starts and negotiates v1")
}

async fn raw_exchange(
    stdin: &mut tokio::process::ChildStdin,
    stdout: &mut tokio::process::ChildStdout,
    request: &Value,
) -> Value {
    let request = serde_json::to_vec(request).expect("encode raw request");
    stdin
        .write_all(&(request.len() as u32).to_be_bytes())
        .await
        .expect("write frame length");
    stdin.write_all(&request).await.expect("write frame body");
    stdin.flush().await.expect("flush request");

    let response_length = stdout.read_u32().await.expect("response frame length") as usize;
    let mut response = vec![0; response_length];
    stdout
        .read_exact(&mut response)
        .await
        .expect("response frame body");
    serde_json::from_slice(&response).expect("response JSON")
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn rust_and_nim_round_trip_and_preserve_typed_remote_failure() {
    let runtime = runtime(8).await;
    let client = runtime.client();

    let value = client
        .call(
            BoundaryRequest::echo(json!({"message": "round trip"})),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect("echo succeeds");
    assert_eq!(
        value,
        BoundaryResult::Echo(EchoPayload {
            data: json!({"message": "round trip"})
        })
    );

    let error = client
        .call(
            BoundaryRequest::test_remote_failure(),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect_err("unknown operation is typed");
    assert!(matches!(
        error,
        BoundaryError::Remote(ref fault) if fault.code == RemoteErrorCode::UnknownOperation
    ));

    runtime.shutdown().await.expect("clean shutdown");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct EventPolicyCorpus {
    schema_version: u16,
    cases: Vec<EventPolicyCase>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventPolicyCase {
    name: String,
    standard: String,
    input: EventPolicyRequest,
    expected: EventPolicyResult,
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn event_policy_golden_corpus_crosses_the_real_worker_boundary() {
    let corpus: EventPolicyCorpus = serde_json::from_str(include_str!(
        "../../../contracts/nimino-event/v1/golden.json"
    ))
    .expect("valid event policy corpus");
    assert_eq!(corpus.schema_version, 1);

    let runtime = runtime(8).await;
    let client = runtime.client();
    for case in corpus.cases {
        assert!(
            !case.standard.is_empty(),
            "{} has no NIP reference",
            case.name
        );
        let result = client
            .call(
                BoundaryRequest::event_policy(case.input),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .unwrap_or_else(|error| panic!("{} failed: {error}", case.name));
        assert_eq!(
            result,
            BoundaryResult::EventPolicy(case.expected),
            "{}",
            case.name
        );
    }
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn timeout_and_cancel_recycle_the_worker_before_the_next_call() {
    let runtime = runtime(8).await;
    let client = runtime.client();

    let timeout = client
        .call(
            BoundaryRequest::test_sleep(5_000),
            CallContext::with_timeout(Duration::from_millis(50)),
        )
        .await
        .expect_err("sleep exceeds deadline");
    assert!(matches!(timeout, BoundaryError::DeadlineExceeded));

    let cancellation = CancellationToken::new();
    let cancel_from_test = cancellation.clone();
    let cancel_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        cancel_from_test.cancel();
    });
    let cancelled = client
        .call(
            BoundaryRequest::test_sleep(5_000),
            CallContext::with_timeout_and_cancellation(Duration::from_secs(2), cancellation),
        )
        .await
        .expect_err("call is cancelled");
    cancel_task.await.expect("cancel task");
    assert!(matches!(cancelled, BoundaryError::Cancelled));

    let recovered = client
        .call(
            BoundaryRequest::echo(json!({"recovered": true})),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect("a fresh worker handles the next call");
    assert_eq!(
        recovered,
        BoundaryResult::Echo(EchoPayload {
            data: json!({"recovered": true})
        })
    );

    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn dropping_the_call_future_cancels_and_recycles_the_worker() {
    let runtime = runtime(8).await;
    let client = runtime.client();
    let abandoned_client = client.clone();
    let abandoned = tokio::spawn(async move {
        abandoned_client
            .call(
                BoundaryRequest::test_sleep(5_000),
                CallContext::with_timeout(Duration::from_secs(10)),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    abandoned.abort();
    let _ = abandoned.await;

    let started = std::time::Instant::now();
    let recovered = client
        .call(
            BoundaryRequest::echo(json!({"after": "abandon"})),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect("abandoned call is cancelled before the next call");
    assert_eq!(
        recovered,
        BoundaryResult::Echo(EchoPayload {
            data: json!({"after": "abandon"})
        })
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn queued_call_deadline_includes_time_waiting_for_the_worker() {
    let runtime = runtime(2).await;
    let client = runtime.client();
    let busy_client = client.clone();
    let busy = tokio::spawn(async move {
        busy_client
            .call(
                BoundaryRequest::test_sleep(500),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(30)).await;

    let started = std::time::Instant::now();
    let queued = client
        .call(
            BoundaryRequest::echo(json!({"tooLate": true})),
            CallContext::with_timeout(Duration::from_millis(50)),
        )
        .await
        .expect_err("queue wait consumes the call budget");
    assert!(matches!(queued, BoundaryError::DeadlineExceeded));
    assert!(started.elapsed() < Duration::from_millis(250));

    assert!(busy.await.expect("busy task").is_ok());
    runtime.shutdown().await.expect("clean shutdown");
}

#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn dropping_runtime_reaps_the_worker_process() {
    let runtime = runtime(2).await;
    let result = runtime
        .client()
        .call(
            BoundaryRequest::test_pid(),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect("test worker returns its pid");
    let BoundaryResult::Test(payload) = result else {
        panic!("pid hook returned the wrong result variant");
    };
    let pid = payload["pid"].as_u64().expect("numeric worker pid");

    drop(runtime);
    let process_path = PathBuf::from(format!("/proc/{pid}"));
    for _ in 0..100 {
        if !process_path.exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("worker {pid} was not reaped after runtime drop");
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn crash_is_isolated_and_queue_capacity_applies_backpressure() {
    let runtime = runtime(1).await;
    let client = runtime.client();

    let crash = client
        .call(
            BoundaryRequest::test_crash(),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect_err("test worker exits");
    assert!(matches!(crash, BoundaryError::WorkerExited { .. }));

    let first_client = client.clone();
    let first = tokio::spawn(async move {
        first_client
            .call(
                BoundaryRequest::test_sleep(500),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let second_client = client.clone();
    let second = tokio::spawn(async move {
        second_client
            .call(
                BoundaryRequest::echo(json!({"queued": true})),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(20)).await;

    let backpressure = client
        .call(
            BoundaryRequest::echo(json!({"overflow": true})),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect_err("bounded queue rejects overflow immediately");
    assert!(matches!(backpressure, BoundaryError::Backpressure));

    assert!(first.await.expect("first task").is_ok());
    assert!(second.await.expect("second task").is_ok());
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires both Nim test workers; run `just nim-boundary-test`"]
async fn handshake_rejects_schema_mismatch_without_fallback() {
    let error = BoundaryRuntime::start(BoundaryConfig::new(mismatch_worker_path()))
        .await
        .expect_err("schema mismatch prevents readiness");
    assert!(matches!(error, BoundaryError::ContractMismatch));
}

#[tokio::test]
#[ignore = "requires the Nim test worker; run `just nim-boundary-test`"]
async fn corrupted_or_mismatched_responses_recycle_before_the_next_call() {
    for request in [
        BoundaryRequest::test_garbage(),
        BoundaryRequest::test_malformed(),
        BoundaryRequest::test_wrong_id(),
    ] {
        let runtime = runtime(8).await;
        let client = runtime.client();
        let error = client
            .call(request, CallContext::with_timeout(Duration::from_secs(2)))
            .await
            .expect_err("invalid response is rejected");
        assert!(matches!(error, BoundaryError::ProtocolViolation(_)));

        let recovered = client
            .call(
                BoundaryRequest::echo(json!({"fresh": true})),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await
            .expect("replacement worker is ready");
        assert_eq!(
            recovered,
            BoundaryResult::Echo(EchoPayload {
                data: json!({"fresh": true})
            })
        );
        runtime.shutdown().await.expect("clean shutdown");
    }
}

#[tokio::test]
#[ignore = "requires the production Nim worker; run `just nim-boundary-test`"]
async fn production_worker_excludes_failure_test_operations() {
    let runtime = BoundaryRuntime::start(BoundaryConfig::new(production_worker_path()))
        .await
        .expect("production worker reaches ready");
    let client = runtime.client();

    let unavailable = client
        .call(
            BoundaryRequest::test_crash(),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect_err("test operation is absent from production");
    assert!(matches!(
        unavailable,
        BoundaryError::Remote(ref fault) if fault.code == RemoteErrorCode::UnknownOperation
    ));

    let value = client
        .call(
            BoundaryRequest::echo(json!({"production": true})),
            CallContext::with_timeout(Duration::from_secs(2)),
        )
        .await
        .expect("production worker remains ready");
    assert_eq!(
        value,
        BoundaryResult::Echo(EchoPayload {
            data: json!({"production": true})
        })
    );
    runtime.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
#[ignore = "requires the production Nim worker; run `just nim-boundary-test`"]
async fn production_worker_bounds_generated_error_messages() {
    let mut child = tokio::process::Command::new(production_worker_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn production worker");
    let mut stdin = child.stdin.take().expect("worker stdin");
    let mut stdout = child.stdout.take().expect("worker stdout");
    let unknown_field = "界".repeat(2_000);
    for request in [
        json!({
            "protocol": "nimino.core.boundary",
            "version": 1,
            "requestId": "oversized-diagnostic",
            "operation": "boundary.echo",
            "payload": {"data": null},
            (unknown_field): true,
        }),
        json!({
            "protocol": "nimino.core.boundary",
            "version": 1,
            "requestId": "x".repeat(129),
            "operation": "boundary.echo",
            "payload": {"data": null},
        }),
        json!({
            "protocol": "nimino.core.boundary",
            "version": 1,
            "requestId": "",
            "operation": "",
            "payload": {},
        }),
        json!({
            "protocol": "nimino.core.boundary",
            "version": 1,
            "requestId": "invalid-operation",
            "operation": "x".repeat(129),
            "payload": {},
        }),
    ] {
        let value = raw_exchange(&mut stdin, &mut stdout, &request).await;
        assert_eq!(value["error"]["code"], "INVALID_REQUEST");
        assert!((1..=128).contains(
            &value["requestId"]
                .as_str()
                .expect("request id")
                .chars()
                .count()
        ));
        assert!((1..=128).contains(
            &value["operation"]
                .as_str()
                .expect("operation")
                .chars()
                .count()
        ));
        assert!(
            value["error"]["message"]
                .as_str()
                .expect("message")
                .chars()
                .count()
                <= 1_024
        );
        serde_json::from_value::<BoundaryResponse>(value).expect("schema-valid failure response");
    }

    child.start_kill().expect("kill worker");
    child.wait().await.expect("reap worker");
}

#[tokio::test]
#[ignore = "requires the production Nim worker; run `just nim-boundary-test`"]
async fn production_worker_converts_oversized_success_without_exiting() {
    let mut child = tokio::process::Command::new(production_worker_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn production worker");
    let mut stdin = child.stdin.take().expect("worker stdin");
    let mut stdout = child.stdout.take().expect("worker stdout");

    let hello = raw_exchange(
        &mut stdin,
        &mut stdout,
        &json!({
            "protocol": PROTOCOL_NAME,
            "version": PROTOCOL_VERSION,
            "requestId": "raw-hello",
            "operation": "system.hello",
            "payload": {
                "schemaHash": SCHEMA_HASH,
                "workerRole": WORKER_ROLE,
                "maxFrameBytes": MAX_FRAME_BYTES,
                "maxInflight": MAX_INFLIGHT,
            },
        }),
    )
    .await;
    serde_json::from_value::<BoundaryResponse>(hello).expect("valid ready response");

    let mut maximal = json!({
        "protocol": PROTOCOL_NAME,
        "version": PROTOCOL_VERSION,
        "requestId": "max-frame",
        "operation": "boundary.echo",
        "payload": {"data": {"blob": ""}},
    });
    let base_length = serde_json::to_vec(&maximal).expect("base request").len();
    maximal["payload"]["data"]["blob"] = Value::String("x".repeat(MAX_FRAME_BYTES - base_length));
    assert_eq!(
        serde_json::to_vec(&maximal).expect("max request").len(),
        MAX_FRAME_BYTES
    );
    let oversized = raw_exchange(&mut stdin, &mut stdout, &maximal).await;
    let fault = serde_json::from_value::<BoundaryResponse>(oversized)
        .expect("valid failure response")
        .into_result()
        .expect_err("response is too large");
    assert_eq!(fault.code, RemoteErrorCode::FrameTooLarge);

    let small = raw_exchange(
        &mut stdin,
        &mut stdout,
        &json!({
            "protocol": PROTOCOL_NAME,
            "version": PROTOCOL_VERSION,
            "requestId": "small-echo",
            "operation": "boundary.echo",
            "payload": {"data": "still-alive"},
        }),
    )
    .await;
    assert!(matches!(
        serde_json::from_value::<BoundaryResponse>(small)
            .expect("valid echo response")
            .into_result(),
        Ok(BoundaryResult::Echo(EchoPayload { data })) if data == "still-alive"
    ));

    child.start_kill().expect("kill worker");
    child.wait().await.expect("reap worker");
}
