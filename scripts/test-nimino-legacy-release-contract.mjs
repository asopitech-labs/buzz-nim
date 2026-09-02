#!/usr/bin/env node

import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";

const manifest = JSON.parse(
  readFileSync("contracts/nimino-legacy-release/v1/manifest.json", "utf8"),
);
const runbook = readFileSync(
  "contracts/nimino-legacy-release/README.md",
  "utf8",
);
const releasing = readFileSync(manifest.sourceOfTruth, "utf8");

assert.equal(manifest.schemaVersion, 1);
assert.equal(manifest.contract, "nimino.legacy-release");
assert.equal(manifest.issue, 65);
assert.equal(manifest.phase, "removed");
assert.equal(manifest.compatibilityMode, false);
assert.equal(manifest.physicalDeletionOwner, 66);
assert.equal(manifest.physicalPromotionOwner, 68);

function sorted(values) {
  return [...values].sort((left, right) => left.localeCompare(right));
}

function unique(values, label) {
  assert.equal(new Set(values).size, values.length, `duplicate ${label}`);
}

const workflowPaths = manifest.workflowInventory.map(({ path }) => path);
const trackedWorkflows = readdirSync(".github/workflows")
  .filter((name) => name.endsWith(".yml") || name.endsWith(".yaml"))
  .map((name) => join(".github/workflows", name));
unique(workflowPaths, "workflow path");
assert.deepEqual(
  sorted(
    manifest.workflowInventory
      .filter(({ action }) => action === "keep")
      .map(({ path }) => path),
  ),
  sorted(trackedWorkflows),
  "surviving workflow inventory is not exact",
);
for (const workflow of manifest.workflowInventory) {
  assert.ok(["delete", "keep", "rename"].includes(workflow.action));
  if (workflow.action === "delete" || workflow.action === "rename")
    assert.ok(workflow.replacement, `missing replacement for ${workflow.path}`);
  assert.equal(
    existsSync(workflow.path),
    workflow.action === "keep",
    `workflow deletion state drifted: ${workflow.path}`,
  );
}

const releaseKeep = manifest.workflowInventory
  .filter(
    ({ action, path }) =>
      action === "keep" &&
      path !== ".github/workflows/ci.yml" &&
      path !== ".github/workflows/nimino-benchmark.yml",
  )
  .map(({ path }) => path);
assert.deepEqual(releaseKeep, [
  ".github/workflows/nimino-platform-release.yml",
  ".github/workflows/nimino-promote.yml",
  ".github/workflows/nimino-relay-release.yml",
]);
const legacyTarget =
  /(?:block\/(?:buzz|nimino)|squareup\/|ghcr\.io\/block\/|sqprod\.co|block\.xyz|sprout-oss|artifactory|buildkite)/i;
for (const path of releaseKeep)
  assert.ok(
    !legacyTarget.test(readFileSync(path, "utf8")),
    `${path} has a legacy target`,
  );

const scriptPaths = manifest.scriptInventory.map(({ path }) => path);
unique(scriptPaths, "script path");
for (const script of manifest.scriptInventory) {
  assert.ok(["delete", "keep"].includes(script.action));
  assert.equal(
    existsSync(script.path),
    script.action === "keep",
    `script state drifted: ${script.path}`,
  );
}
const referencedCredentials = new Set();
for (const path of trackedWorkflows) {
  const source = readFileSync(path, "utf8");
  for (const match of source.matchAll(/\b(secrets|vars)\.([A-Z][A-Z0-9_]*)/g))
    referencedCredentials.add(
      `${match[1] === "vars" ? "variable" : "secret"}:${match[2]}`,
    );
}
const classifiedCredentials = manifest.credentialInventory.map(
  ({ kind, name }) => `${kind}:${name}`,
);
unique(classifiedCredentials, "credential");
assert.deepEqual(
  sorted(
    manifest.credentialInventory
      .filter(({ action }) => action === "keep")
      .map(({ kind, name }) => `${kind}:${name}`),
  ),
  sorted(referencedCredentials),
  "surviving workflow credential inventory is not exact",
);
for (const { kind, name, action } of manifest.credentialInventory)
  if (action === "delete")
    assert.ok(
      !referencedCredentials.has(`${kind}:${name}`),
      `retired credential remains referenced: ${name}`,
    );

const internalPatterns = new RegExp(
  [
    "Block/Square",
    "block/(?:buzz|nimino)",
    "squareup/",
    "ghcr\\.io/block/",
    "sqprod\\.co",
    "block\\.xyz",
    "sprout-oss",
    "artifactory",
    "buildkite",
    "\\bECR\\b",
    "\\bBlox\\b",
  ].join("|"),
  "i",
);
const scanPaths = execFileSync(
  "git",
  [
    "ls-files",
    ".github/workflows/**",
    "scripts/**",
    "deploy/**",
    "AGENTS.md",
    "CONTRIBUTING.md",
    "README.md",
    "RELEASING.md",
    "SECURITY.md",
  ],
  { encoding: "utf8" },
)
  .trim()
  .split("\n")
  .filter(Boolean)
  .filter((path) => existsSync(path))
  .filter((path) => path !== "scripts/test-nimino-legacy-release-contract.mjs");
const foundInternalPaths = scanPaths.filter((path) =>
  internalPatterns.test(readFileSync(path, "utf8")),
);
assert.deepEqual(
  sorted(
    Object.entries(manifest.internalReferenceClassification)
      .filter(([, action]) => action !== "delete")
      .map(([path]) => path),
  ),
  sorted(foundInternalPaths),
  "internal reference paths are missing or unclassified",
);
for (const [path, action] of Object.entries(
  manifest.internalReferenceClassification,
))
  if (action === "delete")
    assert.ok(!existsSync(path), `retired internal path remains: ${path}`);

for (const target of [
  "squareup/buzz-releases",
  "squareup/sprout-oss",
  "squareup/block-coder-tf-stacks",
  "squareup/sprout-backend-blox",
  "ghcr.io/block/buzz",
  "ghcr.io/block/buzz-sprig",
  "oci://ghcr.io/block/buzz/charts",
])
  assert.ok(
    manifest.externalAuthorities.some((entry) => entry.target === target),
    `missing legacy authority: ${target}`,
  );

assert.ok(releasing.includes("Nimino has one release authority"));
assert.ok(
  releasing.includes("contracts/nimino-legacy-release/v1/manifest.json"),
);
for (const command of [
  "just legacy-release-deletion-contract",
  "gh workflow list",
  "gh secret list",
  "gh variable list",
])
  assert.ok(runbook.includes(command), `runbook misses ${command}`);

console.log(
  `Legacy release retirement verified: ${workflowPaths.length} workflows, ${scriptPaths.length} scripts, ${classifiedCredentials.length} credentials, 0 unclassified internal paths`,
);
