#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";

const root = process.cwd();
const cli = join(root, "scripts/nimino-platform-artifacts.mjs");
const releaseSetCli = join(root, "scripts/nimino-release-set.mjs");
const wslCli = join(root, "scripts/nimino-wsl-bundle.mjs");
const work = mkdtempSync(join(tmpdir(), "nimino-platform-release-"));
const source = join(work, "source");
const staged = join(work, "staged");
const artifacts = join(work, "artifacts");
const version = "1.2.3";
const commit = "a".repeat(40);
const endpoint =
  "https://github.com/asopitech-labs/nimino/releases/download/nimino-desktop-latest/latest.json";
const contract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-platform-release/v1/contract.json"),
    "utf8",
  ),
);
const releaseSetContract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-release-set/v1/contract.json"),
    "utf8",
  ),
);
const wslContract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-wsl-bundle/v1/contract.json"),
    "utf8",
  ),
);
const workflow = readFileSync(
  join(root, ".github/workflows/nimino-platform-release.yml"),
  "utf8",
);
const relayWorkflow = readFileSync(
  join(root, ".github/workflows/nimino-relay-release.yml"),
  "utf8",
);
mkdirSync(source);
mkdirSync(staged);
mkdirSync(artifacts);

assert.equal(contract.schemaVersion, 1);
assert.equal(contract.contract, "nimino.platform-release");
assert.equal(contract.issue, 62);
assert.equal(contract.releaseSetAuthority, releaseSetContract.contract);
assert.equal(contract.compatibilityMode, false);
assert.equal(contract.nativeWindows, false);
assert.equal(contract.updaterEndpoint, endpoint);
assert.deepEqual(
  contract.platforms.map(({ id, runner }) => [id, runner]),
  [
    ["macos-aarch64", "macos-15"],
    ["macos-x86-64", "macos-15-intel"],
    ["linux-x86-64", "ubuntu-24.04"],
  ],
);
assert.deepEqual(
  contract.wsl.componentArtifactIds,
  wslContract.components
    .map(({ artifactId }) => artifactId)
    .sort((left, right) => left.localeCompare(right)),
);
for (const signal of [
  'tags: ["nimino-v[0-9]*"]',
  "uses: ./.github/workflows/nimino-relay-release.yml",
  "nimino-platform-artifacts.mjs preflight",
  "TAURI_SIGNING_PRIVATE_KEY",
  "APPLE_CERTIFICATE",
  "codesign --verify --deep --strict",
  "xcrun stapler validate",
  "nimino-release-set.mjs create",
  "nimino-wsl-bundle.mjs compose",
  "just wsl-bundle-certify",
  "attest-build-provenance",
  "--draft --verify-tag",
])
  assert.ok(
    workflow.includes(signal),
    `missing platform pipeline signal: ${signal}`,
  );
assert.ok(
  workflow.indexOf("nimino-platform-artifacts.mjs preflight") <
    workflow.indexOf("tauri-apps/tauri-action"),
  "signing preflight must precede the desktop build",
);
for (const forbidden of [
  "windows-latest",
  "windows-x86_64",
  "block/apple-codesign-action",
  "github.com/block/buzz",
  "desktop-v",
])
  assert.ok(
    !workflow.includes(forbidden),
    `legacy platform lane remains: ${forbidden}`,
  );
assert.match(relayWorkflow, /^ {2}workflow_call:$/m);
assert.doesNotMatch(relayWorkflow, /^ {2}push:$/m);

function run(program, parameters, env = {}) {
  return spawnSync(process.execPath, [program, ...parameters], {
    cwd: root,
    encoding: "utf8",
    env: { ...process.env, ...env },
  });
}

function platform(...parameters) {
  return run(cli, parameters);
}

assert.notEqual(platform("preflight", "--platform", "linux-x86-64").status, 0);
assert.equal(
  run(cli, ["preflight", "--platform", "linux-x86-64"], {
    NIMINO_UPDATER_ENDPOINT: endpoint,
    NIMINO_UPDATER_PUBLIC_KEY: "public",
    TAURI_SIGNING_PRIVATE_KEY: "private",
  }).status,
  0,
);

const fixtures = [
  ["macos-aarch64", "Nimino.dmg", "Nimino.app.tar.gz"],
  ["macos-x86-64", "Nimino.dmg", "Nimino.app.tar.gz"],
  ["linux-x86-64", "Nimino.AppImage"],
];
for (const [id, ...names] of fixtures) {
  const input = join(source, id);
  const output = join(staged, id);
  mkdirSync(input);
  for (const name of names) {
    writeFileSync(join(input, name), `${id}:${name}\n`);
    if (name.endsWith(".tar.gz") || name.endsWith(".AppImage"))
      writeFileSync(join(input, `${name}.sig`), `${id}-signature\n`);
  }
  assert.equal(
    platform(
      "stage",
      "--platform",
      id,
      "--version",
      version,
      "--bundle-dir",
      input,
      "--output",
      output,
    ).status,
    0,
  );
  for (const name of readdirSync(output))
    if (name !== "platform-artifacts.json")
      copyFileSync(join(output, name), join(artifacts, name));
}

writeFileSync(join(source, "linux-x86-64", "duplicate.AppImage"), "duplicate");
assert.notEqual(
  platform(
    "stage",
    "--platform",
    "linux-x86-64",
    "--version",
    version,
    "--bundle-dir",
    join(source, "linux-x86-64"),
    "--output",
    join(work, "duplicate-output"),
  ).status,
  0,
);

const specifications = [];
for (const [id] of fixtures) {
  const map = JSON.parse(
    readFileSync(join(staged, id, "platform-artifacts.json"), "utf8"),
  );
  for (const artifact of map.artifacts)
    specifications.push({
      id: `desktop-${id}-${artifact.role}`,
      path: join(artifacts, artifact.fileName),
    });
}
for (const id of [
  "acp",
  "agent",
  "cli",
  "core-worker",
  "data-ops",
  "mcp",
  "relay",
]) {
  const path = join(artifacts, `nimino-${id}`);
  writeFileSync(path, `${id}\n`);
  specifications.push({ id, path });
}
const releaseSet = join(work, "release-set.json");
assert.equal(
  run(releaseSetCli, [
    "create",
    "--version",
    version,
    "--tag",
    `nimino-v${version}`,
    "--commit",
    commit,
    "--output",
    releaseSet,
    ...specifications.flatMap(({ id, path }) => [
      "--artifact",
      `${id}:${version}:${path}`,
    ]),
  ]).status,
  0,
);

const bundle = join(work, "bundle");
assert.equal(
  run(wslCli, [
    "compose",
    "--release-set",
    releaseSet,
    "--resolved-tag-commit",
    commit,
    "--artifact-dir",
    artifacts,
    "--output",
    bundle,
  ]).status,
  0,
);
const archive = join(artifacts, `nimino-${version}-wsl2-x86_64.tar.gz`);
writeFileSync(archive, "deterministic archive fixture\n");
const latest = join(work, "latest.json");
const manifestArgs = [
  "manifest",
  "--release-set",
  releaseSet,
  "--artifact-dir",
  artifacts,
  "--base-url",
  `https://github.com/asopitech-labs/nimino/releases/download/nimino-v${version}`,
  "--wsl-bundle-dir",
  bundle,
  "--wsl-archive",
  archive,
  "--output",
  latest,
];
assert.equal(platform(...manifestArgs).status, 0);
const update = JSON.parse(readFileSync(latest, "utf8"));
assert.equal(
  update.releaseSetId,
  JSON.parse(readFileSync(releaseSet, "utf8")).releaseSetId,
);
assert.deepEqual(Object.keys(update.platforms).sort(), [
  "darwin-aarch64",
  "darwin-x86_64",
  "linux-x86_64",
]);
assert.equal(update.wsl.platform, "wsl2-x86_64");
assert.equal(basename(new URL(update.wsl.url).pathname), basename(archive));

const bundleManifestPath = join(bundle, "nimino-wsl-bundle.json");
const bundleManifest = JSON.parse(readFileSync(bundleManifestPath, "utf8"));
bundleManifest.releaseSetId = "0".repeat(64);
writeFileSync(
  bundleManifestPath,
  `${JSON.stringify(bundleManifest, null, 2)}\n`,
);
assert.notEqual(platform(...manifestArgs).status, 0);

console.log(
  "Nimino platform release tests passed: signing, matrix, updater, WSL release-set",
);
