#!/usr/bin/env node

import { readFileSync } from "node:fs";

const workflow = readFileSync(".github/workflows/ci.yml", "utf8");
const justfile = readFileSync("Justfile", "utf8");
const hooks = readFileSync("lefthook.yml", "utf8");
const nimFeedback = readFileSync("scripts/measure-nim-feedback.sh", "utf8");
const nimBootstrap = readFileSync(
  "scripts/test-nim-bootstrap-contract.sh",
  "utf8",
);
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
    block.match(/^ {6}files: (.*)$/m)?.[1] ===
      "git diff --name-only origin/main...HEAD",
    `${name} hook must use the CI-equivalent merge-base diff`,
  );
  check(
    JSON.stringify(hookGlobs(name).sort()) ===
      JSON.stringify(expectedGlobs.sort()),
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
  workflow
    .slice(0, filterStart)
    .includes("predicate-quantifier: 'some-with-excludes'"),
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
    !matches(
      "boundary",
      "nim/nimino_core/src/nimino_core/domain/message.nim",
    ) &&
    !matches("rust", "nim/nimino_core/src/nimino_core/domain/message.nim"),
  "Nim domain-only change must stay Rust-free",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/boundary/protocol.nim") &&
    matches(
      "boundary",
      "nim/nimino_core/src/nimino_core/boundary/protocol.nim",
    ),
  "Nim boundary change must run both Nim and cross-language lanes",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/domain/event_policy.nim") &&
    matches(
      "boundary",
      "nim/nimino_core/src/nimino_core/domain/event_policy.nim",
    ),
  "event policy changes must run Nim and real boundary golden tests",
);
check(
  matches(
    "nim",
    "nim/nimino_core/src/nimino_core/domain/community_policy.nim",
  ) &&
    matches(
      "boundary",
      "nim/nimino_core/src/nimino_core/domain/community_policy.nim",
    ),
  "community policy changes must run Nim and real boundary golden tests",
);
check(
  matches(
    "nim",
    "nim/nimino_core/src/nimino_core/domain/membership_policy.nim",
  ) &&
    matches(
      "boundary",
      "nim/nimino_core/src/nimino_core/domain/membership_policy.nim",
    ),
  "membership policy changes must run Nim and real boundary golden tests",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/domain/dm_policy.nim") &&
    matches("boundary", "nim/nimino_core/src/nimino_core/domain/dm_policy.nim"),
  "DM policy changes must run Nim and real boundary golden tests",
);
check(
  matches(
    "nim",
    "nim/nimino_core/src/nimino_core/domain/moderation_policy.nim",
  ) &&
    matches(
      "boundary",
      "nim/nimino_core/src/nimino_core/domain/moderation_policy.nim",
    ),
  "moderation policy changes must run Nim and real boundary golden tests",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/domain/cli_policy.nim") &&
    matches(
      "boundary",
      "nim/nimino_core/src/nimino_core/domain/cli_policy.nim",
    ) &&
    matches("boundary", "contracts/nimino-cli/v1/contract.json") &&
    matches("boundary", "scripts/test-nimino-cli-contract.mjs"),
  "CLI policy changes must run Nim and real boundary golden tests",
);
check(
  matches("nim", "nim/nimino_core/src/nimino_core/domain/agent_policy.nim") &&
    matches(
      "boundary",
      "nim/nimino_core/src/nimino_core/domain/agent_policy.nim",
    ) &&
    matches("boundary", "contracts/nimino-agent/v1/contract.json") &&
    matches("boundary", "scripts/test-nimino-agent-contract.mjs"),
  "agent policy changes must run Nim and real boundary golden tests",
);
check(
  matches(
    "nim",
    "nim/nimino_core/src/nimino_core/domain/cluster_lifecycle.nim",
  ) &&
    matches(
      "boundary",
      "nim/nimino_core/src/nimino_core/domain/cluster_lifecycle.nim",
    ),
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
  matches("desktop-rust", "desktop/src-tauri/src/native_websocket.rs") &&
    !matches("desktop-rust", "contracts/nimino-tauri/v1/manifest.json"),
  "Tauri source changes must run native tests while manifest-only changes stay contract-only",
);
check(
  Object.keys(filters).every(
    (group) => !matches(group, "mobile/lib/main.dart"),
  ),
  "Mobile changes must not select a CI product lane",
);
check(
  matches("web", "web/src/app/routes.ts") &&
    matches("web", "admin-web/src/App.tsx"),
  "Web lane must cover both surviving browser clients",
);
const webJob = job("web");
check(
  webJob.includes("run: just web-check") &&
    webJob.includes("run: just web-build") &&
    webJob.includes("run: pnpm -C admin-web check") &&
    webJob.includes("run: pnpm -C admin-web build"),
  "Web lane must check and build Web and Admin",
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
  job("changes").includes(
    "run: node scripts/test-nimino-community-contract.mjs",
  ),
  "changed-path gate must verify the Nimino community policy contract",
);
check(
  job("changes").includes(
    "run: node scripts/test-nimino-membership-contract.mjs",
  ),
  "changed-path gate must verify the Nimino membership policy contract",
);
check(
  job("changes").includes("run: node scripts/test-nimino-dm-contract.mjs"),
  "changed-path gate must verify the Nimino DM policy contract",
);
check(
  job("changes").includes(
    "run: node scripts/test-nimino-moderation-contract.mjs",
  ),
  "changed-path gate must verify the Nimino moderation policy contract",
);
check(
  job("changes").includes("run: just nimino-cli-contract"),
  "changed-path gate must verify the Nimino CLI contract",
);
check(
  job("changes").includes("run: just nimino-agent-contract"),
  "changed-path gate must verify the Nimino agent contract",
);
check(
  job("changes").includes("run: just nimino-tauri-contract"),
  "changed-path gate must verify the Tauri adapter contract",
);
check(
  job("changes").includes("run: just rust-responsibility-contract"),
  "changed-path gate must verify the Rust responsibility contract",
);
check(
  job("changes").includes("run: just release-set-contract"),
  "changed-path gate must verify the immutable release-set contract",
);
check(
  job("changes").includes("run: just relay-release-contract"),
  "changed-path gate must verify the unified relay release contract",
);
check(
  job("changes").includes("run: just agent-bundle-contract"),
  "changed-path gate must verify the manifest-driven agent bundle",
);
check(
  job("changes").includes("run: just wsl-service-contract"),
  "changed-path gate must verify the WSL service lifecycle",
);
check(
  job("changes").includes("run: just wsl-launcher-contract"),
  "changed-path gate must verify the WSL launcher and secret boundary",
);
check(
  job("changes").includes("run: just wsl-chirps-contract"),
  "changed-path gate must verify the WSL Chirps runtime contract",
);
check(
  job("changes").includes("run: just wsl-bundle-contract"),
  "changed-path gate must verify the complete WSL bundle contract",
);
check(
  job("changes").includes("run: just nimino-cluster-contract"),
  "changed-path gate must verify the Nimino cluster lifecycle contract",
);
check(
  job("changes").includes("run: just nimino-cluster-scenario-contract"),
  "changed-path gate must verify the real-mesh scenario contract",
);

const nimJob = job("nim");
check(
  nimJob.includes("scripts/measure-nim-feedback.sh"),
  "Nim job must run the measured nim-ci gate",
);
check(!nimJob.includes("nim-boundary-ci"), "Nim job must not run boundary CI");
check(
  JSON.stringify(
    [...nimJob.matchAll(/^ {8}run: (.*)$/gm)].map((match) => match[1]),
  ) ===
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
const nimShellBody =
  nimShellEnd === -1 ? nimShellRest : nimShellRest.slice(0, nimShellEnd);
check(
  nimShellBody === "          nim --version\n          nimble --version\n",
  "Nim job shell block must contain only pinned toolchain version checks",
);
check(
  !/\b(?:cargo|rustc)\b/i.test(nimFeedback),
  "Nim feedback probe must stay Rust-free",
);
check(
  nimBootstrap.includes("bin/just nim-ci") &&
    nimBootstrap.includes("(cargo|rustc)") &&
    nimBootstrap.includes('bin/just --show "$recipe"'),
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
  /^nim-boundary-ci: nim-boundary-test nim-boundary-benchmark nimino-cluster-scenarios$/m.test(
    justfile,
  ),
  "boundary CI must include the real-mesh scenario gate",
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
check(
  /^nimino-effect-ledger-contract:\n    node scripts\/test-nimino-effect-ledger-contract\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just nimino-effect-ledger-contract"),
  "workflow effect ledger contract must remain in CI",
);
check(
  /^nimino-data-ops-contract:\n    node scripts\/test-nimino-data-ops-contract\.mjs$/m.test(
    justfile,
  ) &&
    /^nimino-data-ops-scenarios:\n    cargo test -p nimino-data-ops --test convergence_scenarios -- --test-threads=1$/m.test(
      justfile,
    ) &&
    workflow.includes("run: just nimino-data-ops-contract") &&
    justfile.includes("cargo nextest run -p nimino-data-ops"),
  "data verify/repair contract and scenarios must remain in CI",
);
check(
  /^nimino-mcp-execution-contract:\n    node scripts\/test-nimino-mcp-execution-contract\.mjs$/m.test(
    justfile,
  ) &&
    /^nimino-mcp-framing:\n    cargo build -p buzz-dev-mcp\n    node scripts\/test-nimino-mcp-framing\.mjs target\/debug\/buzz-dev-mcp$/m.test(
      justfile,
    ) &&
    workflow.includes("run: just nimino-mcp-execution-contract") &&
    justfile.includes("cargo nextest run -p buzz-dev-mcp") &&
    justfile.includes(
      "node scripts/test-nimino-mcp-framing.mjs target/debug/buzz-dev-mcp",
    ) &&
    rustTests.includes("cargo test -p buzz-dev-mcp") &&
    rustTests.includes(
      "node scripts/test-nimino-mcp-framing.mjs target/debug/buzz-dev-mcp",
    ),
  "MCP execution contract, framing, and unit tests must remain in CI",
);
check(
  /^nimino-cli-contract:\n    node scripts\/test-nimino-cli-contract\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just nimino-cli-contract"),
  "CLI grammar and exit contract must remain in CI",
);
check(
  /^nimino-agent-contract:\n    node scripts\/test-nimino-agent-contract\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just nimino-agent-contract"),
  "agent/persona contract must remain in CI",
);
check(
  /^legacy-control-manifest-contract:\n    node scripts\/check-legacy-control-manifest\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just legacy-control-manifest-contract"),
  "legacy mesh and Redis replacement manifest must remain in CI",
);
check(
  /^nimino-tauri-contract:\n    node scripts\/test-nimino-tauri-contract\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just nimino-tauri-contract"),
  "Tauri adapter inventory must remain in CI",
);
check(
  hook("tauri-adapter-contract").includes("run: just nimino-tauri-contract") &&
    hookGlobs("tauri-adapter-contract").includes("desktop/src-tauri/src/**") &&
    hookGlobs("tauri-adapter-contract").includes("contracts/nimino-tauri/**"),
  "Tauri adapter pre-push gate must cover source and manifest changes",
);
check(
  /^rust-responsibility-contract:\n {4}node scripts\/test-rust-responsibility-contract\.mjs$/m.test(
    justfile,
  ) && workflow.includes("run: just rust-responsibility-contract"),
  "Rust responsibility inventory must remain in CI",
);
check(
  hook("rust-responsibility-contract").includes(
    "run: just rust-responsibility-contract",
  ) &&
    hookGlobs("rust-responsibility-contract").includes("**/*.rs") &&
    hookGlobs("rust-responsibility-contract").includes("**/Cargo.toml") &&
    hookGlobs("rust-responsibility-contract").includes(
      "contracts/rust-responsibility/**",
    ),
  "Rust responsibility pre-push gate must cover source, packages, and manifest",
);
check(
  /^release-set-contract:\n {4}node scripts\/test-nimino-release-set\.mjs$/m.test(
    justfile,
  ) &&
    workflow.includes("run: just release-set-contract") &&
    hook("release-set-contract").includes("run: just release-set-contract") &&
    hookGlobs("release-set-contract").includes(
      "contracts/nimino-release-set/**",
    ),
  "immutable release-set authority must remain in CI and pre-push",
);
check(
  /^relay-release-contract:\n {4}node scripts\/test-nimino-relay-release-contract\.mjs$/m.test(
    justfile,
  ) &&
    workflow.includes("run: just relay-release-contract") &&
    hook("relay-release-contract").includes("run: just relay-release-contract") &&
    hookGlobs("relay-release-contract").includes(".github/workflows/nimino-relay-release.yml") &&
    hookGlobs("relay-release-contract").includes("deploy/charts/nimino/**"),
  "unified relay release contract must remain in CI and pre-push",
);
check(
  /^agent-bundle-contract:\n {4}node scripts\/test-nimino-agent-bundle\.mjs$/m.test(
    justfile,
  ) &&
    workflow.includes("run: just agent-bundle-contract") &&
    hook("agent-bundle-contract").includes("run: just agent-bundle-contract") &&
    hookGlobs("agent-bundle-contract").includes(
      "contracts/nimino-agent-bundle/**",
    ),
  "manifest-driven agent bundle must remain in CI and pre-push",
);
check(
  /^wsl-service-contract:\n {4}node scripts\/test-nimino-wsl-service\.mjs$/m.test(
    justfile,
  ) &&
    workflow.includes("run: just wsl-service-contract") &&
    hook("wsl-service-contract").includes("run: just wsl-service-contract") &&
    hookGlobs("wsl-service-contract").includes("scripts/nimino-wsl-service.sh"),
  "WSL service lifecycle must remain in CI and pre-push",
);
check(
  /^wsl-launcher-contract:\n {4}node scripts\/test-nimino-wsl-launcher-contract\.mjs$/m.test(
    justfile,
  ) &&
    workflow.includes("run: just wsl-launcher-contract") &&
    hook("wsl-launcher-contract").includes("run: just wsl-launcher-contract") &&
    hookGlobs("wsl-launcher-contract").includes(
      "crates/nimino-wsl-launcher/**",
    ) &&
    justfile.includes("cargo nextest run -p nimino-wsl-launcher") &&
    rustTests.includes("cargo test -p nimino-wsl-launcher"),
  "WSL launcher and Secret Service boundary must remain in all test runners",
);
check(
  /^wsl-chirps-contract:\n {4}node scripts\/test-nimino-wsl-chirps-contract\.mjs$/m.test(
    justfile,
  ) &&
    /^wsl-chirps-certify: wsl-chirps-contract\n {4}node scripts\/test-nimino-wsl-chirps-contract\.mjs --certify$/m.test(
      justfile,
    ) &&
    workflow.includes("run: just wsl-chirps-contract") &&
    hook("wsl-chirps-contract").includes("run: just wsl-chirps-contract") &&
    hookGlobs("wsl-chirps-contract").includes("crates/nimino-chirps/**"),
  "WSL Chirps certification must remain in source gates and the real WSL runner",
);
check(
  /^wsl-bundle-contract:\n {4}node scripts\/test-nimino-wsl-bundle-contract\.mjs$/m.test(
    justfile,
  ) &&
    /^wsl-bundle-e2e: wsl-bundle-contract\n {4}node scripts\/test-nimino-wsl-bundle-contract\.mjs --e2e$/m.test(
      justfile,
    ) &&
    /^wsl-bundle-certify: wsl-bundle-contract\n {4}node scripts\/test-nimino-wsl-bundle-contract\.mjs --certify$/m.test(
      justfile,
    ) &&
    workflow.includes("run: just wsl-bundle-contract") &&
    hook("wsl-bundle-contract").includes("run: just wsl-bundle-contract") &&
    hookGlobs("wsl-bundle-contract").includes(
      "scripts/nimino-wsl-service.sh",
    ) &&
    hookGlobs("wsl-bundle-contract").includes("contracts/nimino-wsl-bundle/**"),
  "complete WSL bundle must remain in source gates and the real WSL runner",
);
check(
  /^nimino-cluster-scenario-contract:\n    node scripts\/test-nimino-cluster-scenario-contract\.mjs$/m.test(
    justfile,
  ) &&
    /^nimino-cluster-scenarios output="target\/nim\/nimino-cluster-scenarios\.json": nimino-cluster-scenario-contract nim-boundary-build$/m.test(
      justfile,
    ) &&
    workflow.includes("run: just nim-boundary-ci") &&
    workflow.includes("path: target/nim/nimino-cluster-scenarios.json"),
  "real-mesh scenario suite and evidence must remain in CI",
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
    boundaryJob.includes(
      "cargo clippy -p nimino-boundary --all-targets --all-features",
    ),
  "boundary Rust checks must stay focused",
);
check(
  job("security").includes("needs.changes.outputs.boundary == 'true'"),
  "boundary dependency changes must run security policy",
);
check(
  !/\b(?:mobile|flutter)\b/i.test(workflow),
  "CI must not contain a Mobile product lane",
);
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
checkFocusedHook(
  "boundary-checks",
  localFilter(filters.boundary),
  "just nim-boundary-ci",
);
check(
  hook("rust-tests").includes('exclude: ["crates/nimino-boundary/**"]') &&
    hook("desktop-tauri-checks").includes(
      'exclude: ["crates/nimino-boundary/**"]',
    ),
  "general Rust hooks must exclude the focused boundary crate",
);
check(
  !/^\s+mobile-(?:fmt|checks):$/m.test(hooks),
  "Mobile git-hook lanes must be removed",
);

console.log(
  "CI lane contract passed: Nim-only is Rust-free; boundary is integrated; Mobile has no lane",
);
