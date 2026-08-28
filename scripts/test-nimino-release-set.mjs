#!/usr/bin/env node

import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import assert from "node:assert/strict";

const root = process.cwd();
const cli = join(root, "scripts/nimino-release-set.mjs");
const work = mkdtempSync(join(tmpdir(), "nimino-release-set-"));
const commit = "a".repeat(40);
const movedCommit = "b".repeat(40);
const cliArtifact = join(work, "buzz");
const agentArtifact = join(work, "buzz-agent");
writeFileSync(cliArtifact, "cli-v1\n");
writeFileSync(agentArtifact, "agent-v1\n");

function run(...args) {
  return spawnSync(process.execPath, [cli, ...args], {
    cwd: root,
    encoding: "utf8",
  });
}

function create(output, version = "1.2.0", artifacts = []) {
  return run(
    "create",
    "--version",
    version,
    "--tag",
    `nimino-v${version}`,
    "--commit",
    commit,
    "--output",
    output,
    ...artifacts.flatMap(({ id, version: artifactVersion, path }) => [
      "--artifact",
      `${id}:${artifactVersion}:${path}`,
    ]),
  );
}

function verify(path, ...args) {
  return run(
    "verify",
    "--manifest",
    path,
    "--resolved-tag-commit",
    commit,
    ...args,
  );
}

const artifacts = [
  { id: "cli", version: "0.1.0", path: cliArtifact },
  { id: "agent", version: "0.1.0", path: agentArtifact },
];
const first = join(work, "first.json");
const second = join(work, "second.json");
assert.equal(create(first, "1.2.0", artifacts).status, 0);
assert.equal(create(second, "1.2.0", artifacts).status, 0);
assert.equal(readFileSync(first, "utf8"), readFileSync(second, "utf8"));
assert.equal(verify(first).status, 0);
assert.equal(verify(first, "--artifact-dir", work).status, 0);
assert.equal(verify(first, "--source-root", root).status, 0);
assert.notEqual(verify(first, "--source-root", work).status, 0);

const releaseSet = JSON.parse(readFileSync(first, "utf8"));
assert.match(releaseSet.releaseSetId, /^[0-9a-f]{64}$/);
assert.deepEqual(
  releaseSet.components.map(({ id }) => id),
  ["alopex-chirps", "boundary-schema", "nim-core", "rust-workspace"],
);
assert.deepEqual(
  releaseSet.artifacts.map(({ id }) => id),
  ["agent", "cli"],
);

const missing = create(join(work, "missing.json"), "1.2.0", [
  { id: "missing", version: "0.1.0", path: join(work, "missing") },
]);
assert.notEqual(missing.status, 0);
assert.match(missing.stderr, /artifact.*missing/i);

const tamperedPath = join(work, "tampered.json");
const tampered = structuredClone(releaseSet);
tampered.artifacts[0].sha256 = "0".repeat(64);
writeFileSync(tamperedPath, `${JSON.stringify(tampered, null, 2)}\n`);
assert.notEqual(verify(tamperedPath).status, 0);

writeFileSync(cliArtifact, "cli-tampered\n");
assert.notEqual(verify(first, "--artifact-dir", work).status, 0);
writeFileSync(cliArtifact, "cli-v1\n");

const moved = run(
  "verify",
  "--manifest",
  first,
  "--resolved-tag-commit",
  movedCommit,
);
assert.notEqual(moved.status, 0);
assert.match(moved.stderr, /tag.*commit/i);

const previous = join(work, "previous.json");
assert.equal(create(previous, "1.3.0", artifacts).status, 0);
assert.notEqual(verify(first, "--previous", previous).status, 0);

assert.equal(verify(first, "--previous", second).status, 0);
writeFileSync(agentArtifact, "agent-v2\n");
const sameVersionDifferent = join(work, "same-version-different.json");
assert.equal(create(sameVersionDifferent, "1.2.0", artifacts).status, 0);
assert.notEqual(verify(sameVersionDifferent, "--previous", first).status, 0);

console.log(
  "Nimino release-set tests passed: rerun, mismatch, tag-move, downgrade",
);
