# Nim core development lane

Status: active. Nim 2.2.10 is pinned and the Rust-independent build, check,
test, and timing lanes are available.

This lane is the development foundation for moving high-change product and
domain policy into Nim. It deliberately contains no migrated domain behavior
and no Nim/Rust or Chirps boundary; those are owned by later focused issues.

## Quick start

```bash
. ./bin/activate-hermit
just nim-ci
```

`just nim-ci` validates the Nimble package, type-checks and compiles the root
module, and runs its unit tests. It does not invoke `cargo`, `rustc`, or any
Rust build.

| Command | Purpose |
|---|---|
| `just nim-build` | Compile the package without linking a product binary |
| `just nim-check` | Validate package metadata and type-check the root module |
| `just nim-test` | Compile and run the core unit tests |
| `just nim-ci` | Run the complete Rust-independent lane |
| `just nim-baseline` | Measure the warm edit-to-test loop and full Nim lane |

Nim still emits C and therefore needs a C compiler on the host. No Rust
toolchain or Buzz service is required for these commands.

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
  src/nimino_core.nim         package root; no domain policy yet
  tests/test_nimino_core.nim  import and unit-test smoke
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
| `.github/workflows/ci.yml` (`Nim Core`) | Path-scoped Nim verification and timing | Independent Nim CI lane | keep | Superseded by the independent release pipeline | bootstrap contract test and CI artifact |
| Rust workspace | Existing product and adapter implementation | Stable host and adapter boundary | shrink | Per later responsibility-manifest issues | Not changed by this bootstrap |

The package root may expose build and diagnostic metadata, but domain behavior
must enter through its owning migration issue. Do not add compatibility modes,
dual-runtime product paths, database replication, or Chirps integration here.

## Clean-checkout verification

From a fresh Linux x86_64 clone:

```bash
. ./bin/activate-hermit
nim --version
just nim-ci
just nim-baseline
```

The first command invocation downloads and verifies the pinned archive. A clean
run is successful when the compiler reports 2.2.10, `nim-ci` passes without a
Rust build, and the timing JSON is produced under `target/nim/`.
