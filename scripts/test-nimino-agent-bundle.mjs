#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  unlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = process.cwd();
const releaseCli = join(root, "scripts/nimino-release-set.mjs");
const bundleCli = join(root, "scripts/nimino-agent-bundle.mjs");
const work = mkdtempSync(join(tmpdir(), "nimino-agent-bundle-"));
const artifacts = join(work, "artifacts");
const output = join(work, "installed");
const releaseSet = join(work, "release-set.json");
const commit = "a".repeat(40);

function run(command, args) {
  return spawnSync(command, args, { cwd: root, encoding: "utf8" });
}

function stub(name) {
  const path = join(artifacts, name);
  writeFileSync(
    path,
    `#!/usr/bin/env node\nconsole.log(${JSON.stringify(name)} + ":" + process.argv.slice(2).join(","));\n`,
  );
  chmodSync(path, 0o755);
  return path;
}

mkdirSync(artifacts);
const inputs = [
  ["acp", stub("buzz-acp")],
  ["agent", stub("buzz-agent")],
  ["cli", stub("nimino")],
  ["mcp", stub("buzz-dev-mcp")],
  ["relay", stub("nimino-relay")],
];
const created = run(process.execPath, [
  releaseCli,
  "create",
  "--version",
  "1.0.0",
  "--tag",
  "nimino-v1.0.0",
  "--commit",
  commit,
  "--output",
  releaseSet,
  ...inputs.flatMap(([id, path]) => ["--artifact", `${id}:0.1.0:${path}`]),
]);
assert.equal(created.status, 0, created.stderr);
unlinkSync(join(artifacts, "nimino-relay"));

const composed = run(process.execPath, [
  bundleCli,
  "compose",
  "--release-set",
  releaseSet,
  "--resolved-tag-commit",
  commit,
  "--artifact-dir",
  artifacts,
  "--output",
  output,
]);
assert.equal(composed.status, 0, composed.stderr);
assert.deepEqual(readdirSync(output).sort(), [
  "bin",
  "nimino-agent-bundle.json",
]);
assert.deepEqual(readdirSync(join(output, "bin")).sort(), [
  "nimino",
  "nimino-acp",
  "nimino-agent",
  "nimino-dev-mcp",
]);

const manifest = JSON.parse(
  readFileSync(join(output, "nimino-agent-bundle.json"), "utf8"),
);
assert.equal(manifest.releaseSetId.length, 64);
assert.equal(manifest.version, "1.0.0");
assert.deepEqual(
  manifest.components.map(({ role, installName }) => [role, installName]),
  [
    ["cli", "nimino"],
    ["acp", "nimino-acp"],
    ["agent", "nimino-agent"],
    ["mcp", "nimino-dev-mcp"],
  ],
);
for (const component of manifest.components) {
  assert.match(component.version, /^\d+\.\d+\.\d+$/);
  assert.match(component.sha256, /^[0-9a-f]{64}$/);
  const smoke = run(join(output, "bin", component.installName), ["--version"]);
  assert.equal(smoke.status, 0, smoke.stderr);
  assert.match(smoke.stdout, new RegExp(`^${component.sourceFile}:--version`));
}

const incompleteReleaseSet = join(work, "incomplete-release-set.json");
const incomplete = run(process.execPath, [
  releaseCli,
  "create",
  "--version",
  "1.0.0",
  "--tag",
  "nimino-v1.0.0",
  "--commit",
  commit,
  "--output",
  incompleteReleaseSet,
  ...inputs
    .filter(([id]) => id !== "mcp" && id !== "relay")
    .flatMap(([id, path]) => ["--artifact", `${id}:0.1.0:${path}`]),
]);
assert.equal(incomplete.status, 0, incomplete.stderr);
const missing = run(process.execPath, [
  bundleCli,
  "compose",
  "--release-set",
  incompleteReleaseSet,
  "--resolved-tag-commit",
  commit,
  "--artifact-dir",
  artifacts,
  "--output",
  join(work, "missing-output"),
]);
assert.notEqual(missing.status, 0);
assert.match(missing.stderr, /required bundle artifact is missing: mcp/i);

console.log(
  "Nimino agent bundle tests passed: inventory, smoke, missing component",
);
