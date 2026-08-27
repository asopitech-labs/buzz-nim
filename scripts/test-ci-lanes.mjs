#!/usr/bin/env node

import { readFileSync } from "node:fs";

const workflow = readFileSync(".github/workflows/ci.yml", "utf8");
const justfile = readFileSync("Justfile", "utf8");
const hooks = readFileSync("lefthook.yml", "utf8");
const nimFeedback = readFileSync("scripts/measure-nim-feedback.sh", "utf8");
const nimBootstrap = readFileSync("scripts/test-nim-bootstrap-contract.sh", "utf8");
const rustTests = readFileSync("scripts/run-tests.sh", "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function job(name) {
  const start = workflow.indexOf(`\n  ${name}:\n`);
  check(start !== -1, `missing ${name} job`);
  const rest = workflow.slice(start + 1);
  const end = rest.slice(1).search(/^  [a-z][a-z0-9-]*:\n/m);
  return end === -1 ? rest : rest.slice(0, end + 1);
}

function hook(name) {
  const marker = `    ${name}:\n`;
  const start = hooks.indexOf(marker);
  check(start !== -1, `missing ${name} hook`);
  const bodyStart = start + marker.length;
  const next = /^    [a-z][a-z0-9-]*:\n/m.exec(hooks.slice(bodyStart));
  return hooks.slice(start, next ? bodyStart + next.index : hooks.length);
}

function hookGlobs(name) {
  const value = hook(name).match(/^ {6}glob: (\[.*\])$/m)?.[1];
  check(value, `missing ${name} hook glob`);
  return JSON.parse(value);
}

function checkFocusedHook(name, expectedGlobs, expectedRun) {
  const block = hook(name);
  check(
    block.match(/^ {6}files: (.*)$/m)?.[1] === "git diff --name-only origin/main...HEAD",
    `${name} hook must use the CI-equivalent merge-base diff`,
  );
  check(
    JSON.stringify(hookGlobs(name).sort()) === JSON.stringify(expectedGlobs.sort()),
    `${name} hook paths must mirror CI`,
  );
  check(
    block.match(/^ {6}run: (.*)$/m)?.[1] === expectedRun &&
      (block.match(/^ {6}run:/gm) ?? []).length === 1,
    `${name} hook must run only ${expectedRun}`,
  );
}

const filterStart = workflow.indexOf("          filters: |\n");
check(filterStart !== -1, "missing path filters");
check(
  workflow.slice(0, filterStart).includes("predicate-quantifier: 'some-with-excludes'"),
  "negative path rules require some-with-excludes",
);
const filterEnd = workflow.indexOf("\n      - name:", filterStart);
const filters = {};
let current;
for (const line of workflow.slice(filterStart, filterEnd).split("\n")) {
  const group = line.match(/^ {12}([a-z][a-z-]*):$/)?.[1];
  if (group) {
    current = group;
    filters[current] = [];
    continue;
  }
  const pattern = line.match(/^ {14}- '([^']+)'$/)?.[1];
  if (pattern && current) filters[current].push(pattern);
}

function globRegex(glob) {
  let regex = "^";
  for (let index = 0; index < glob.length; index += 1) {
    const char = glob[index];
    if (char === "*" && glob[index + 1] === "*") {
      index += 1;
      if (glob[index + 1] === "/") {
        index += 1;
        regex += "(?:.*/)?";
      } else regex += ".*";
    } else if (char === "*") regex += "[^/]*";
    else if (char === "?") regex += "[^/]";
    else regex += char.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }
  return new RegExp(`${regex}$`);
}

function matches(group, path) {
  const patterns = filters[group] ?? [];
  const included = patterns
    .filter((pattern) => !pattern.startsWith("!"))
    .some((pattern) => globRegex(pattern).test(path));
  const excluded = patterns
    .filter((pattern) => pattern.startsWith("!"))
    .some((pattern) => globRegex(pattern.slice(1)).test(path));
  return included && !excluded;
}

check(filters.boundary, "missing boundary path filter");
check(!filters.mobile, "Mobile path filter must be removed");
check(
  matches("nim", "nim/nimino_core/src/nimino_core/domain/message.nim") &&
    !matches("boundary", "nim/nimino_core/src/nimino_core/domain/message.nim") &&
    !matches("rust", "nim/nimino_core/src/nimino_core/domain/message.nim"),
  "Nim domain-only change must stay Rust-free",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/boundary/protocol.nim") &&
    matches("boundary", "nim/nimino_core/src/nimino_core/boundary/protocol.nim"),
  "Nim boundary change must run both Nim and cross-language lanes",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/domain/event_policy.nim") &&
    matches("boundary", "nim/nimino_core/src/nimino_core/domain/event_policy.nim"),
  "event policy changes must run Nim and real boundary golden tests",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/domain/community_policy.nim") &&
    matches("boundary", "nim/nimino_core/src/nimino_core/domain/community_policy.nim"),
  "community policy changes must run Nim and real boundary golden tests",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/domain/membership_policy.nim") &&
    matches("boundary", "nim/nimino_core/src/nimino_core/domain/membership_policy.nim"),
  "membership policy changes must run Nim and real boundary golden tests",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/domain/dm_policy.nim") &&
    matches("boundary", "nim/nimino_core/src/nimino_core/domain/dm_policy.nim"),
  "DM policy changes must run Nim and real boundary golden tests",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/domain/moderation_policy.nim") &&
    matches("boundary", "nim/nimino_core/src/nimino_core/domain/moderation_policy.nim"),
  "moderation policy changes must run Nim and real boundary golden tests",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/domain/cluster_lifecycle.nim") &&
    matches("boundary", "nim/nimino_core/src/nimino_core/domain/cluster_lifecycle.nim"),
  "cluster lifecycle changes must run Nim and real boundary golden tests",
);
check(
  matches("boundary", "crates/nimino-boundary/src/lib.rs") &&
    !matches("rust", "crates/nimino-boundary/src/lib.rs"),
  "Rust boundary adapter must use the focused boundary lane",
);
check(
  matches("rust", "crates/buzz-core/src/lib.rs") &&
    !matches("boundary", "crates/buzz-core/src/lib.rs"),
  "ordinary Rust changes must not run the boundary lane",
);
check(
  matches("desktop", "desktop/src/App.tsx") &&
    !matches("desktop-rust", "desktop/src/App.tsx") &&
    !matches("desktop", "desktop/src-tauri/src/lib.rs") &&
    matches("desktop-rust", "desktop/src-tauri/src/lib.rs"),
  "desktop frontend and Tauri paths must stay disjoint",
);
check(
  Object.keys(filters).every((group) => !matches(group, "mobile/lib/main.dart")),
  "Mobile changes must not select a CI product lane",
);
check(
  job("changes").includes("run: node scripts/test-nimino-data-contract.mjs"),
  "changed-path gate must verify the Nimino data contract",
);
check(
  justfile.includes("cargo nextest run -p nimino-store") &&
    rustTests.includes("cargo test -p nimino-store"),
  "both Rust unit runners must execute nimino-store recovery tests",
);
check(
  job("changes").includes("run: node scripts/test-nimino-event-contract.mjs"),
  "changed-path gate must verify the Nimino event policy contract",
);
check(
  job("changes").includes("run: node scripts/test-nimino-community-contract.mjs"),
  "changed-path gate must verify the Nimino community policy contract",
);
check(
  job("changes").includes("run: node scripts/test-nimino-membership-contract.mjs"),
  "changed-path gate must verify the Nimino membership policy contract",
);
check(
  job("changes").includes("run: node scripts/test-nimino-dm-contract.mjs"),
  "changed-path gate must verify the Nimino DM policy contract",
);
check(
  job("changes").includes("run: node scripts/test-nimino-moderation-contract.mjs"),
  "changed-path gate must verify the Nimino moderation policy contract",
);
check(
  job("changes").includes("run: just nimino-cluster-contract"),
  "changed-path gate must verify the Nimino cluster lifecycle contract",
);

const nimJob = job("nim");
check(
  nimJob.includes("scripts/measure-nim-feedback.sh"),
  "Nim job must run the measured nim-ci gate",
);
check(!nimJob.includes("nim-boundary-ci"), "Nim job must not run boundary CI");
check(
  JSON.stringify([...nimJob.matchAll(/^ {8}run: (.*)$/gm)].map((match) => match[1])) ===
    JSON.stringify([
      "|",
      "scripts/test-nim-bootstrap-contract.sh",
      'scripts/measure-nim-feedback.sh "$RUNNER_TEMP/nim-feedback-baseline.json"',
    ]),
  "Nim job run entrypoints must stay on the Rust-free allowlist",
);
const nimShellMarker = "        run: |\n";
const nimShellStart = nimJob.indexOf(nimShellMarker);
check(nimShellStart !== -1, "Nim job must contain its toolchain version block");
const nimShellRest = nimJob.slice(nimShellStart + nimShellMarker.length);
const nimShellEnd = nimShellRest.search(/^ {6}- /m);
const nimShellBody = nimShellEnd === -1 ? nimShellRest : nimShellRest.slice(0, nimShellEnd);
check(
  nimShellBody === "          nim --version\n          nimble --version\n",
  "Nim job shell block must contain only pinned toolchain version checks",
);
check(!/\b(?:cargo|rustc)\b/i.test(nimFeedback), "Nim feedback probe must stay Rust-free");
check(
  nimBootstrap.includes("bin/just nim-ci") &&
    nimBootstrap.includes("(cargo|rustc)") &&
    nimBootstrap.includes("bin/just --show \"$recipe\""),
  "Nim bootstrap must reject Rust toolchain commands before running nim-ci",
);
check(
  JSON.stringify(
    nimBootstrap
      .split("\n")
      .filter((line) => /\b(?:cargo|rustc)\b/i.test(line))
      .map((line) => line.trim()),
  ) ===
    JSON.stringify([
      "if bin/just --show \"$recipe\" | grep -Eq '(^|[[:space:]])(cargo|rustc)([[:space:]]|$)'; then",
    ]),
  "Nim bootstrap must not invoke Cargo or rustc directly",
);
check(
  JSON.stringify(
    nimBootstrap
      .split("\n")
      .filter((line) => /\bjust\b/.test(line))
      .map((line) => line.trim()),
  ) ===
    JSON.stringify([
      'bin/just --show "$recipe" >/dev/null || fail "missing just recipe: $recipe"',
      "if bin/just --show \"$recipe\" | grep -Eq '(^|[[:space:]])(cargo|rustc)([[:space:]]|$)'; then",
      "bin/just nim-ci",
    ]),
  "Nim bootstrap may invoke only the inspected Nim recipes",
);
check(
  /^nim-ci: nim-check nim-build nim-test$/m.test(justfile),
  "nim-ci dependencies must stay Rust-free",
);
check(
  /^nim-boundary-ci: nim-boundary-test nim-boundary-benchmark$/m.test(justfile),
  "boundary CI must remain a separate focused gate",
);
check(
  /^control-model-contract:\n    node scripts\/check-nimino-control-model\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just control-model-contract"),
  "control model contract must remain in CI",
);
check(
  /^nimino-control-state-contract:\n    node scripts\/test-nimino-control-state-contract\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just nimino-control-state-contract"),
  "control state-machine contract must remain in CI",
);
check(
  /^nimino-lease-contract:\n    node scripts\/test-nimino-lease-contract\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just nimino-lease-contract"),
  "lease and fencing contract must remain in CI",
);
check(
  /^nimino-sync-contract:\n    node scripts\/test-nimino-sync-contract\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just nimino-sync-contract"),
  "bounded anti-entropy contract must remain in CI",
);
check(
  /^nimino-convergence-contract:\n    node scripts\/test-nimino-convergence-contract\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just nimino-convergence-contract"),
  "deterministic convergence contract must remain in CI",
);
check(
  /^nimino-object-sync-contract:\n    node scripts\/test-nimino-object-sync-contract\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just nimino-object-sync-contract"),
  "object sync contract must remain in CI",
);
check(
  /^nimino-projection-contract:\n    node scripts\/test-nimino-projection-contract\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just nimino-projection-contract"),
  "projection rebuild contract must remain in CI",
);

const boundaryJob = job("boundary");
check(
  boundaryJob.includes("run: just nim-boundary-ci"),
  "boundary job must run the cross-language gate",
);
check(
  boundaryJob.includes("Swatinem/rust-cache"),
  "boundary Cargo work must use the Rust cache",
);
check(
  boundaryJob.includes("cargo fmt -p nimino-boundary") &&
    boundaryJob.includes("cargo clippy -p nimino-boundary --all-targets --all-features"),
  "boundary Rust checks must stay focused",
);
check(
  job("security").includes("needs.changes.outputs.boundary == 'true'"),
  "boundary dependency changes must run security policy",
);
check(!/\b(?:mobile|flutter)\b/i.test(workflow), "CI must not contain a Mobile product lane");
check(
  !justfile.match(/^check:.*\bmobile-check\b/m) &&
    !justfile.match(/^ci:.*\bmobile-test\b/m) &&
    !justfile.match(/^fmt-all:.*\bmobile-fmt\b/m) &&
    !justfile.includes("node mobile/scripts/check-file-sizes.mjs"),
  "default quality gates must not run Mobile tooling",
);
const localFilter = (patterns) =>
  patterns.filter((pattern) => pattern !== ".github/workflows/ci.yml").sort();
checkFocusedHook("nim-checks", localFilter(filters.nim), "just nim-ci");
checkFocusedHook("boundary-checks", localFilter(filters.boundary), "just nim-boundary-ci");
check(
  hook("rust-tests").includes('exclude: ["crates/nimino-boundary/**"]') &&
    hook("desktop-tauri-checks").includes('exclude: ["crates/nimino-boundary/**"]'),
  "general Rust hooks must exclude the focused boundary crate",
);
check(!/^\s+mobile-(?:fmt|checks):$/m.test(hooks), "Mobile git-hook lanes must be removed");

console.log("CI lane contract passed: Nim-only is Rust-free; boundary is integrated; Mobile has no lane");
