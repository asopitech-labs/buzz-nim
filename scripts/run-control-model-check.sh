#!/usr/bin/env bash
set -euo pipefail

TLA_VERSION=1.7.4
TLA_SHA256=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
CACHE_DIR="${NIMINO_TLA_CACHE_DIR:-${XDG_CACHE_HOME:-${HOME:?HOME is required}/.cache}/nimino/tla}"
TLA_JAR="${CACHE_DIR}/tla2tools-${TLA_VERSION}.jar"

for tool in node curl java; do
  command -v "${tool}" >/dev/null || {
    echo "${tool} is required to run TLC" >&2
    exit 1
  }
done

hash_file() {
  node -p "require('node:crypto').createHash('sha256').update(require('node:fs').readFileSync(process.argv[1])).digest('hex')" "$1"
}

if [[ ! -f "${TLA_JAR}" ]] || [[ "$(hash_file "${TLA_JAR}")" != "${TLA_SHA256}" ]]; then
  mkdir -p "${CACHE_DIR}"
  tmp="$(mktemp "${TLA_JAR}.tmp.XXXXXX")"
  trap 'rm -f "${tmp}"' EXIT
  curl -fsSL "https://github.com/tlaplus/tlaplus/releases/download/v${TLA_VERSION}/tla2tools.jar" -o "${tmp}"
  [[ "$(hash_file "${tmp}")" == "${TLA_SHA256}" ]] || {
    echo "Downloaded TLA+ Tools checksum mismatch" >&2
    exit 1
  }
  mv -f "${tmp}" "${TLA_JAR}"
  trap - EXIT
fi

exec java -XX:+UseParallelGC -cp "${TLA_JAR}" tlc2.TLC "$@"
