#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

fail() {
  echo "ERROR: $*" >&2
  exit 1
}

for binary in atlas nim nim-gdb nim_dbg nimble nimgrep nimpretty nimsuggest testament; do
  [[ -L "bin/$binary" ]] || fail "bin/$binary must be managed by Hermit"
  [[ "$(readlink "bin/$binary")" == ".nim-2.2.10.pkg" ]] ||
    fail "bin/$binary must select Nim 2.2.10"
done
[[ "$(readlink bin/.nim-2.2.10.pkg)" == "hermit" ]] ||
  fail "Nim package selector must dispatch through Hermit"
bin/hermit manifest validate "file://$repo_root/hermit-packages"

nim_version="$(bin/nim --version 2>&1)"
[[ "$nim_version" == *"Nim Compiler Version 2.2.10"* ]] ||
  fail "expected pinned Nim 2.2.10"

for recipe in nim-build nim-check nim-test nim-ci; do
  bin/just --show "$recipe" >/dev/null || fail "missing just recipe: $recipe"
  if bin/just --show "$recipe" | grep -Eq '(^|[[:space:]])(cargo|rustc)([[:space:]]|$)'; then
    fail "$recipe must not invoke the Rust toolchain"
  fi
done

grep -Fq 'nim: ${{ steps.filter.outputs.nim }}' .github/workflows/ci.yml ||
  fail "CI changed-path output for Nim is missing"
grep -Eq '^  nim:$' .github/workflows/ci.yml || fail "dedicated CI Nim job is missing"
grep -Fq -- "- 'nim/**'" .github/workflows/ci.yml || fail "Nim path filter is missing"
grep -Fq 'run: scripts/test-nim-bootstrap-contract.sh' .github/workflows/ci.yml ||
  fail "CI does not run the Nim bootstrap contract"
grep -Fq 'run: scripts/measure-nim-feedback.sh "$RUNNER_TEMP/nim-feedback-baseline.json"' \
  .github/workflows/ci.yml || fail "CI does not record the Nim feedback baseline"
grep -Fq 'name: nim-feedback-baseline' .github/workflows/ci.yml ||
  fail "CI does not retain the Nim feedback artifact"

bin/just nim-ci
