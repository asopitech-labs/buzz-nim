#!/usr/bin/env bash
# =============================================================================
# start-isolated-test-relay.sh — GUI read-model overhaul test harness (Dawn)
# =============================================================================
# Stands up a FULLY ISOLATED relay for seeding + parity/perf runs, from source
# on the current branch. Never touches the shared :3000 team relay or the
# default `nimino-*` dev stack. Backing services run under the dedicated
# `nimino-harness` Compose project (docker-compose.harness.yml); the relay runs
# in the foreground on override ports.
#
#   Topology (reuse this exact tuple for desktop parity runs):
#     compose project : nimino-harness
#     postgres        : localhost:5471  (db=nimino, user=nimino, pass=nimino_dev)
#     minio           : localhost:9471 (console 9472)
#     relay main      : localhost:3030   ← NIMINO_E2E_RELAY_URL=http://localhost:3030
#     relay health    : localhost:8088
#     relay metrics   : localhost:9202
#
# Usage:
#   ./scripts/start-isolated-test-relay.sh [--profile <cargo-profile>]
#
# Teardown (safe — scoped to our project only):
#   docker compose -p nimino-harness -f docker-compose.harness.yml down -v
# =============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${REPO_ROOT}"

CARGO_PROFILE="${CARGO_PROFILE:-ci}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --profile) CARGO_PROFILE="$2"; shift 2 ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

# Cargo names the development profile `dev`, but writes its binaries under
# target/debug. Accept `debug` as the user-facing spelling too.
case "${CARGO_PROFILE}" in
  dev|debug)
    CARGO_BUILD_PROFILE="dev"
    CARGO_TARGET_PROFILE="debug"
    ;;
  *)
    CARGO_BUILD_PROFILE="${CARGO_PROFILE}"
    CARGO_TARGET_PROFILE="${CARGO_PROFILE}"
    ;;
esac

PROJECT="nimino-harness"
COMPOSE_FILE="docker-compose.harness.yml"

# Isolated ports (distinct from :3000 team relay, default dev stack, and Eva's
# evaperf :5470/:6470/:9470/:3170 stack).
PG_PORT=5471
MINIO_PORT=9471
RELAY_MAIN=3030
RELAY_HEALTH=8088
RELAY_METRICS=9202
CHIRPS_PORT=7473
COMMUNITY_HOST="localhost:${RELAY_MAIN}"

BLUE='\033[0;34m'; GREEN='\033[0;32m'; RED='\033[0;31m'; NC='\033[0m'
log() { echo -e "${BLUE}[isolated-relay]${NC} $*"; }
ok()  { echo -e "${GREEN}[isolated-relay]${NC} $*"; }
err() { echo -e "${RED}[isolated-relay]${NC} $*" >&2; }

# ── Backing services (scoped to nimino-harness only) ───────────────────────────
log "Bringing up backing services (project=${PROJECT})..."
docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" up -d

wait_pg() {
  for _ in $(seq 1 60); do
    if docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" exec -T postgres \
         pg_isready -U nimino >/dev/null 2>&1; then
      ok "Postgres ready"; return 0
    fi
    sleep 2
  done
  err "Postgres did not become ready"; return 1
}
wait_pg

# ── Schema + partitions ──────────────────────────────────────────────────────
export PGPASSWORD=nimino_dev
psql_h() { docker compose -p "${PROJECT}" -f "${COMPOSE_FILE}" exec -T postgres \
  psql -U nimino -d nimino -v ON_ERROR_STOP=1 "$@"; }

log "Resetting isolated database and applying schema..."
# This database belongs only to the nimino-harness Compose project. Reset it on
# every launch so stale partitions/events from an earlier proof cannot alter
# schema planning or test results.
psql_h -c "DROP SCHEMA public CASCADE; CREATE SCHEMA public;"
export PGSCHEMA_PLAN_HOST=localhost PGSCHEMA_PLAN_PORT=${PG_PORT}
export PGSCHEMA_PLAN_DB=nimino PGSCHEMA_PLAN_USER=nimino PGSCHEMA_PLAN_PASSWORD=nimino_dev
export PGHOST=localhost PGPORT=${PG_PORT} PGUSER=nimino PGDATABASE=nimino
./bin/pgschema apply --file schema/schema.sql --auto-approve
psql_h < scripts/attach-schema-partitions.sql
ok "Schema applied"

# ── Deployment community + channels + members ────────────────────────────────
# setup-desktop-test-data.sh is the single writer of the dev community row and
# the channel/member seed. It keys everything off a fixed COMMUNITY_ID and an
# overridable host — point that host at OUR relay so the tenant binding matches,
# and point its DB env at OUR isolated postgres. (psql is on PATH, so it uses
# NIMINO_DB_HOST/PORT rather than the shared `nimino-postgres` container.)
log "Seeding community (host=${COMMUNITY_HOST}), channels, and members..."
NIMINO_COMMUNITY_HOST="${COMMUNITY_HOST}" \
  NIMINO_DB_HOST=localhost NIMINO_DB_PORT=${PG_PORT} NIMINO_DB_USER=nimino \
  NIMINO_DB_PASS=nimino_dev NIMINO_DB_NAME=nimino \
  NIMINO_DB_DOCKER_CONTAINER="${PROJECT}-postgres-1" \
  ./scripts/setup-desktop-test-data.sh
ok "Community + channels + members seeded"

# ── Build relay from source (current branch) ─────────────────────────────────
# The repo pins Rust via rust-toolchain.toml (1.95.0). Outside the hermit env a
# stray Homebrew `cargo` (1.89) shadows the pin and fails on sqlx's MSRV, so
# prefer the rustup shim, which honors the pin.
if [[ -x "${HOME}/.cargo/bin/cargo" ]]; then
  export PATH="${HOME}/.cargo/bin:${PATH}"
fi
log "Building relay (profile=${CARGO_BUILD_PROFILE}, cargo=$(command -v cargo), $(cargo --version))..."
cargo build --profile "${CARGO_BUILD_PROFILE}" -p nimino-relay
just nim-boundary-build _dev-cluster-material
ok "Relay built"

# ── Run relay (detached tmux session) ────────────────────────────────────────
# Run inside tmux, NOT the foreground: this script is invoked from ephemeral
# shells whose process group is reaped on return, which SIGTERMs a foreground
# relay ~seconds after startup. tmux fully daemonizes the session so the relay
# survives (same pattern the perf stack uses). Logs to ${RELAY_LOG}.
RELAY_LOG="${RELAY_LOG:-/tmp/dawn-relay-run.log}"
TMUX_SESSION="${TMUX_SESSION:-dawn-relay}"
CLUSTER_DIR="${REPO_ROOT}/target/nim/isolated-relay"
CHIRPS_DIR="${REPO_ROOT}/target/nim/dev-cluster"
mkdir -p "${CLUSTER_DIR}"
tmux kill-session -t "${TMUX_SESSION}" 2>/dev/null || true
if command -v lsof >/dev/null 2>&1 && lsof -nP -iTCP:"${RELAY_MAIN}" -sTCP:LISTEN >/dev/null 2>&1; then
  err "Port ${RELAY_MAIN} is already in use; refusing to report a stale relay as this harness."
  lsof -nP -iTCP:"${RELAY_MAIN}" -sTCP:LISTEN >&2 || true
  exit 1
fi
log "Starting relay in tmux session '${TMUX_SESSION}' on :${RELAY_MAIN} (health :${RELAY_HEALTH}, metrics :${RELAY_METRICS})..."
tmux new-session -d -s "${TMUX_SESSION}" "cd '${REPO_ROOT}' && env \
  DATABASE_URL=postgres://nimino:nimino_dev@localhost:${PG_PORT}/nimino \
  RELAY_URL=ws://localhost:${RELAY_MAIN} \
  NIMINO_BIND_ADDR=0.0.0.0:${RELAY_MAIN} \
  NIMINO_HEALTH_PORT=${RELAY_HEALTH} \
  NIMINO_METRICS_PORT=${RELAY_METRICS} \
  NIMINO_BOUNDARY_WORKER='${REPO_ROOT}/target/nim/nimino_boundary/bin/nimino-core-worker' \
  NIMINO_CHIRPS_BIND_ADDR=127.0.0.1:${CHIRPS_PORT} \
  NIMINO_CHIRPS_IDENTITY_PATH='${CLUSTER_DIR}/node.identity' \
  NIMINO_CHIRPS_CERTIFICATE_PATH='${CHIRPS_DIR}/tls.crt' \
  NIMINO_CHIRPS_PRIVATE_KEY_PATH='${CHIRPS_DIR}/tls.key' \
  NIMINO_CHIRPS_TRUST_ANCHOR_PATHS='${CHIRPS_DIR}/ca.crt' \
  NIMINO_NODE_STORE_PATH='${CLUSTER_DIR}/data.redb' \
  NIMINO_OBJECT_STORE_PATH='${CLUSTER_DIR}/objects' \
  NIMINO_S3_ENDPOINT=http://localhost:${MINIO_PORT} \
  NIMINO_S3_ACCESS_KEY=nimino_dev \
  NIMINO_S3_SECRET_KEY=nimino_dev_secret \
  NIMINO_S3_BUCKET=nimino-media \
  NIMINO_REQUIRE_AUTH_TOKEN=false \
  NIMINO_RECONCILE_CHANNELS=true \
  './target/${CARGO_TARGET_PROFILE}/nimino-relay' > '${RELAY_LOG}' 2>&1"

# Wait for the main port to accept connections.
for _ in $(seq 1 30); do
  if curl -s -o /dev/null "http://localhost:${RELAY_MAIN}/"; then
    ok "Relay live — NIMINO_E2E_RELAY_URL=http://localhost:${RELAY_MAIN}"
    ok "Logs: ${RELAY_LOG}   Attach: tmux attach -t ${TMUX_SESSION}"
    ok "Stop relay: tmux kill-session -t ${TMUX_SESSION}"
    ok "Full teardown: docker compose -p ${PROJECT} -f ${COMPOSE_FILE} down -v"
    exit 0
  fi
  sleep 1
done
err "Relay did not come up on :${RELAY_MAIN} within 30s — check ${RELAY_LOG}"
exit 1
