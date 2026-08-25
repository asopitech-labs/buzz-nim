#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
output_path="${1:-$repo_root/target/nim/feedback-baseline.json}"

now_ms() {
  local value
  value="$(date +%s%3N)"
  if [[ "$value" =~ ^[0-9]+$ ]]; then
    printf '%s\n' "$value"
  else
    # BSD date has no %N support. Preserve portability with second precision.
    printf '%s000\n' "$(date +%s)"
  fi
}

measure_ms() {
  local result_var="$1"
  shift
  local started finished
  started="$(now_ms)"
  "$@"
  finished="$(now_ms)"
  printf -v "$result_var" '%d' "$((finished - started))"
}

cd "$repo_root"
mkdir -p "$(dirname "$output_path")"
mkdir -p "$repo_root/target/nim"

probe_root="$(mktemp -d "$repo_root/target/nim/feedback-probe.XXXXXX")"
probe_dir="$probe_root/nimino_core"
mkdir -p "$probe_dir"
cp -R "$repo_root/nim/nimino_core/." "$probe_dir/"
printf '%s\n' \
  '--path:"src"' \
  '--nimcache:".build/cache"' \
  '--outdir:".build/bin"' >"$probe_dir/nim.cfg"

cleanup_probe() {
  rm -rf -- "$probe_root"
}
trap cleanup_probe EXIT

measure_ms clean_nim_ci_ms bin/just --set nim_core_dir "$probe_dir" nim-ci

probe="$(date +%s)-$$"
printf '\nproc niminoIssue13FeedbackProbe*(): string =\n  "%s"\n' "$probe" \
  >>"$probe_dir/src/nimino_core.nim"
printf '\ntest "feedback probe recompiles changed code":\n  check niminoIssue13FeedbackProbe() == "%s"\n' "$probe" \
  >>"$probe_dir/tests/test_nimino_core.nim"
measure_ms edit_to_test_ms bin/just --set nim_core_dir "$probe_dir" nim-test

cleanup_probe
trap - EXIT

compiler="$(bin/nim --version 2>&1 | sed -n '1p')"
platform="$(uname -sm)"
generated_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

printf '{\n' >"$output_path"
printf '  "generated_at": "%s",\n' "$generated_at" >>"$output_path"
printf '  "platform": "%s",\n' "$platform" >>"$output_path"
printf '  "compiler": "%s",\n' "$compiler" >>"$output_path"
printf '  "edit_to_test_ms": %d,\n' "$edit_to_test_ms" >>"$output_path"
printf '  "clean_nim_ci_ms": %d\n' "$clean_nim_ci_ms" >>"$output_path"
printf '}\n' >>"$output_path"

echo "Nim feedback baseline written to $output_path"
