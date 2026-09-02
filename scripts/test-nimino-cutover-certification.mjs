#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync(
    "contracts/nimino-cutover-certification/v1/contract.json",
    "utf8",
  ),
);
const readiness = JSON.parse(
  readFileSync("contracts/nimino-cutover-readiness/v1/manifest.json", "utf8"),
);
const ci = readFileSync(contract.source.workflow, "utf8");
const relay = readFileSync(contract.relay.workflow, "utf8");
const platform = readFileSync(contract.platform.workflow, "utf8");
const promotion = readFileSync(contract.promotion.workflow, "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1 && contract.version === 1, "wrong contract version");
check(contract.issue === 67, "wrong certification owner");
check(contract.compatibilityMode === false, "compatibility mode is forbidden");
const sourceReady =
  readiness.phase === "frozen-for-cleanup" && readiness.sourceBlockers.length === 0;
check(
  contract.publicationState === (sourceReady ? "cutover-ready" : "blocked"),
  "certification publication state disagrees with source readiness",
);
check(
  contract.externalCertificationRequired === true,
  "source checks must not claim external certification",
);
for (const signal of [
  "actions/checkout@",
  "run: just nim-boundary-ci",
  "run: just protocol-contract",
  "run: just removed-client-contract",
  "cargo-deny check",
]) {
  check(ci.includes(signal), `missing clean source gate: ${signal}`);
}
for (const signal of [
  "nimino-cluster-scenarios",
  "nimino-sync-scenarios",
  "nimino-object-scenarios",
  "nimino-projection-scenarios",
  "nimino-data-ops-scenarios",
  "helm upgrade --install nimino",
]) {
  check(relay.includes(signal), `missing relay evidence: ${signal}`);
}
for (const platformName of contract.platform.desktopMatrix) {
  check(platform.includes(platformName), `missing platform target: ${platformName}`);
}
for (const signal of [
  contract.platform.wslQualification,
  "just wsl-bundle-certify",
  "candidate/SHA256SUMS",
  "nimino-release.spdx.json",
  "cosign sign-blob",
  "actions/attest@",
  `name: ${contract.evidenceArtifact}`,
  "just cutover-certify-source",
]) {
  check(platform.includes(signal), `missing platform evidence: ${signal}`);
}
check(
  relay.includes("just cutover-certify-source"),
  "relay tag publication must require source readiness",
);
for (const path of Object.values(contract.compatibilityNegative)) {
  JSON.parse(readFileSync(path, "utf8"));
}
check(
  contract.promotion.issue === 68 &&
    contract.promotion.automatic === false &&
    promotion.includes("workflow_dispatch:") &&
    !platform.includes("nimino-promote-release.mjs promote"),
  "candidate certification must not perform promotion",
);

if (process.argv.includes("--require-ready") && !sourceReady) {
  throw new Error(
    `source certification blocked: ${readiness.sourceBlockers.map(({ id }) => id).join(", ")}`,
  );
}

console.log(
  sourceReady
    ? "Nimino cutover certification definition verified: source ready; external certification still required"
    : `Nimino cutover certification definition verified: source blocked by ${readiness.sourceBlockers.map(({ id }) => id).join(", ")}`,
);
