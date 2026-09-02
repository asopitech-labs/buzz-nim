#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = process.cwd();
const cli = join(root, "scripts/nimino-release-supply-chain.mjs");
const releaseSetCli = join(root, "scripts/nimino-release-set.mjs");
const work = mkdtempSync(join(tmpdir(), "nimino-supply-chain-"));
const artifact = join(work, "nimino");
const releaseSet = join(work, "release-set.json");
const sbom = join(work, "nimino-release.spdx.json");
const sums = join(work, "SHA256SUMS");
const commit = "a".repeat(40);
writeFileSync(artifact, "immutable artifact\n");

function run(program, ...parameters) {
  return spawnSync(process.execPath, [program, ...parameters], {
    cwd: root,
    encoding: "utf8",
  });
}

assert.equal(
  run(
    releaseSetCli,
    "create",
    "--version",
    "1.2.3",
    "--tag",
    "nimino-v1.2.3",
    "--commit",
    commit,
    "--output",
    releaseSet,
    "--artifact",
    `cli:1.2.3:${artifact}`,
  ).status,
  0,
);
assert.equal(
  run(cli, "sbom", "--release-set", releaseSet, "--output", sbom).status,
  0,
);
writeFileSync(join(work, "release-set.sigstore.json"), "signature\n");
assert.equal(
  run(cli, "checksums", "--directory", work, "--output", sums).status,
  0,
);
const verifyArgs = [
  "verify",
  "--release-set",
  releaseSet,
  "--artifact-dir",
  work,
  "--sbom",
  sbom,
  "--checksums",
  sums,
  "--resolved-tag-commit",
  commit,
];
assert.equal(run(cli, ...verifyArgs).status, 0);
const document = JSON.parse(readFileSync(sbom, "utf8"));
assert.equal(document.spdxVersion, "SPDX-2.3");
assert.equal(document.packages[0].name, "Nimino");
assert.equal(document.files[0].fileName, "nimino");
assert.equal(document.files[0].checksums[0].checksumValue.length, 64);

writeFileSync(artifact, "tampered\n");
assert.notEqual(run(cli, ...verifyArgs).status, 0);
writeFileSync(artifact, "immutable artifact\n");

const originalSbom = readFileSync(sbom, "utf8");
writeFileSync(sbom, originalSbom.replace("Nimino 1.2.3", "forged"));
assert.notEqual(run(cli, ...verifyArgs).status, 0);
writeFileSync(sbom, originalSbom);

const originalSums = readFileSync(sums, "utf8");
writeFileSync(
  sums,
  `${originalSums[0] === "0" ? "1" : "0"}${originalSums.slice(1)}`,
);
assert.notEqual(run(cli, ...verifyArgs).status, 0);

console.log(
  "Nimino supply-chain tests passed: deterministic SPDX, complete checksums, tamper rejection",
);
