use std::{
    error::Error,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nimino_boundary::{
    BoundaryConfig, BoundaryError, BoundaryRequest, BoundaryResult, BoundaryRuntime, CallContext,
    EchoPayload, SCHEMA_HASH,
};
use serde::Serialize;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const COLD_START_P95_BUDGET_MS: f64 = 100.0;
const WARM_1K_P99_BUDGET_MS: f64 = 25.0;
const WARM_16K_P99_BUDGET_MS: f64 = 75.0;
const WARM_256K_P99_BUDGET_MS: f64 = 250.0;
const RECOVERY_P99_BUDGET_MS: f64 = 250.0;
#[cfg(target_os = "linux")]
const MAX_HOST_CPU_BUSY_RATIO: f64 = 0.6;
#[cfg(target_os = "linux")]
const HOST_IDLE_ATTEMPTS: usize = 20;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Distribution {
    samples: usize,
    p50_ms: f64,
    p95_ms: f64,
    p99_ms: f64,
    throughput_per_second: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PayloadScenario {
    payload_bytes: usize,
    ipc: Distribution,
    serde_only: Distribution,
    p99_budget_ms: f64,
    passed: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchmarkReport {
    schema_hash: &'static str,
    measured_at_unix_seconds: u64,
    os: &'static str,
    arch: &'static str,
    cold_start: Distribution,
    cold_start_p95_budget_ms: f64,
    payloads: Vec<PayloadScenario>,
    crash_to_ready: Distribution,
    cancel_to_ready: Distribution,
    recovery_p99_budget_ms: f64,
    passed: bool,
}

fn distribution(mut samples: Vec<Duration>) -> Distribution {
    samples.sort_unstable();
    let elapsed_seconds = samples.iter().sum::<Duration>().as_secs_f64();
    Distribution {
        samples: samples.len(),
        p50_ms: percentile(&samples, 50).as_secs_f64() * 1_000.0,
        p95_ms: percentile(&samples, 95).as_secs_f64() * 1_000.0,
        p99_ms: percentile(&samples, 99).as_secs_f64() * 1_000.0,
        throughput_per_second: if elapsed_seconds > 0.0 {
            samples.len() as f64 / elapsed_seconds
        } else {
            0.0
        },
    }
}

fn percentile(samples: &[Duration], percentile: usize) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    let index = ((samples.len() - 1) * percentile).div_ceil(100);
    samples[index]
}

#[cfg(target_os = "linux")]
fn cpu_snapshot() -> Result<(u64, u64), Box<dyn Error>> {
    let stat = std::fs::read_to_string("/proc/stat")?;
    let fields: Vec<u64> = stat
        .lines()
        .next()
        .ok_or_else(|| std::io::Error::other("/proc/stat is empty"))?
        .split_whitespace()
        .skip(1)
        .map(str::parse)
        .collect::<Result<_, _>>()?;
    if fields.len() < 5 {
        return Err(std::io::Error::other("/proc/stat CPU row is incomplete").into());
    }
    Ok((fields.iter().sum(), fields[3] + fields[4]))
}

#[cfg(target_os = "linux")]
fn cpu_busy_ratio(before: (u64, u64), after: (u64, u64)) -> Option<f64> {
    let total = after.0.checked_sub(before.0)?;
    let idle = after.1.checked_sub(before.1)?;
    (total > 0).then_some(1.0 - (idle as f64 / total as f64))
}

async fn wait_for_qualified_host() -> Result<(), Box<dyn Error>> {
    #[cfg(target_os = "linux")]
    for _ in 0..HOST_IDLE_ATTEMPTS {
        let before = cpu_snapshot()?;
        tokio::time::sleep(Duration::from_millis(500)).await;
        let after = cpu_snapshot()?;
        if cpu_busy_ratio(before, after).is_some_and(|ratio| ratio <= MAX_HOST_CPU_BUSY_RATIO) {
            return Ok(());
        }
    }
    #[cfg(target_os = "linux")]
    return Err(std::io::Error::other(
        "benchmark host stayed above 60% CPU; result is inconclusive",
    )
    .into());

    #[cfg(not(target_os = "linux"))]
    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::cpu_busy_ratio;

    #[test]
    fn cpu_busy_ratio_uses_delta_and_rejects_invalid_samples() {
        assert_eq!(cpu_busy_ratio((100, 40), (200, 80)), Some(0.6));
        assert_eq!(cpu_busy_ratio((100, 40), (100, 40)), None);
        assert_eq!(cpu_busy_ratio((200, 40), (100, 80)), None);
    }
}

fn write_inconclusive(output: &Path, reason: &str) -> Result<(), Box<dyn Error>> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&json!({
            "schemaHash": SCHEMA_HASH,
            "measuredAtUnixSeconds": SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "passed": false,
            "inconclusive": true,
            "reason": reason,
        }))?,
    )?;
    Ok(())
}

async fn measure_cold_start(
    worker: &Path,
    iterations: usize,
) -> Result<Distribution, BoundaryError> {
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let runtime = BoundaryRuntime::start(BoundaryConfig::new(worker)).await?;
        samples.push(started.elapsed());
        runtime.shutdown().await?;
    }
    Ok(distribution(samples))
}

async fn measure_payload(
    runtime: &BoundaryRuntime,
    payload_bytes: usize,
    iterations: usize,
    p99_budget_ms: f64,
) -> Result<PayloadScenario, Box<dyn Error>> {
    let client = runtime.client();
    let payload = json!({"blob": "x".repeat(payload_bytes)});
    let mut ipc_samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let result = client
            .call(
                BoundaryRequest::echo(payload.clone()),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await?;
        if result
            != (BoundaryResult::Echo(EchoPayload {
                data: payload.clone(),
            }))
        {
            return Err(std::io::Error::other("echo payload changed at the boundary").into());
        }
        ipc_samples.push(started.elapsed());
    }

    let mut serde_samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let encoded = serde_json::to_vec(&payload)?;
        let _: Value = serde_json::from_slice(&encoded)?;
        serde_samples.push(started.elapsed());
    }

    let ipc = distribution(ipc_samples);
    let serde_only = distribution(serde_samples);
    let passed = ipc.p99_ms <= p99_budget_ms;
    Ok(PayloadScenario {
        payload_bytes,
        ipc,
        serde_only,
        p99_budget_ms,
        passed,
    })
}

async fn measure_recovery(
    runtime: &BoundaryRuntime,
    iterations: usize,
) -> Result<(Distribution, Distribution), Box<dyn Error>> {
    let client = runtime.client();
    let mut crash_samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let crash = client
            .call(
                BoundaryRequest::test_crash(),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await;
        if !matches!(crash, Err(BoundaryError::WorkerExited { .. })) {
            return Err(std::io::Error::other("crash hook did not exit the worker").into());
        }
        client
            .call(
                BoundaryRequest::echo(json!({"after": "crash"})),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await?;
        crash_samples.push(started.elapsed());
    }

    let mut cancel_samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let cancellation = CancellationToken::new();
        let trigger = cancellation.clone();
        let cancel_task = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            trigger.cancel();
        });
        let started = Instant::now();
        let cancelled = client
            .call(
                BoundaryRequest::test_sleep(5_000),
                CallContext::with_timeout_and_cancellation(Duration::from_secs(2), cancellation),
            )
            .await;
        cancel_task.await?;
        if !matches!(cancelled, Err(BoundaryError::Cancelled)) {
            return Err(std::io::Error::other("cancellation hook did not cancel the call").into());
        }
        client
            .call(
                BoundaryRequest::echo(json!({"after": "cancel"})),
                CallContext::with_timeout(Duration::from_secs(2)),
            )
            .await?;
        cancel_samples.push(started.elapsed());
    }

    Ok((distribution(crash_samples), distribution(cancel_samples)))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let production_worker = arguments.next().map(PathBuf::from).ok_or_else(|| {
        std::io::Error::other("usage: boundary-bench PRODUCTION_WORKER TEST_WORKER OUTPUT_JSON")
    })?;
    let test_worker = arguments.next().map(PathBuf::from).ok_or_else(|| {
        std::io::Error::other("usage: boundary-bench PRODUCTION_WORKER TEST_WORKER OUTPUT_JSON")
    })?;
    let output = arguments.next().map(PathBuf::from).ok_or_else(|| {
        std::io::Error::other("usage: boundary-bench PRODUCTION_WORKER TEST_WORKER OUTPUT_JSON")
    })?;
    if arguments.next().is_some() {
        return Err(std::io::Error::other("unexpected benchmark argument").into());
    }

    if let Err(error) = wait_for_qualified_host().await {
        write_inconclusive(&output, &error.to_string())?;
        return Err(error);
    }

    let cold_start = measure_cold_start(&production_worker, 10).await?;
    let runtime = BoundaryRuntime::start(BoundaryConfig::new(&production_worker)).await?;
    let payloads = vec![
        measure_payload(&runtime, 1_024, 200, WARM_1K_P99_BUDGET_MS).await?,
        measure_payload(&runtime, 16_384, 100, WARM_16K_P99_BUDGET_MS).await?,
        measure_payload(&runtime, 262_144, 30, WARM_256K_P99_BUDGET_MS).await?,
    ];
    runtime.shutdown().await?;

    let recovery_runtime = BoundaryRuntime::start(BoundaryConfig::new(&test_worker)).await?;
    let (crash_to_ready, cancel_to_ready) = measure_recovery(&recovery_runtime, 10).await?;
    recovery_runtime.shutdown().await?;

    let passed = cold_start.p95_ms <= COLD_START_P95_BUDGET_MS
        && payloads.iter().all(|scenario| scenario.passed)
        && crash_to_ready.p99_ms <= RECOVERY_P99_BUDGET_MS
        && cancel_to_ready.p99_ms <= RECOVERY_P99_BUDGET_MS;
    let report = BenchmarkReport {
        schema_hash: SCHEMA_HASH,
        measured_at_unix_seconds: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        cold_start,
        cold_start_p95_budget_ms: COLD_START_P95_BUDGET_MS,
        payloads,
        crash_to_ready,
        cancel_to_ready,
        recovery_p99_budget_ms: RECOVERY_P99_BUDGET_MS,
        passed,
    };
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
    println!("{}", serde_json::to_string(&report)?);
    if !passed {
        return Err(std::io::Error::other("boundary benchmark budget exceeded").into());
    }
    Ok(())
}
