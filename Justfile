# Nimino — development task runner

set dotenv-load := true

desktop_dir := "desktop"
desktop_tauri_manifest := "desktop/src-tauri/Cargo.toml"
web_dir := "web"
nim_core_dir := "nim/nimino_core"
nim_boundary_bin_dir := "target/nim/nimino_boundary/bin"
nim_boundary_cache_dir := "target/nim/nimino_boundary/cache"

# Opt-in mesh-llm. Off by default so `just dev`/`just staging`/`just production`
# skip ~420 extra crates + the llama.cpp native runtime build and stay fast to
# iterate on. Turn on to test mesh compute features: `just mesh=1 dev` /
# `just mesh=1 staging` / `just mesh=1 production`.
mesh := ""

# Reset only the current standalone desktop instance before launch.
# Usage: `just fresh=1 desktop-standalone`.
fresh := ""

# List all available tasks
default:
    @just --list

# ─── Dev Environment ─────────────────────────────────────────────────────────

# Install required dev tools via Hermit and create .env (safe to re-run)
bootstrap:
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    # Hermit's bin/ symlinks auto-download pinned tool versions on first use.
    # Running each tool once triggers the download if not already cached.
    echo "Ensuring toolchain via Hermit..."
    tools=(cargo nim nimble node pnpm)
    pids=()
    for tool in "${tools[@]}"; do
        "$tool" --version &
        pids+=("$!")
    done
    toolchain_failed=0
    for i in "${!pids[@]}"; do
        if ! wait "${pids[$i]}"; then
            echo "Error: failed to provision ${tools[$i]} via Hermit." >&2
            toolchain_failed=1
        fi
    done
    [[ "$toolchain_failed" -eq 0 ]] || exit 1
    if ! command -v docker &>/dev/null; then
        echo "Error: Docker is required but not installed."
        echo "Install it from https://docs.docker.com/get-docker/"
        exit 1
    fi
    if [[ ! -f .env ]]; then
        cp .env.example .env
        echo "Created .env from .env.example — review it before running just dev."
    fi

# Start Docker services, run migrations, install desktop deps
setup: bootstrap
    ./scripts/dev-setup.sh

# Install git hooks via lefthook (dispatches from the shared .git/hooks dir so all
# linked worktrees inherit the same hooks without a worktree-relative .hooks path)
hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    # Use the Hermit-pinned lefthook (bin/lefthook self-downloads on first use):
    # works with no pre-installed lefthook and guarantees the pinned version
    # rather than whatever happens to be on PATH.
    export PATH="{{justfile_directory()}}/bin:$PATH"
    # --path-format=absolute guarantees an absolute path from every invocation context:
    # without it, --git-common-dir returns ".git" from the main checkout and a
    # relative hooksPath would break linked-worktree dispatch just like .hooks did.
    HOOKS_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/hooks"
    git config --local core.hooksPath "$HOOKS_DIR"
    lefthook install --force

# Wipe development state and recreate a clean environment. Installed Nimino is preserved.
[confirm("This will DELETE all development data and preserve installed Nimino. Continue? (y/N)")]
reset:
    ./scripts/dev-reset.sh --yes

# Stop all dev services (keep data)
down:
    docker compose down

# Show dev service status
ps:
    docker compose ps

# Tail all service logs
logs *ARGS:
    docker compose logs -f {{ARGS}}

# ─── Build & Check ───────────────────────────────────────────────────────────

# Build the Rust workspace
build:
    cargo build --workspace

# Build the Rust workspace in release mode
build-release:
    cargo build --workspace --release

# Compile the Nimino core package without linking a product binary
nim-build:
    cd "{{nim_core_dir}}" && nim c --compileOnly:on --hints:off src/nimino_core.nim
    cd "{{nim_core_dir}}" && nim c --compileOnly:on --hints:off src/nimino_core_worker.nim

# Validate the Nimble manifest and type-check the Nimino core package
nim-check:
    cd "{{nim_core_dir}}" && nimble check
    cd "{{nim_core_dir}}" && nim check --hints:off src/nimino_core.nim
    cd "{{nim_core_dir}}" && nim check --hints:off src/nimino_core_worker.nim

# Run the Nimino core unit tests without building Rust
nim-test:
    cd "{{nim_core_dir}}" && for test_file in tests/test_*.nim; do nim c -r --hints:off "$test_file"; done

# Run the complete Rust-independent Nim lane
nim-ci: nim-check nim-build nim-test

# Record the warm edit-to-test loop and complete Nim lane timing
nim-baseline output="target/nim/feedback-baseline.json":
    ./scripts/measure-nim-feedback.sh "{{output}}"

# Build the production Nimino core worker without test-only operations
nim-boundary-build:
    mkdir -p "{{nim_boundary_bin_dir}}" "{{nim_boundary_cache_dir}}/production"
    cd "{{nim_core_dir}}" && nim c -d:release --hints:off --nimcache:"{{justfile_directory()}}/{{nim_boundary_cache_dir}}/production" --out:"{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker" src/nimino_core_worker.nim

# Build deterministic failure workers used only by cross-language tests
nim-boundary-test-workers:
    mkdir -p "{{nim_boundary_bin_dir}}" "{{nim_boundary_cache_dir}}/test-hooks" "{{nim_boundary_cache_dir}}/mismatch"
    cd "{{nim_core_dir}}" && nim c -d:release -d:niminoBoundaryTestHooks --hints:off --nimcache:"{{justfile_directory()}}/{{nim_boundary_cache_dir}}/test-hooks" --out:"{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker-test" src/nimino_core_worker.nim
    cd "{{nim_core_dir}}" && nim c -d:release -d:niminoBoundaryWrongSchema --hints:off --nimcache:"{{justfile_directory()}}/{{nim_boundary_cache_dir}}/mismatch" --out:"{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker-mismatch" src/nimino_core_worker.nim

# Verify checksums, ownership, and the absence of a compatibility fallback
nim-boundary-contract:
    ./scripts/test-nim-boundary-contract.sh

# Verify the Nim-owned command, JSON output, and exit-code contract
nimino-cli-contract:
    node scripts/test-nimino-cli-contract.mjs

# Verify persona precedence, trigger routing, and cancel/restart policy
nimino-agent-contract:
    node scripts/test-nimino-agent-contract.mjs

# Run Rust unit contracts and real Rust↔Nim process lifecycle scenarios
nim-boundary-test: nim-boundary-contract nim-boundary-build nim-boundary-test-workers
    cargo test -p nimino-boundary --lib --test contract
    NIMINO_BOUNDARY_WORKER="{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker-test" NIMINO_BOUNDARY_MISMATCH_WORKER="{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker-mismatch" NIMINO_BOUNDARY_PRODUCTION_WORKER="{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker" cargo test -p nimino-boundary --features test-hooks --test cross_language -- --ignored

# Measure fixed payload and recovery scenarios against pre-declared budgets
nim-boundary-benchmark output="target/nim/nimino-boundary-benchmark.json": nim-boundary-contract nim-boundary-build nim-boundary-test-workers
    cargo run --release -p nimino-boundary --features test-hooks --bin boundary-bench -- "{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker" "{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker-test" "{{output}}"

# Complete cross-language gate; the separate nim-ci lane remains Rust-free
nim-boundary-ci: nim-boundary-test nim-boundary-benchmark nimino-cluster-scenarios nimino-sync-scenarios nimino-control-scenarios nimino-effect-scenarios nimino-object-scenarios nimino-projection-scenarios

# Exercise Nim-planned content-addressed transfer, resume, pin, and GC over real Chirps
nimino-object-scenarios: nim-boundary-build
    NIMINO_BOUNDARY_WORKER="{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker" cargo test -p nimino-object-store --test chirps_replication -- --ignored --test-threads=1

# Exercise partial resume, atomic publish, drop, and query-equivalent rebuild
nimino-projection-scenarios: nim-boundary-build
    NIMINO_BOUNDARY_PRODUCTION_WORKER="{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker" cargo test -p nimino-data-ops --test projection_rebuild -- --ignored --exact --test-threads=1

# Verify the pinned Chirps dependency and its narrow Rust API boundary
chirps-contract:
    node scripts/check-chirps-api-contract.mjs

# Verify the versioned quorum/control-log model, ownership, hashes, and evidence
control-model-contract:
    node scripts/check-nimino-control-model.mjs

# Verify the pure Nim replicated-control reducer and persistence-first boundary
nimino-control-state-contract:
    node scripts/test-nimino-control-state-contract.mjs

# Verify committed lease activation, fencing, routing, and consumer ownership
nimino-lease-contract:
    node scripts/test-nimino-lease-contract.mjs

# Verify bounded community-scoped anti-entropy and durable resume ownership
nimino-sync-contract:
    node scripts/test-nimino-sync-contract.mjs

# Verify deterministic conflict, tombstone, restriction, and retention merges
nimino-convergence-contract:
    node scripts/test-nimino-convergence-contract.mjs

# Verify content-addressed manifest, fetch, pin, partial install, and GC policy
nimino-object-sync-contract:
    node scripts/test-nimino-object-sync-contract.mjs

# Verify resumable search/thread/feed rebuild ownership and atomic publication
nimino-projection-contract:
    node scripts/test-nimino-projection-contract.mjs

# Verify fenced workflow effect claims, receipts, and manual reconciliation
nimino-effect-ledger-contract:
    node scripts/test-nimino-effect-ledger-contract.mjs

# Verify quorum-selected manual repair policy and the operator adapter contract
nimino-data-ops-contract:
    node scripts/test-nimino-data-ops-contract.mjs

# Exercise backlog, capacity failure, corruption, kill, and repeat repair
nimino-data-ops-scenarios:
    cargo test -p nimino-data-ops --test convergence_scenarios -- --test-threads=1

# Verify fail-closed backup, restore, integrity, and rollback ownership
nimino-cutover-rehearsal-contract:
    node scripts/test-nimino-cutover-rehearsal.mjs

# Exercise backup/restore plus the failed-promotion rollback and emit evidence
nimino-cutover-rehearsal output="target/nim/nimino-cutover-rehearsal.json": nimino-cutover-rehearsal-contract promotion-contract
    NIMINO_CUTOVER_EVIDENCE="{{justfile_directory()}}/{{output}}" NIMINO_FAILED_PROMOTION_VERIFIED=1 cargo test -p nimino-data-ops --test convergence_scenarios backup_restore_rehearsal_preserves_inventory_and_rejects_corruption -- --exact --test-threads=1

# Verify MCP capability, audit, timeout, cancellation, and output boundaries
nimino-mcp-execution-contract:
    node scripts/test-nimino-mcp-execution-contract.mjs

# Exercise real MCP stdio framing and a capability denial
nimino-mcp-framing:
    cargo build -p nimino-dev-mcp
    node scripts/test-nimino-mcp-framing.mjs target/debug/nimino-dev-mcp

# Verify complete legacy mesh/Redis ownership and hard-cut dispositions
legacy-control-manifest-contract:
    node scripts/check-legacy-control-manifest.mjs

# Verify complete Tauri command classification and adapter-only target ownership
nimino-tauri-contract:
    node scripts/test-nimino-tauri-contract.mjs

# Verify every Rust package/module owner and reject product-policy reverse flow
rust-responsibility-contract:
    node scripts/test-rust-responsibility-contract.mjs

# Verify immutable release-set creation, rerun, mismatch, and downgrade rules
release-set-contract:
    node scripts/test-nimino-release-set.mjs

# Verify the unified Nimino relay image/chart/Compose candidate pipeline
relay-release-contract:
    node scripts/test-nimino-relay-release-contract.mjs

# Verify signed Desktop/WSL artifacts bind to one immutable release-set
platform-release-contract:
    node scripts/test-nimino-platform-release.mjs

# Verify SBOM/provenance, idempotent promotion, downgrade, and rollback gates
promotion-contract:
    node scripts/test-nimino-release-supply-chain.mjs
    node scripts/test-nimino-promotion.mjs

# Verify exact predecessor workflow/script/credential/target retirement inventory
legacy-release-deletion-contract:
    node scripts/test-nimino-legacy-release-contract.mjs

# Verify the complete Epic #2-#11 keep/delete/owner/proof readiness index
cutover-readiness-contract:
    node scripts/test-nimino-cutover-readiness.mjs

# Bind clean source, compatibility-negative, cluster, platform, and supply-chain gates
cutover-certification-contract:
    node scripts/test-nimino-cutover-certification.mjs

# Verify exact Agent/CLI bundle inventory, execution, and missing-component rejection
agent-bundle-contract:
    node scripts/test-nimino-agent-bundle.mjs

# Verify the fixed 1/3/5-node real-mesh scenario definition and ownership
nimino-cluster-scenario-contract:
    node scripts/test-nimino-cluster-scenario-contract.mjs

# Run the deterministic Chirps UDP/QUIC + Nim lifecycle scenario suite
nimino-cluster-scenarios output="target/nim/nimino-cluster-scenarios.json": nimino-cluster-scenario-contract nim-boundary-build
    NIMINO_BOUNDARY_WORKER="{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker" NIMINO_CLUSTER_EVIDENCE="{{justfile_directory()}}/{{output}}" cargo test -p nimino-chirps --test lifecycle_scenarios -- --ignored --test-threads=1

# Verify the fixed real-Chirps 3-node data synchronization definition
nimino-sync-scenario-contract:
    node scripts/test-nimino-sync-scenario-contract.mjs

# Run bounded bootstrap, durable resume, duplicate and isolation over real Chirps
nimino-sync-scenarios output="target/nim/nimino-sync-scenarios.json": nimino-sync-scenario-contract nim-boundary-build
    NIMINO_BOUNDARY_WORKER="{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker" NIMINO_SYNC_EVIDENCE="{{justfile_directory()}}/{{output}}" cargo test -p nimino-sync --test three_node -- --ignored --test-threads=1

# Run election, quorum commit, minority rejection, and durable catch-up over real Chirps
nimino-control-scenarios: nimino-control-state-contract nim-boundary-build
	NIMINO_BOUNDARY_WORKER="{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker" cargo test -p nimino-control --test three_node -- --ignored --test-threads=1

nimino-effect-scenarios: nimino-effect-ledger-contract nim-boundary-build
	NIMINO_BOUNDARY_WORKER="{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker" cargo test -p nimino-data-ops --test effect_reconcile -- --ignored --test-threads=1

# Exhaustively check the bounded 3-node control-log state graph with TLC
control-model-check: control-model-contract
    tlc -workers auto -config formal/scenarios/NiminoControlLog_3Node.cfg formal/tla/cluster/NiminoControlLog.tla

# Verify Nim-owned cluster admission, lifecycle, lane gates, and corpus coverage
nimino-cluster-contract:
    node scripts/test-nimino-cluster-contract.mjs

# Verify the canonical Nimino names and generate the legacy Buzz denylist
naming-contract:
    node scripts/check-nimino-naming-contract.mjs

runtime-namespace-contract:
    node scripts/check-nimino-runtime-namespace.mjs

# Verify the versioned Nimino wire/data classification and old-client rejection fixture
protocol-contract:
    node scripts/check-nimino-protocol-contract.mjs

# Verify GUI surface/route dispositions and feature-root ownership
gui-surface-contract:
    node scripts/check-gui-surface-contract.mjs

# Verify the single supported WSL2/WSLg configuration and negative matrix
wsl-support-contract:
    node scripts/check-wsl-support-contract.mjs

# Verify idempotent WSL service install, rollback, restart, and uninstall
wsl-service-contract:
    node scripts/test-nimino-wsl-service.mjs

# Verify typed WSL argv/PID ownership and Secret Service-only persistence
wsl-launcher-contract:
    node scripts/test-nimino-wsl-launcher-contract.mjs

# Verify WSL Chirps mTLS rotation, rebind, rejoin, and shutdown evidence wiring
wsl-chirps-contract:
    node scripts/test-nimino-wsl-chirps-contract.mjs

# Run the real UDP/QUIC certification on a WSL ext4 workspace
wsl-chirps-certify: wsl-chirps-contract
    node scripts/test-nimino-wsl-chirps-contract.mjs --certify

# Verify the manifest-pinned complete WSL install bundle and lifecycle wiring
wsl-bundle-contract:
    node scripts/test-nimino-wsl-bundle-contract.mjs

# Run clean install, update/rollback, full tool workflow, and uninstall on WSL
wsl-bundle-e2e: wsl-bundle-contract
    node scripts/test-nimino-wsl-bundle-contract.mjs --e2e

# Certify the bundle only on the exact supported WSL release candidate
wsl-bundle-certify: wsl-bundle-contract
    node scripts/test-nimino-wsl-bundle-contract.mjs --certify

# Verify complete Mobile-tree/reference classification and NIP-AB ownership
removed-client-contract:
    node scripts/check-mobile-removal-contract.mjs

# Verify path ownership and the absence of a Mobile product lane
ci-lanes-contract:
    node scripts/test-ci-lanes.mjs

# Run repo lint, formatting, and repository policy checks
check: fmt-check clippy chirps-contract control-model-contract naming-contract protocol-contract runtime-namespace-contract gui-surface-contract release-set-contract relay-release-contract platform-release-contract promotion-contract cutover-certification-contract wsl-support-contract wsl-chirps-contract wsl-bundle-contract removed-client-contract ci-lanes-contract desktop-check desktop-tauri-fmt-check desktop-tauri-clippy web-check file-size-check

# Run the active-product differential file-size ratchet and its policy tests.
# The ratchet inspects only files changed from the merge base, so this stays
# cheap enough to run unconditionally without duplicating path filters.
file-size-check:
    node --test scripts/check-file-sizes-core.test.mjs
    node desktop/scripts/check-file-sizes.mjs
    node web/scripts/check-file-sizes.mjs

# Format all Rust code
fmt:
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Run clippy with warnings as errors
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Install JS dependencies (pnpm workspace — installs all packages from root)
desktop-install:
    pnpm install

# Install JS dependencies reproducibly for CI (pnpm workspace)
desktop-install-ci:
    pnpm install --frozen-lockfile

# Run desktop lint and format checks
desktop-check:
    cd {{desktop_dir}} && pnpm check

# Fix desktop lint and format issues
desktop-fix:
    cd {{desktop_dir}} && pnpm exec biome check --write .

# Run desktop TS helper unit tests
desktop-test:
    cd {{desktop_dir}} && pnpm test

# Run desktop TypeScript checks
desktop-typecheck:
    cd {{desktop_dir}} && pnpm typecheck

# Build desktop frontend assets
desktop-build:
    cd {{desktop_dir}} && pnpm build

# Format desktop Tauri Rust code
desktop-tauri-fmt:
    cargo fmt --manifest-path {{desktop_tauri_manifest}} --all

# Check desktop Tauri Rust formatting
desktop-tauri-fmt-check:
    cargo fmt --manifest-path {{desktop_tauri_manifest}} --all -- --check

# Format all code (Rust + Tauri Rust + Dart)
fmt-all: fmt desktop-tauri-fmt

# Fix all formatting and lint issues
fix-all: fmt desktop-tauri-fmt desktop-fix web-fix

# Ensure sidecar placeholder binaries exist (Tauri validates externalBin at compile time)
# Sidecar binary list must stay in sync with desktop-release-build below.
_ensure-sidecar-stubs:
    #!/usr/bin/env bash
    set -euo pipefail
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    mkdir -p desktop/src-tauri/binaries
    SIDECARS=(nimino-acp nimino-agent nimino-dev-mcp git-credential-nostr nimino)
    if [[ "$TARGET" != *windows* ]]; then
        SIDECARS+=(nimino-backend-kubernetes)
    fi
    for bin in "${SIDECARS[@]}"; do
        touch "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done

# Ensure the Docker dev database is running and healthy.
_ensure-services:
    #!/usr/bin/env bash
    set -euo pipefail
    pg=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' nimino-postgres 2>/dev/null || echo "not_found")
    if [[ "$pg" == "healthy" ]]; then
        echo "Services already healthy"
        exit 0
    fi
    echo "Starting services..."
    docker compose up -d || true
    echo -n "Waiting for services"
    for i in $(seq 1 40); do
        pg=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' nimino-postgres 2>/dev/null || echo "not_found")
        if [[ "$pg" == "healthy" ]]; then
            echo " ready"
            exit 0
        fi
        echo -n "."
        sleep 3
    done
    echo " timed out"
    exit 1

# Apply database migrations and seed the local dev community if the dev database is running
_ensure-migrations: _ensure-services
    cargo run -p nimino-admin -- migrate
    ./scripts/seed-local-community.sh

# Run clippy on the desktop Tauri Rust crate
desktop-tauri-clippy: _ensure-sidecar-stubs
    cargo clippy --manifest-path {{desktop_tauri_manifest}} --workspace --all-targets -- -D warnings

# Check the desktop Tauri Rust crate compiles
desktop-tauri-check: _ensure-sidecar-stubs
    cargo check --manifest-path {{desktop_tauri_manifest}}

# Run desktop Tauri Rust unit tests
desktop-tauri-test: _ensure-sidecar-stubs
    cd desktop/src-tauri && cargo test --workspace

# Exercise the retained WebSocket, unread projection, and terminal hot paths
nimino-tauri-native-test: _ensure-sidecar-stubs
    cargo test --manifest-path {{desktop_tauri_manifest}} native_websocket::tests:: -- --test-threads=1
    cargo test --manifest-path {{desktop_tauri_manifest}} observed_unread::tests:: -- --test-threads=1
    cargo test --manifest-path {{desktop_tauri_manifest}} terminal_runtime::tests:: -- --test-threads=1

# Run the native terminal latency gate explicitly on a known-idle host.
# This is intentionally excluded from shared CI: scheduler contention makes a
# wall-clock assertion flaky, and the release profile is the shipped shape.
desktop-terminal-performance-test:
    cargo test --manifest-path desktop/src-tauri/crates/nimino-terminal/Cargo.toml --release --test latency g3_renderer_acquire_stays_within_frame_budget -- --ignored --exact --nocapture

# Verify compiled-flag behavior under both compile states (clean + capability set).
# Runs the auto-connect and owner-only access focused tests twice with
# independently supplied expected values; build.rs rerun-if-env-changed
# triggers recompilation.
desktop-tauri-test-compiled-flags: _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    cd desktop/src-tauri
    echo "=== Clean build (no flag) → expect false ==="
    env -u NIMINO_BUILD_AUTO_CONNECT_DEFAULT_RELAY \
      NIMINO_TEST_EXPECTED_AUTO_CONNECT_DEFAULT_RELAY=false \
      cargo test compiled_flag_matches_expected -- --ignored --nocapture
    env -u NIMINO_BUILD_AGENT_ACCESS_OWNER_ONLY \
      NIMINO_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY=false \
      cargo test --lib
    env -u NIMINO_BUILD_AGENT_ACCESS_OWNER_ONLY \
      NIMINO_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY=false \
      cargo test compiled_policy_matches_expected -- --ignored --nocapture
    echo "=== Internal build (flags set) → expect true ==="
    NIMINO_BUILD_AUTO_CONNECT_DEFAULT_RELAY=1 \
      NIMINO_TEST_EXPECTED_AUTO_CONNECT_DEFAULT_RELAY=true \
      cargo test compiled_flag_matches_expected -- --ignored --nocapture
    NIMINO_BUILD_AGENT_ACCESS_OWNER_ONLY=1 \
      NIMINO_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY=true \
      cargo test --lib
    NIMINO_BUILD_AGENT_ACCESS_OWNER_ONLY=1 \
      NIMINO_TEST_EXPECTED_AGENT_ACCESS_OWNER_ONLY=true \
      cargo test compiled_policy_matches_expected -- --ignored --nocapture
    echo "Both compiled states verified."

# Build the full desktop Tauri app locally (unsigned, for testing)
# Sidecar binary list must stay in sync with _ensure-sidecar-stubs above.
# pnpm install is unconditional here: release builds must start from a clean dep tree.
desktop-release-build target="aarch64-apple-darwin":
    #!/usr/bin/env bash
    set -euo pipefail
    TARGET={{target}}
    mkdir -p desktop/src-tauri/binaries
    touch "desktop/src-tauri/binaries/nimino-acp-$TARGET"
    touch "desktop/src-tauri/binaries/nimino-agent-$TARGET"
    if [[ "$TARGET" != *windows* ]]; then
        touch "desktop/src-tauri/binaries/nimino-backend-kubernetes-$TARGET"
    fi
    touch "desktop/src-tauri/binaries/nimino-dev-mcp-$TARGET"
    touch "desktop/src-tauri/binaries/git-credential-nostr-$TARGET"
    touch "desktop/src-tauri/binaries/nimino-$TARGET"
    pnpm install
    cd {{desktop_dir}} && pnpm tauri build --features mesh-llm --target {{target}}

# Run desktop checks suitable for CI / pre-push
desktop-ci: desktop-check desktop-test desktop-tauri-fmt-check desktop-build desktop-tauri-check desktop-tauri-test

# Seed deterministic channel data for desktop Playwright tests
desktop-e2e-seed: _ensure-migrations
    ./scripts/setup-desktop-test-data.sh

# Run desktop browser smoke tests
desktop-e2e-smoke:
    cd {{desktop_dir}} && pnpm test:e2e:smoke

# Run desktop relay-backed e2e tests
desktop-e2e-integration: _ensure-migrations
    cd {{desktop_dir}} && pnpm test:e2e:integration

# Run the deterministic desktop correctness smoke against an isolated local relay
desktop-release-smoke:
    ./scripts/run-desktop-release-smoke.sh

# Run only the e2e specs changed vs origin/main (both projects) before pushing
desktop-e2e-pre-push: _ensure-migrations
    git fetch origin main
    cd {{desktop_dir}} && pnpm build:e2e && pnpm exec playwright test --only-changed=origin/main

# Run all checks suitable for CI / pre-push (no infra needed)
ci: check test-unit nim-ci nim-boundary-ci desktop-test desktop-build desktop-tauri-check desktop-tauri-test web-build

# ─── Test ─────────────────────────────────────────────────────────────────────

# Run all tests (unit + integration)
test:
    ./scripts/run-tests.sh all

# Run unit tests only (no infra needed)
test-unit:
    #!/usr/bin/env bash
    set -euo pipefail
    if command -v cargo-nextest &>/dev/null; then
        cargo nextest run -p nimino-core -p nimino-auth --lib
        cargo nextest run -p nimino-voice --lib
        cargo nextest run -p nimino-cli
        # nimino-db migrator/lint tests: pure SQL-parsing unit tests (no infra).
        # They guard the embedded-migrator invariant (exactly the consolidated
        # 0001; cutover/backfill stays an operator script, not startup state)
        # and the tenant-scoping lints. The Postgres-backed nimino-db tests are
        # #[ignore]d, so --lib runs only the infra-free set. Without this gate a
        # stray file in migrations/ or a broken lint ships green.
        cargo nextest run -p nimino-db --lib
        # Multi-tenant conformance gate (nimino-conformance): the independent
        # replay checker + golden fixtures. No infra — pure in-process trace
        # replay — so it belongs in the unit job. Run all targets (lib + the
        # tests/replay_fixtures.rs integration test), not just --lib.
        cargo nextest run -p nimino-conformance
        # Kubernetes backend provider: the decision layers (state machine, GC
        # planner, env precedence, naming, wire) are pure functions with a fake
        # substrate, so they belong in the unit job. Enumerated explicitly
        # because nothing in CI runs `cargo test --workspace` — workspace
        # membership alone buys clippy/check, not a single executed test.
        cargo nextest run -p nimino-backend-kubernetes
        cargo nextest run -p nimino-store
        cargo nextest run -p nimino-object-store
        cargo nextest run -p nimino-data-ops
        cargo nextest run -p nimino-wsl-launcher
        cargo nextest run -p nimino-dev-mcp
        # nimino-agent model-capabilities corpus: the Rust half of the
        # cross-language drift guard. `model_capabilities.rs` embeds
        # scripts/model-capabilities.json + scripts/normative-corpus.json via
        # include_str! and replays the full locked corpus as pure in-process tests (no
        # infra). Enumerated explicitly because nothing in CI runs
        # `cargo test --workspace`; without this step a manifest edit that
        # diverges Rust from the corpus ships green.
        cargo nextest run -p nimino-agent --lib
        cargo build -p nimino-dev-mcp
        node scripts/test-nimino-mcp-framing.mjs target/debug/nimino-dev-mcp
    else
        ./scripts/run-tests.sh unit
    fi

# Run integration tests only (starts services if needed)
test-integration:
    ./scripts/run-tests.sh integration

# Regenerate the model-capability normative corpus from the production Rust
# resolver. The corpus is a golden snapshot, never hand-edited: this runs the
# `#[ignore]`d writer test in nimino-agent, which serializes `resolve()` over the
# inputs-only question table to scripts/normative-corpus.json. Run this after
# any model-capabilities.json edit, then commit the regenerated file. The
# `corpus_matches_generated_snapshot` gate fails CI if the committed file drifts.
regen-model-corpus:
    cargo test -p nimino-agent --lib model_capabilities::tests::regen_corpus_file -- --ignored --exact

# Nimino shared compute e2e: current desktop discovery/admission logic and
# Playwright UI coverage.
mesh-e2e:
    cargo test --manifest-path {{desktop_dir}}/src-tauri/Cargo.toml --features mesh-llm mesh_llm --lib
    cd {{desktop_dir}} && pnpm test:e2e:smoke -- mesh-compute.spec.ts

# Reset only development state, seed deterministic local channels, and launch
# the mesh-enabled desktop with the repository's public Tyler test identity.
# This is for local verification only; never point this identity at staging/prod.
[confirm("This will reset development data, preserve installed Nimino, then launch a seeded mesh dev app. Continue? (y/N)")]
mesh-dev-fresh:
    #!/usr/bin/env bash
    set -euo pipefail
    ./scripts/dev-reset.sh --yes
    ./scripts/setup-desktop-test-data.sh
    export NIMINO_PRIVATE_KEY="3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03"
    export NIMINO_REQUIRE_RELAY_MEMBERSHIP=true
    export NIMINO_ALLOW_NIP_OA_AUTH=true
    export RELAY_OWNER_PUBKEY="e5ebc6cdb579be112e336cc319b5989b4bb6af11786ea90dbe52b5f08d741b34"
    export NIMINO_RELAY_PRIVATE_KEY="0000000000000000000000000000000000000000000000000000000000000001"
    export NIMINO_RECONCILE_CHANNELS=true
    export NIMINO_RESET_WEBVIEW_STATE=1
    exec just mesh=1 dev

# Take desktop screenshots using the mock bridge
desktop-screenshot *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    pnpm -C {{desktop_dir}} build:e2e
    cd {{desktop_dir}}
    if ! curl -sf http://127.0.0.1:4173/ >/dev/null 2>&1; then
        python3 -m http.server 4173 -d dist >/dev/null 2>&1 &
        trap "kill $! 2>/dev/null || true" EXIT
        for i in $(seq 1 20); do curl -sf http://127.0.0.1:4173/ >/dev/null && break; sleep 0.5; done
    fi
    node tests/helpers/screenshot.mjs {{ARGS}}

# ─── Run ──────────────────────────────────────────────────────────────────────

# Start the relay server (auto-starts Docker services if needed)
_dev-cluster-material:
    #!/usr/bin/env bash
    set -euo pipefail
    root="{{justfile_directory()}}/target/nim/dev-cluster"
    mkdir -p "$root"
    if [[ ! -s "$root/tls.crt" || ! -s "$root/tls.key" ]]; then
        openssl req -x509 -newkey rsa:2048 -nodes -days 7 -subj '/CN=nimino-dev-chirps' \
            -keyout "$root/key.pem" -out "$root/cert.pem" >/dev/null 2>&1
        openssl x509 -in "$root/cert.pem" -outform DER -out "$root/tls.crt"
        openssl pkcs8 -topk8 -nocrypt -in "$root/key.pem" -outform DER -out "$root/tls.key"
        cp "$root/tls.crt" "$root/ca.crt"
        chmod 600 "$root/tls.key"
    fi

relay: bootstrap _ensure-migrations nim-boundary-build _dev-cluster-material
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    root="{{justfile_directory()}}/target/nim/dev-cluster"
    NIMINO_BOUNDARY_WORKER="{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker" \
    NIMINO_CHIRPS_IDENTITY_PATH="$root/node.identity" \
    NIMINO_CHIRPS_CERTIFICATE_PATH="$root/tls.crt" \
    NIMINO_CHIRPS_PRIVATE_KEY_PATH="$root/tls.key" \
    NIMINO_CHIRPS_TRUST_ANCHOR_PATHS="$root/ca.crt" \
    NIMINO_NODE_STORE_PATH="$root/data.redb" cargo run -p nimino-relay

# Start the relay with the built web UI served from it
relay-web: bootstrap _ensure-migrations nim-boundary-build _dev-cluster-material
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    [[ -d node_modules ]] || pnpm install
    pnpm -C web build
    root="{{justfile_directory()}}/target/nim/dev-cluster"
    NIMINO_WEB_DIR=./web/dist \
    NIMINO_BOUNDARY_WORKER="{{justfile_directory()}}/{{nim_boundary_bin_dir}}/nimino-core-worker" \
    NIMINO_CHIRPS_IDENTITY_PATH="$root/node.identity" \
    NIMINO_CHIRPS_CERTIFICATE_PATH="$root/tls.crt" \
    NIMINO_CHIRPS_PRIVATE_KEY_PATH="$root/tls.key" \
    NIMINO_CHIRPS_TRUST_ANCHOR_PATHS="$root/ca.crt" \
    NIMINO_NODE_STORE_PATH="$root/data.redb" cargo run -p nimino-relay

# Build and run the private read-only admin dashboard
admin: bootstrap _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    [[ -d node_modules ]] || pnpm install
    pnpm -C admin-web build
    export NIMINO_ADMIN_HOST="${NIMINO_ADMIN_HOST:-admin.localhost:3000}"
    export NIMINO_ADMIN_WEB_DIR="${NIMINO_ADMIN_WEB_DIR:-{{justfile_directory()}}/admin-web/dist}"
    echo "Admin dashboard: http://${NIMINO_ADMIN_HOST}/reports"
    cargo run -p nimino-relay

# Seed deterministic reports and product feedback for local admin dashboard review
admin-seed: _ensure-migrations
    ./scripts/seed-admin-dashboard.sh

# Run focused relay and browser checks for the read-only admin dashboard
admin-check: fmt-check
    cargo check -p nimino-relay --all-targets
    cargo test -p nimino-relay api::admin
    cargo test -p nimino-relay router::tests
    pnpm -C admin-web check
    pnpm -C admin-web exec playwright test

# Start the relay server in release mode
relay-release: _ensure-migrations
    cargo run -p nimino-relay --release


# Run the desktop Tauri app in dev mode with a local relay (ports and identity derived from worktree)
dev *ARGS: bootstrap _ensure-sidecar-stubs _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    bind_addr="${NIMINO_BIND_ADDR:-0.0.0.0:3000}"
    relay_port="${bind_addr##*:}"; [[ -n "$relay_port" ]] || relay_port=3000
    health_port="${NIMINO_HEALTH_PORT:-8080}"
    metrics_port="${NIMINO_METRICS_PORT:-9102}"
    if command -v lsof >/dev/null 2>&1; then
        for spec in "relay:$relay_port" "health:$health_port" "metrics:$metrics_port"; do
            name="${spec%%:*}"; port="${spec##*:}"
            if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
                echo "Error: $name port $port is already in use; refusing to launch desktop against a stale relay." >&2
                lsof -nP -iTCP:"$port" -sTCP:LISTEN >&2 || true
                echo "Stop the process above (often a stale nimino-relay) and rerun: just dev" >&2
                exit 1
            fi
        done
    fi
    cargo build -p nimino-acp -p nimino-agent -p nimino-backend-kubernetes -p nimino-dev-mcp -p nimino-cli -p git-credential-nostr -p nimino-relay
    # Docker Desktop's forwarded MinIO port can stall under the deployment
    # probe's 32 concurrent writers. Keep the gate enabled in local dev, using
    # the bounded profile already used by the relay test launcher.
    export NIMINO_GIT_PROBE_WRITERS="${NIMINO_GIT_PROBE_WRITERS:-8}"
    export NIMINO_GIT_PROBE_ROUNDS="${NIMINO_GIT_PROBE_ROUNDS:-2}"
    ./target/debug/nimino-relay &
    RELAY_PID=$!
    cleanup() {
        [[ -n "${INSTANCE_ID:-}" ]] && ../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true
        kill "$RELAY_PID" 2>/dev/null || true
    }
    trap cleanup EXIT
    relay_ready=false
    for _ in $(seq 1 120); do
        if ! kill -0 "$RELAY_PID" 2>/dev/null; then
            echo "Error: nimino-relay exited during startup; refusing to launch desktop." >&2
            wait "$RELAY_PID" || true
            exit 1
        fi
        if curl --silent --fail --max-time 1 "http://127.0.0.1:${health_port}/_readiness" >/dev/null; then
            relay_ready=true
            break
        fi
        sleep 0.5
    done
    if [[ "$relay_ready" != true ]]; then
        echo "Error: nimino-relay did not become healthy within 60 seconds; refusing to launch desktop." >&2
        exit 1
    fi
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    source ../scripts/instance-env.sh
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.NIMINO_TAURI_CONFIG).identifier)")
    echo "Starting on Vite port ${NIMINO_VITE_PORT}, relay ${NIMINO_RELAY_URL}"
    FEATURES=(); [[ -n "{{mesh}}" ]] && FEATURES=(--features mesh-llm)
    pnpm exec tauri dev ${FEATURES[@]+"${FEATURES[@]}"} --config "$NIMINO_TAURI_CONFIG" {{ARGS}}

# Run only the desktop app. No relay, database, Docker, migrations, or .env are needed.
# The app opens normally and asks for a community before making a relay connection.
desktop-standalone *ARGS: _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    cargo build -p nimino-acp -p nimino-agent -p nimino-backend-kubernetes -p nimino-dev-mcp -p nimino-cli -p git-credential-nostr
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")
    for bin in nimino-acp nimino-agent nimino-backend-kubernetes nimino-dev-mcp git-credential-nostr nimino; do
        cp "${TARGET_DIR}/debug/${bin}" "desktop/src-tauri/binaries/${bin}-${TARGET}"
        chmod +x "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    unset NIMINO_PRIVATE_KEY NIMINO_SHARE_IDENTITY
    if [[ -n "{{fresh}}" ]]; then
        export NIMINO_RESET_WEBVIEW_STATE=1
    fi
    source ../scripts/instance-env.sh
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.NIMINO_TAURI_CONFIG).identifier)")
    export NIMINO_DEV_KEYRING_SERVICE="nimino-desktop-dev.${NIMINO_INSTANCE_SLUG:-main}"
    if [[ -n "{{fresh}}" ]]; then
        ../scripts/reset-desktop-standalone-state.sh "$INSTANCE_ID" "$NIMINO_DEV_KEYRING_SERVICE"
    fi
    trap '../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true' EXIT
    echo "Starting standalone desktop on Vite port ${NIMINO_VITE_PORT}; no relay services were started"
    pnpm exec tauri dev --config "$NIMINO_TAURI_CONFIG" {{ARGS}}

# Run the desktop app against the internal staging relay (installs deps + builds agent tools automatically)
staging *ARGS: bootstrap _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    pnpm install  # unconditional: staging must always start with a clean dep tree
    cargo build --release -p nimino-acp -p nimino-agent -p nimino-backend-kubernetes -p nimino-dev-mcp -p nimino-cli -p git-credential-nostr
    FEATURES=()
    if [[ -n "{{mesh}}" ]]; then
        FEATURES=(--features mesh-llm)
    fi
    # Replace 0-byte sidecar stubs with real binaries so tauri dev picks them up.
    # nimino: the CLI sidecar. nimino-backend-kubernetes: provider discovery scans the
    # exe dir for executable nimino-backend-* files, so the non-executable stub that
    # tauri dev copies next to the exe would hide the provider from "Run on".
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")
    STAGING_SIDECARS=(nimino)
    if [[ "$TARGET" != *windows* ]]; then
        STAGING_SIDECARS+=(nimino-backend-kubernetes)
    fi
    for bin in "${STAGING_SIDECARS[@]}"; do
        cp "${TARGET_DIR}/release/${bin}" "desktop/src-tauri/binaries/${bin}-${TARGET}"
        chmod +x "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done
    cd {{desktop_dir}}
    export NIMINO_RELAY_URL="wss://sprout-oss.stage.blox.sqprod.co"
    source ../scripts/instance-env.sh
    # Ctrl+C kills the Tauri app before its in-process sweep finishes, leaking
    # agent workers. Reap this instance's agents on exit as a backstop.
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.NIMINO_TAURI_CONFIG).identifier)")
    trap '../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true' EXIT
    echo "Starting staging on Vite port ${NIMINO_VITE_PORT}, relay ${NIMINO_RELAY_URL}"
    pnpm exec tauri dev ${FEATURES[@]+"${FEATURES[@]}"} --config "$NIMINO_TAURI_CONFIG" {{ARGS}}

# Run the desktop app against the production relay (installs deps + builds agent tools automatically)
production *ARGS: bootstrap _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    pnpm install  # unconditional: production must always start with a clean dep tree
    cargo build --release -p nimino-acp -p nimino-agent -p nimino-backend-kubernetes -p nimino-dev-mcp -p nimino-cli -p git-credential-nostr
    FEATURES=()
    if [[ -n "{{mesh}}" ]]; then
        FEATURES=(--features mesh-llm)
    fi
    # Replace 0-byte sidecar stubs with real binaries so tauri dev picks them up.
    # nimino: the CLI sidecar. nimino-backend-kubernetes: provider discovery scans the
    # exe dir for executable nimino-backend-* files, so the non-executable stub that
    # tauri dev copies next to the exe would hide the provider from "Run on".
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")
    PRODUCTION_SIDECARS=(nimino)
    if [[ "$TARGET" != *windows* ]]; then
        PRODUCTION_SIDECARS+=(nimino-backend-kubernetes)
    fi
    for bin in "${PRODUCTION_SIDECARS[@]}"; do
        cp "${TARGET_DIR}/release/${bin}" "desktop/src-tauri/binaries/${bin}-${TARGET}"
        chmod +x "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done
    cd {{desktop_dir}}
    export NIMINO_RELAY_URL="wss://nimino.block.builderlab.xyz"
    source ../scripts/instance-env.sh
    # Ctrl+C kills the Tauri app before its in-process sweep finishes, leaking
    # agent workers. Reap this instance's agents on exit as a backstop.
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.NIMINO_TAURI_CONFIG).identifier)")
    trap '../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true' EXIT
    echo "Starting production on Vite port ${NIMINO_VITE_PORT}, relay ${NIMINO_RELAY_URL}"
    pnpm exec tauri dev ${FEATURES[@]+"${FEATURES[@]}"} --config "$NIMINO_TAURI_CONFIG" {{ARGS}}

# Run the desktop frontend dev server (port derived from worktree)
desktop-dev:
    #!/usr/bin/env bash
    set -euo pipefail
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    source ../scripts/instance-env.sh
    echo "Starting frontend dev server on Vite port ${NIMINO_VITE_PORT}, relay ${NIMINO_RELAY_URL}"
    pnpm exec vite --port "${NIMINO_VITE_PORT}" --strictPort

# ─── Web ─────────────────────────────────────────────────────────────────────

# Run the web frontend dev server (port derived from worktree to avoid collisions)
web:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ -d node_modules ]] || pnpm install
    source scripts/instance-env.sh
    export VITE_PORT=$((NIMINO_VITE_PORT + 100))
    export VITE_RELAY_URL="${NIMINO_RELAY_URL}"
    echo "Starting web dev server on port ${VITE_PORT}, relay ${NIMINO_RELAY_URL}"
    cd {{web_dir}}
    pnpm exec vite --port "${VITE_PORT}" --strictPort

# Run web lint and format checks
web-check:
    cd {{web_dir}} && pnpm check

# Fix web lint and format issues
web-fix:
    cd {{web_dir}} && pnpm exec biome check --write .

# Run web TypeScript checks
web-typecheck:
    cd {{web_dir}} && pnpm typecheck

# Build web frontend assets
web-build:
    cd {{web_dir}} && pnpm build

# Run web browser smoke tests
web-e2e-smoke:
    cd {{web_dir}} && pnpm test:e2e:smoke

# ─── Database ─────────────────────────────────────────────────────────────────

# Apply database migrations
migrate: _ensure-migrations

# ─── Utilities ────────────────────────────────────────────────────────────────

# Remove build artifacts
clean:
    cargo clean
    cargo clean --manifest-path desktop/src-tauri/Cargo.toml

# Check the Rust workspace compiles without producing binaries
check-compile:
    cargo check --workspace --all-targets

# ─── Release ─────────────────────────────────────────────────────────────────

# Release candidates are created only by the immutable Nimino workflows.

# ─── Agent Harness ────────────────────────────────────────────────────────────

# Run a goose agent connected to a Nimino relay (foreground)
goose relay="ws://localhost:3000" agents="1" heartbeat="0" prompt="" key="$NIMINO_PRIVATE_KEY":
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    source ./scripts/_goose-env.sh "{{relay}}" "{{key}}" "{{agents}}" "{{heartbeat}}" "{{prompt}}"
    exec env "${env_args[@]}" ./target/release/nimino-acp

# Run a goose agent in the background (screen session named 'goose-agent-N')
goose-bg relay="ws://localhost:3000" agents="1" heartbeat="0" prompt="" key="$NIMINO_PRIVATE_KEY":
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    source ./scripts/_goose-env.sh "{{relay}}" "{{key}}" "{{agents}}" "{{heartbeat}}" "{{prompt}}"
    screen -dmS goose-agent-{{agents}} bash -c "$(printf '%q ' env "${env_args[@]}") ./target/release/nimino-acp"
    echo "Agent running in screen session 'goose-agent-{{agents}}'. Attach with: screen -r goose-agent-{{agents}}"

# ─── Benchmarking ─────────────────────────────────────────────────────────────

# Run the Nimino orchestra benchmark — leaderboard-eligible by default (TB 2.1, k=5, Sonnet+Haiku). Stands up its own Docker stack; --gui opens a live spectator desktop app; other flags pass to benchmark.py (--dataset/--path, --include-task, --attempts, --manifest, --dry-run, ...)
benchmark *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    uv run --project benchmarks/nimino-orchestra/testbed \
        benchmarks/nimino-orchestra/scripts/benchmark.py {{ARGS}}

# Run the benchmark adapter + testbed gate exactly as CI does (pytest + ruff, pinned ruff from pyproject)
benchmark-check:
    #!/usr/bin/env bash
    set -euo pipefail
    cd "{{justfile_directory()}}/benchmarks/nimino-orchestra"
    # CI installs the dev extra with pip, so pyproject — not uv.lock — decides
    # which ruff lints. Read the pin from there so this recipe cannot drift
    # from the workflow (a floating specifier once meant CI failed on RUF100
    # while the locked local ruff passed).
    ruff_pin="$(grep -oE 'ruff==[0-9.]+' pyproject.toml | head -1 | cut -d= -f3)"
    for project in . testbed; do
        (
            cd "$project"
            echo "── nimino-orchestra/$project (ruff $ruff_pin)"
            uv run --frozen pytest -q
            uvx "ruff@$ruff_pin" check .
            uvx "ruff@$ruff_pin" format --check .
        )
    done
    # The task verifiers live in the sibling benchmarks/nimino-dataset, so they
    # need the harness config passed explicitly to stay linted.
    echo "── nimino-dataset (ruff $ruff_pin)"
    uvx "ruff@$ruff_pin" check --config pyproject.toml ../nimino-dataset
    uvx "ruff@$ruff_pin" format --check --config pyproject.toml ../nimino-dataset

# Stop the benchmark Docker stack (state and channels are kept)
benchmark-down:
    docker compose --project-name nimino-benchmark down
