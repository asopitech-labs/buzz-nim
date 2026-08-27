# ADR: Nim/Rust boundary v1

Status: Accepted for the Nimino v1.0 cutover. Implements Issue #22.

## Decision summary

Nimino uses a long-lived, stateless Nim child process supervised by a narrow
Rust adapter. Messages use four-byte unsigned big-endian length framing with a
UTF-8 JSON body. Startup requires an exact protocol version, schema hash,
worker role, frame limit, and max-inflight match. No downgrade, old Buzz mode,
or Rust domain fallback exists.

Rust owns process and transport mechanics. Nim owns operation semantics and
all product/domain decisions. Alopex Chirps, storage adapters, and OS adapters
remain Rust where appropriate, but this boundary does not grant them domain,
database, replication, sync, quorum, lease, or cluster-authority ownership.

The checked-in source of truth is
`contracts/nim-rust-boundary/v1/`. The contract bundle hash is
`3b799b4bf7bf4fb3720de2103cf37642c781ea3746de5a98ddd9f04a5293e233`.

## System invariant

At most one request is active in a Nim worker. A response changes caller state
only after its complete frame, schema, protocol metadata, and request ID have
been accepted. Timeout, cancellation, crash, or protocol corruption kills and
reaps that worker before another request is sent. Caller-future abandonment is
the same as cancellation. A reap failure is terminal: no replacement worker is
started. An in-flight request is never retried automatically.

This makes cancellation deterministic without moving side-effect ownership
into the boundary. Future Nim domain operations must return decisions or typed
effect intents; they must not directly commit external side effects that could
outlive a killed worker.

## Use cases and activities

| Use case | Activities | Completion condition |
|---|---|---|
| Start core worker | spawn → frame `system.hello` → exact contract comparison → ready | all v1 handshake fields match |
| Execute Nim operation | typed request → bounded enqueue → one active exchange → validate correlation → typed result | exactly one terminal result |
| Cancel or time out | terminal signal wins before accepted response → kill → reap → stopped | no late response can be applied |
| Recover from crash/corruption | reject active call without retry → reap → spawn and handshake on next queued call | fresh generation reaches ready |
| Shut down | cancel active and queued work → kill/reap | lifecycle returns to stopped |

The biased terminal race commits a fully decoded response when the response and
cancel signal are ready in the same poll. Otherwise cancellation wins and the
worker is recycled. The monotonic call deadline begins before enqueue, so queue
wait and worker restart consume the same budget. Queue overflow returns
`BACKPRESSURE` immediately and does not mutate worker state.

## Responsibility map

| Decision or side effect | Owner | Boundary artifact |
|---|---|---|
| Product rules and domain invariants | Nimino core | typed operation/result variants |
| DB, replication, sync, conflict, projection semantics | later Nimino domain/data issues | not implemented by this adapter |
| Cluster authority, quorum, leases, routing policy | Nimino control plane | Chirps facts may become typed input only |
| Node negotiation and secure cluster messaging | Alopex Chirps Rust adapter | outside this contract until its focused issue |
| Spawn, framing, queue, timeout, cancel, kill, reap | `nimino-boundary` Rust crate | host error taxonomy |
| Contract version lifecycle | `contracts/nim-rust-boundary/v1` | schemas, errors, state machine, fixtures, hash |
| External effect commit | calling use case and explicit adapter | only after an accepted Nim result |
| Logs | worker stderr / host observability sink | never worker stdout |

## Model classification and pattern choice

- `BoundaryRequest` and `BoundaryResponse` are contract entities, not domain
  models. The public Rust API exposes typed constructors rather than an
  arbitrary operation-string/value constructor.
- `BoundaryFault` is a remote contract entity. Its retry rule is the enum
  `never`, `idempotent_only`, or `after_refresh`; exception text never decides
  retries.
- `BoundaryRuntime` is the lifecycle service and composition root.
- `BoundaryClient` is a bounded Facade for callers.
- The Rust process implementation is an integration Adapter. It has no imports
  from Buzz domain crates, SQLx, Redis, Nostr, Iroh, or Chirps.
- The worker lifecycle follows the versioned State model in `lifecycle.json`.

The operation union contains the exact-match handshake, diagnostic echo, and
the `domain.event.policy` variant added by Issue #31. That domain variant owns
event classification, replacement ordering, thread plans, deletion plans, and
reaction plans; its contract and golden corpus live under
`contracts/nimino-event/`. Test-only builds add deterministic sleep, crash,
corruption, and mismatch operations. Later domain variants must likewise enter
through their owning migration issue and extend the schema and both typed
adapters together.

## Framing and lifecycle contract

- Transport: child stdin/stdout pipes; no socket path, port, or endpoint ACL.
- Frame: `u32be length` followed by UTF-8 JSON.
- Maximum frame body: 1,048,576 bytes.
- Worker stdout: frames only. Worker stderr: diagnostics only.
- Worker concurrency: one active request.
- Host queue: bounded; default capacity 64, hard maximum 1,024; no silent drop.
- Startup: `system.hello` must exactly match version, schema hash, role,
  max-frame, and max-inflight.
- Remote errors and host/process errors are separate typed sets.
- Timeout/cancel/caller drop: kill and reap; no cooperative cancel RPC in v1.
- Crash/corruption: fail the active request without retry and recycle.
- Next queued call: spawn once and handshake; no restart loop or Rust policy
  fallback.
- Shutdown: cancel active and queued calls, then kill and reap the worker.
- Cleanup failure: return `CLEANUP_FAILED`, enter terminal failed state, and do
  not spawn a replacement whose coexistence cannot be ruled out.

## Performance decision

Budgets were encoded in `boundary-bench` before the first accepted run:

| Scenario | Gate |
|---|---:|
| cold spawn + handshake p95 | ≤ 100 ms |
| 1 KiB warm round-trip p99 | ≤ 25 ms |
| 16 KiB warm round-trip p99 | ≤ 75 ms |
| 256 KiB warm round-trip p99 | ≤ 250 ms |
| crash or cancel through next ready response p99 | ≤ 250 ms |

GitHub Actions run `32816181597` produced the Linux x86_64 reference and passed
every gate:

| Scenario | Samples | Result |
|---|---:|---:|
| cold spawn + handshake p95 | 10 | 0.960 ms |
| 1 KiB warm p50 / p99 | 200 | 0.086 / 0.139 ms |
| 16 KiB warm p50 / p99 | 100 | 0.357 / 0.477 ms |
| 256 KiB warm p50 / p99 | 30 | 4.913 / 6.493 ms |
| crash through next ready p50 / p99 | 10 | 0.910 / 0.946 ms |
| cancel through next ready p50 / p99 | 10 | 12.048 / 12.184 ms |

Cold-start and payload measurements use the production worker. Failure recovery
uses the test-hook worker and records ten samples per scenario. The benchmark
also records local serde encode/decode as a lower bound. It does
not claim serde-only work is a viable cross-language design. Raw evidence is in
`docs/benchmarks/nim-rust-boundary-linux-x86_64.json`; CI regenerates the same
scenarios into an artifact.

On Linux, measurement starts only after a 500 ms CPU sample reports at most 60%
host utilization. The preflight retries for ten seconds; a persistently busy
shared host writes an `inconclusive` artifact and fails without comparing noisy
wall-clock samples to release budgets. CI must rerun that gate on a qualified
runner rather than accepting or relaxing an inconclusive result.

Performance is a gate after semantic filtering, not the sole selector. The
process boundary meets the declared budget while preserving failure isolation
and a Rust-independent Nim edit/test lane.

## Alternatives rejected

| Candidate | Decision |
|---|---|
| C ABI / dynamic library | Rejected: requires unsafe host calls, couples allocator/GC/exception failure, cannot deterministically kill one call, and forces host relinking after Nim changes |
| Nim-generated C statically linked into Rust | Rejected: destroys the Nim-only feedback lane and process isolation |
| NDJSON | Rejected: line limits and accidental stdout logging become framing semantics |
| Unix socket or TCP | Rejected for v1: endpoint discovery, permissions, stale cleanup, and Windows/WSL differences add no required capability |
| gRPC/Protobuf | Rejected for v1: code generation and runtime cost precede stable domain schemas |
| Shared memory | Rejected: ownership, synchronization, and crash cleanup are disproportionate |
| Process per request | Rejected: process isolation is retained by the chosen long-lived worker without paying cold startup for each call |

C ABI was disqualified by the repository's no-unsafe invariant and required
failure semantics before performance comparison. The benchmark therefore
compares the selected real IPC path with its serialization lower bound and its
cold-process cost rather than retaining a forbidden ABI prototype.

## File and artifact lifecycle

| Path | Current role | Target role | Action | Delete when | Proof |
|---|---|---|---|---|---|
| `contracts/nim-rust-boundary/v1/` | v1 schemas and lifecycle | canonical boundary contract | keep | replaced only by an explicit new-version cutover | checksum gate and fixture tests |
| `nim/nimino_core/src/nimino_core/boundary/` | Nim codec and framing | Nim side of typed boundary | keep | never during v1 | Nim unit tests and cross-language scenarios |
| `nim/nimino_core/src/nimino_core_worker.nim` | diagnostic worker shell | Nim domain worker composition root | grow without transport policy | replaced by a new worker role/version | exact handshake and scenario tests |
| `crates/nimino-boundary/` | Rust supervisor adapter | process/framing lifecycle only | keep narrow | a future boundary replaces process IPC | dependency deny gate and Clippy |
| `target/nim/nimino_boundary/` | generated workers and benchmark | ignored replay artifacts | replace per run | immediately after build cache cleanup | Git remains clean |
| existing Rust domain paths | current product logic | removed or reduced by focused issues | unchanged here, later shrink/delete | owning migration acceptance and final cutover | later responsibility/dead-code gates |

## Verification ownership

- Nim unit: request validation, stable remote codes, response encoding.
- Rust unit: strict fixtures, host error inventory, frame round-trip, truncated
  and oversized frames.
- Cross-language: handshake mismatch, echo, typed remote failure, queue-inclusive
  deadline, cancellation, caller drop, runtime-drop reap, crash, queue
  saturation, stdout corruption, malformed JSON, wrong request ID, and
  successful fresh-worker recovery.
- Architecture: checksum equality, no generic operation constructor, no
  compatibility terminology/path, no forbidden domain/storage/cluster imports,
  and no library stdout writes.
- Performance: fixed cold, payload, crash recovery, and cancel recovery budgets.

Run all evidence with:

```bash
. ./bin/activate-hermit
just nim-boundary-ci
```

`just nim-ci` remains Rust-independent. `just nim-boundary-ci` is the explicit
cross-language gate.

## Operational and release impact

This ADR fixes the boundary but does not migrate a domain or switch the running
product. Domain migration issues will compose typed ports on this worker.
Physical deletion and the single incompatible public cutover remain owned by
Issue #12 and its child gates. There will be no production dual-runtime mode.

Linux x86_64 is verified here. WSL2, Linux ARM64, macOS x64, and macOS ARM64
runtime certification remain unverified by this issue and must be supplied by
their platform/release issues before the final release gate can pass.
