# Nim core development lane

Status: active. Nim 2.2.10 is pinned, the Rust-independent development lane is
available, and the versioned Nim/Rust worker boundary is accepted and tested.

This lane owns migrated event/message, data-contract, community, cluster
control, admission, authorization-invalidation, and ephemeral convergence
policy in Nim. It deliberately contains no Chirps runtime integration. A
separate `nimino-chirps` package pins and guards the Rust dependency; the
process boundary contains transport and lifecycle mechanics only. Focused
issues own each typed operation.

The production relay runs separate supervised worker processes for general
product policy, admission, and replicated control. Admission therefore remains
fail-closed without competing with query, sync, or ephemeral policy traffic.

## Quick start

```bash
. ./bin/activate-hermit
just nim-ci
just nim-boundary-ci
```

`just nim-ci` validates the Nimble package, type-checks and compiles the root
module, and runs its unit tests. It does not invoke `cargo`, `rustc`, or any
Rust build. `just nim-boundary-ci` is the explicit cross-language contract,
failure-lifecycle, and performance gate and therefore invokes Rust.

| Command | Purpose |
|---|---|
| `just nim-build` | Compile the package without linking a product binary |
| `just nim-check` | Validate package metadata and type-check the root module |
| `just nim-test` | Compile and run the core unit tests |
| `just nim-ci` | Run the complete Rust-independent lane |
| `just nim-baseline` | Measure the warm edit-to-test loop and full Nim lane |
| `just nim-boundary-build` | Build the production worker without test hooks |
| `just nim-boundary-test` | Run unit and real Rust↔Nim process scenarios |
| `just nim-boundary-benchmark` | Reproduce fixed payload and recovery budgets |
| `just nim-boundary-ci` | Run the complete versioned boundary gate |
| `just ci-lanes-contract` | Verify path ownership and the absent Mobile lane |

Nim still emits C and therefore needs a C compiler on the host. The Nim-only
commands need no Rust toolchain or Nimino service; boundary commands also use the
pinned Rust toolchain but still need no database, relay, Redis, or container.

## CI lane ownership

Pull requests use separate path filters for fast Nim feedback and integrated
boundary proof. The filters and default git hooks are executable policy, checked
by `just ci-lanes-contract`.

| Change | Required lane | Cargo work |
|---|---|---|
| Nim domain module or unit test | `Nim Core` (`just nim-ci`) | none |
| Boundary Nim protocol/worker or `nimino-boundary` adapter | `Nim Core` when Nim changed, plus `Nim/Rust Boundary` | focused boundary crate only |
| Other Rust workspace code | Rust lint/test/integration lanes | normal Rust gates |
| Mobile path | no product CI lane | none |

The `Nim/Rust Boundary` job owns the cross-language lifecycle tests and uploads
the performance artifact. The `Nim Core` job never invokes Cargo. Mobile source
removal remains a separate physical-deletion issue; it is not kept alive by CI.

## Toolchain and package layout

The repository-owned Hermit manifest declares the official Nim 2.2.10 binary
distribution for Linux and macOS on x86_64 and ARM64, pins every archive by
SHA-256, and isolates Nimble state under `.hermit/nimble`. The bootstrap CI
gate and reference timings currently qualify Linux x86_64. The WSL support
contract and release support matrix are owned by their dedicated issues rather
than being inferred from archive availability here.

```text
hermit-packages/nim.hcl       pinned compiler package and checksums
bin/.nim-2.2.10.pkg           Hermit package selector
nim/nimino_core/
  nimino_core.nimble          package metadata and compiler constraint
  nim.cfg                     source, cache, and output paths
  src/nimino_core.nim         package root and version metadata
  src/nimino_core/domain/     pure product/domain decisions
  src/nimino_core_worker.nim  long-lived worker composition root
  src/nimino_core/boundary/   strict v1 protocol and length framing
  tests/                      package and boundary unit tests
contracts/nim-rust-boundary/  machine-readable versioned source of truth
crates/nimino-boundary/       Rust process/framing lifecycle adapter only
crates/nimino-chirps/         sole direct Alopex Chirps dependency boundary
```

Compiler output is written below ignored `target/nim/`; it does not dirty the
package tree.

## Feedback baseline

The baseline command copies the package into a unique ignored directory below
`target/nim/`, measures a clean `just nim-ci` lane there, then adds a temporary
production symbol and unit assertion to that copy. It measures the resulting
`just nim-test` compile/link/run and removes the probe directory through an EXIT
trap. The tracked source tree is never edited. The committed reference below
was recorded on the issue #13 implementation checkout.

| Recorded at | Host | Compiler | Edit to unit test | Clean Nim CI lane |
|---|---|---|---:|---:|
| 2026-08-25 | Linux x86_64 | Nim 2.2.10 | 16.454 s | 37.766 s |

Run `just nim-baseline` to write machine-readable evidence to
`target/nim/feedback-baseline.json`. The `Nim Core` GitHub Actions job performs
the same measurement and uploads `nim-feedback-baseline` on each applicable CI
run, so runner-to-runner changes remain inspectable without committing build
artifacts.

These values are observations, not a pass/fail performance budget. Establish a
budget only after representative domain modules exist.

## Lifecycle ownership

| Path | Current role | Target role | Action | Delete when | Proof |
|---|---|---|---|---|---|
| `hermit-packages/nim.hcl` | Pins the bootstrap compiler distributions | Canonical Nim toolchain definition | keep | Replaced by an explicitly approved toolchain owner | Hermit manifest validation and clean-clone install |
| `nim/nimino_core/` | Importable package skeleton | Owner of high-change product and domain policy | keep | Never as part of the current cutover | `just nim-ci` |
| `contracts/nim-rust-boundary/v1/` | Exact schema/error/lifecycle bundle | Canonical cross-language contract | keep | Explicit new-version cutover | checksum and fixture gates |
| `contracts/nimino-event/v1/` | Event policy corpus and Rust deletion inventory | Canonical event/message decision contract | keep | Explicit new-version cutover | Nim unit and real worker golden tests |
| `contracts/nimino-community/v1/` | Community lifecycle/isolation corpus and Rust policy inventory | Canonical community decision contract | keep | Explicit new-version cutover | Nim unit and real worker golden tests |
| `crates/nimino-boundary/` | Supervised process adapter | Spawn/frame/queue/timeout/cancel/kill/reap only | keep narrow | Explicit replacement boundary | dependency deny and cross-language tests |
| `.github/workflows/ci.yml` (`Nim Core`) | Path-scoped Nim verification and timing | Independent Nim CI lane | keep | Superseded by the independent release pipeline | bootstrap contract test and CI artifact |
| Rust workspace | Product host and effect adapters | Stable host and adapter boundary | keep narrow | Explicit replacement of an adapter | responsibility manifest and boundary tests |

The package root may expose build and diagnostic metadata, but domain behavior
must enter through its owning migration issue as a typed operation. Do not add
generic operation/value entrypoints, compatibility modes, dual-runtime product
paths, database replication, or Chirps policy here. See
[`../adr/nim-rust-boundary-v1.md`](../adr/nim-rust-boundary-v1.md) for ownership,
failure semantics, and benchmark evidence.

## Clean-checkout verification

From a fresh Linux x86_64 clone:

```bash
. ./bin/activate-hermit
nim --version
just nim-ci
just nim-boundary-ci
just nim-baseline
```

The first command invocation downloads and verifies the pinned archive. A clean
run is successful when the compiler reports 2.2.10, `nim-ci` passes without a
Rust build, the cross-language contract and performance gate passes, and the
timing JSON is produced under `target/nim/`.
