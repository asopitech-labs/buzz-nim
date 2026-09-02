#!/usr/bin/env bash
# Run deterministic desktop correctness smoke; timing metrics are informational.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ARTIFACT_DIR="${NIMINO_RELEASE_SMOKE_ARTIFACT_DIR:-${ROOT}/target/release-smoke}"
DB_NAME="${NIMINO_RELEASE_SMOKE_DB:-nimino_release_smoke_${$}}"
RUNTIME_DIR="$(mktemp -d "${TMPDIR:-/tmp}/nimino-desktop-release-smoke.XXXXXX")"
RELAY_PID=""

free_port() {
  python3 - <<'PY'
import socket
with socket.socket() as sock:
    sock.bind(("127.0.0.1", 0))
    print(sock.getsockname()[1])
PY
}

RELAY_PORT="${NIMINO_RELEASE_SMOKE_RELAY_PORT:-$(free_port)}"
HEALTH_PORT="${NIMINO_RELEASE_SMOKE_HEALTH_PORT:-$(free_port)}"
METRICS_PORT="${NIMINO_RELEASE_SMOKE_METRICS_PORT:-$(free_port)}"
CHIRPS_PORT="${NIMINO_RELEASE_SMOKE_CHIRPS_PORT:-$(free_port)}"
COMMUNITY_HOST="localhost:${RELAY_PORT}"
RELAY_HTTP_URL="http://${COMMUNITY_HOST}"
STARTED_AT="$(date +%s)"
RELAY_READY_TIMEOUT_SECONDS="${NIMINO_RELEASE_SMOKE_RELAY_READY_TIMEOUT_SECONDS:-180}"
DB_POOL_SIZE="${NIMINO_RELEASE_SMOKE_DB_POOL_SIZE:-80}"
NIM_WORKER="${ROOT}/target/nim/nimino_boundary/bin/nimino-core-worker"
CHIRPS_DIR="${ROOT}/target/nim/dev-cluster"

log() { printf '[desktop-release-smoke] %s\n' "$*"; }
phase() {
  local name="$1" start="$2"
  printf '{"phase":"%s","duration_ms":%d}\n' "$name" "$(( ($(date +%s) - start) * 1000 ))" >> "${ARTIFACT_DIR}/phases.jsonl"
}

cleanup() {
  local status=$?
  trap - EXIT INT TERM
  if [[ -n "${RELAY_PID}" ]]; then
    kill "${RELAY_PID}" 2>/dev/null || true
    for _ in $(seq 1 50); do
      kill -0 "${RELAY_PID}" 2>/dev/null || break
      sleep 0.1
    done
    kill -9 "${RELAY_PID}" 2>/dev/null || true
  fi
  docker exec nimino-postgres dropdb -U nimino --if-exists "${DB_NAME}" >/dev/null 2>&1 || true
  rm -rf -- "${RUNTIME_DIR}"
  exit "${status}"
}
trap cleanup EXIT INT TERM

mkdir -p "${ARTIFACT_DIR}"
: > "${ARTIFACT_DIR}/phases.jsonl"
cd "${ROOT}"

phase_start="$(date +%s)"
log "starting backing services"
docker compose up -d postgres minio minio-init
for container in nimino-postgres nimino-minio; do
  for _ in $(seq 1 60); do
    [[ "$(docker inspect --format='{{.State.Health.Status}}' "${container}" 2>/dev/null || true)" == "healthy" ]] && break
    sleep 1
  done
  [[ "$(docker inspect --format='{{.State.Health.Status}}' "${container}" 2>/dev/null || true)" == "healthy" ]] || {
    docker logs "${container}" || true
    exit 1
  }
done
phase services "${phase_start}"

phase_start="$(date +%s)"
log "creating isolated database ${DB_NAME}"
docker exec nimino-postgres createdb -U nimino "${DB_NAME}"
export PGHOST=localhost PGPORT=5432 PGUSER=nimino PGPASSWORD=nimino_dev PGDATABASE="${DB_NAME}"
export PGSCHEMA_PLAN_HOST=localhost PGSCHEMA_PLAN_PORT=5432 PGSCHEMA_PLAN_DB="${DB_NAME}"
export PGSCHEMA_PLAN_USER=nimino PGSCHEMA_PLAN_PASSWORD=nimino_dev
./bin/pgschema apply --file schema/schema.sql --auto-approve
docker exec -i -e PGPASSWORD=nimino_dev nimino-postgres \
  psql -U nimino -d "${DB_NAME}" -v ON_ERROR_STOP=1 < scripts/attach-schema-partitions.sql
NIMINO_DB_NAME="${DB_NAME}" NIMINO_COMMUNITY_HOST="${COMMUNITY_HOST}" ./scripts/setup-desktop-test-data.sh
phase database "${phase_start}"

phase_start="$(date +%s)"
if [[ -n "${NIMINO_E2E_RELAY_BIN:-}" ]]; then
  RELAY_BIN="${NIMINO_E2E_RELAY_BIN}"
else
  log "building relay"
  cargo build --profile ci -p nimino-relay
  RELAY_BIN="${ROOT}/target/ci/nimino-relay"
fi
for required in "${NIM_WORKER}" "${CHIRPS_DIR}/tls.crt" "${CHIRPS_DIR}/tls.key" "${CHIRPS_DIR}/ca.crt"; do
  [[ -s "${required}" ]] || { log "missing runtime dependency: ${required}"; exit 1; }
done
log "starting relay at ${RELAY_HTTP_URL}"
env \
  DATABASE_URL="postgres://nimino:nimino_dev@localhost:5432/${DB_NAME}" \
  RELAY_URL="ws://${COMMUNITY_HOST}" \
  NIMINO_BIND_ADDR="127.0.0.1:${RELAY_PORT}" \
  NIMINO_HEALTH_PORT="${HEALTH_PORT}" \
  NIMINO_METRICS_PORT="${METRICS_PORT}" \
  NIMINO_BOUNDARY_WORKER="${NIM_WORKER}" \
  NIMINO_CHIRPS_BIND_ADDR="127.0.0.1:${CHIRPS_PORT}" \
  NIMINO_CHIRPS_IDENTITY_PATH="${RUNTIME_DIR}/node.identity" \
  NIMINO_CHIRPS_CERTIFICATE_PATH="${CHIRPS_DIR}/tls.crt" \
  NIMINO_CHIRPS_PRIVATE_KEY_PATH="${CHIRPS_DIR}/tls.key" \
  NIMINO_CHIRPS_TRUST_ANCHOR_PATHS="${CHIRPS_DIR}/ca.crt" \
  NIMINO_NODE_STORE_PATH="${RUNTIME_DIR}/data.redb" \
  NIMINO_OBJECT_STORE_PATH="${RUNTIME_DIR}/objects" \
  NIMINO_DB_POOL_SIZE="${DB_POOL_SIZE}" \
  NIMINO_REQUIRE_AUTH_TOKEN=false \
  NIMINO_RECONCILE_CHANNELS=true \
  NIMINO_RATE_LIMIT_HUMAN_MESSAGES_PER_MIN=1000000 \
  NIMINO_RATE_LIMIT_HUMAN_API_CALLS_PER_MIN=1000000 \
  NIMINO_RATE_LIMIT_HUMAN_WS_EVENTS_PER_SEC=100000 \
  "${RELAY_BIN}" > "${ARTIFACT_DIR}/relay.log" 2>&1 &
RELAY_PID=$!
ready=false
ready_deadline=$((SECONDS + RELAY_READY_TIMEOUT_SECONDS))
while (( SECONDS < ready_deadline )); do
  kill -0 "${RELAY_PID}" 2>/dev/null || { cat "${ARTIFACT_DIR}/relay.log"; exit 1; }
  if curl --silent --fail --max-time 1 "http://127.0.0.1:${HEALTH_PORT}/_readiness" >/dev/null; then
    ready=true
    break
  fi
  sleep 0.1
done
[[ "${ready}" == true ]] || { cat "${ARTIFACT_DIR}/relay.log"; exit 1; }
phase relay "${phase_start}"

phase_start="$(date +%s)"
if [[ "${NIMINO_RELEASE_SMOKE_NO_BUILD:-0}" == "1" ]]; then
  log "reusing existing desktop E2E bundle"
else
  log "building desktop E2E bundle"
  pnpm -C desktop build:e2e
fi
phase build "${phase_start}"

phase_start="$(date +%s)"
log "running release smoke"
NIMINO_E2E_RELAY_URL="${RELAY_HTTP_URL}" \
NIMINO_RELEASE_SMOKE_ARTIFACT_DIR="${ARTIFACT_DIR}" \
pnpm -C desktop exec playwright test --config=playwright.release-smoke.config.ts
phase smoke "${phase_start}"
phase total "${STARTED_AT}"
log "artifacts: ${ARTIFACT_DIR}"
