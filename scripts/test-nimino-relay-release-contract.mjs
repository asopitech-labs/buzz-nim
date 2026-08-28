#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-relay-release/v1/contract.json", "utf8"),
);
const releaseSet = JSON.parse(
  readFileSync("contracts/nimino-release-set/v1/contract.json", "utf8"),
);
const naming = JSON.parse(
  readFileSync("contracts/nimino-naming-v1.json", "utf8"),
);
const workflow = readFileSync(
  ".github/workflows/nimino-relay-release.yml",
  "utf8",
);
const legacyDocker = readFileSync(".github/workflows/docker.yml", "utf8");
const legacyChart = readFileSync(".github/workflows/helm-chart.yml", "utf8");
const legacyAutoTag = readFileSync(
  ".github/workflows/auto-tag-on-release-pr-merge.yml",
  "utf8",
);
const dockerfile = readFileSync("Dockerfile", "utf8");
const chart = readFileSync("deploy/charts/nimino/Chart.yaml", "utf8");
const values = readFileSync("deploy/charts/nimino/values.yaml", "utf8");
const compose = readFileSync("deploy/compose/compose.yml", "utf8");
const composeRunner = readFileSync("deploy/compose/run.sh", "utf8");
const justfile = readFileSync("Justfile", "utf8");
const hooks = readFileSync("lefthook.yml", "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function canonical(id) {
  return naming.surfaces.find((surface) => surface.id === id)?.canonical;
}

function triggers(source) {
  const start = source.indexOf("\non:\n");
  const end = source.indexOf("\npermissions:", start);
  return source.slice(start, end);
}

check(contract.schemaVersion === 1, "wrong relay release schema");
check(
  contract.contract === "nimino.relay-release",
  "wrong relay release contract",
);
check(contract.issue === 61, "relay release ownership drifted");
check(
  contract.releaseSetAuthority === releaseSet.contract,
  "release-set authority drifted",
);
check(
  contract.compatibilityMode === false,
  "relay release compatibility mode is forbidden",
);
check(
  contract.tagPrefix === releaseSet.tagPrefix,
  "release tag authority drifted",
);
check(
  contract.sourceRepository === canonical("repository.slug"),
  "source repository drifted",
);
check(
  contract.image.repository ===
    canonical("release.container-repository.relay") &&
    JSON.stringify(contract.image.platforms) ===
      JSON.stringify(["linux/amd64", "linux/arm64"]),
  "relay image matrix drifted",
);
check(
  contract.chart.name === canonical("release.helm-chart.relay") &&
    contract.chart.repository === canonical("release.helm-chart-repository"),
  "relay chart identity drifted",
);
check(contract.compose.project === "nimino-prod", "Compose project drifted");
check(
  contract.compose.imagePolicy === "sha256-digest-required",
  "Compose image policy drifted",
);
check(contract.physicalPromotionOwner === 63, "wrong promotion owner");
check(
  contract.legacyPipelineDeletionOwner === 65,
  "wrong legacy deletion owner",
);
for (const proof of [
  "release-set-artifact-digests",
  "native-amd64-arm64-image",
  "container-readiness",
  "clean-chart-install",
  "compose-config",
  "three-node-chirps-negotiation",
  "three-node-data-convergence",
  "keyless-signatures",
  "provenance-attestation",
]) {
  check(
    contract.requiredProofs.includes(proof),
    `missing relay release proof: ${proof}`,
  );
}
for (const signal of [
  'tags: ["nimino-v[0-9]*"]',
  "linux/amd64",
  "linux/arm64",
  "push-by-digest=true",
  "CANONICAL_REPOSITORY: asopitech-labs/nimino",
  "nimino-cluster-scenarios",
  "nimino-data-ops-scenarios",
  "helm upgrade --install nimino",
  "nimino-release-set.mjs",
  "cosign sign --yes",
  "cosign sign-blob --yes",
  "attest-build-provenance",
]) {
  check(
    workflow.includes(signal),
    `missing relay release pipeline signal: ${signal}`,
  );
}
for (const forbidden of [
  "ghcr.io/block/buzz",
  "relay-v",
  "chart-v",
  ":latest",
  ":main",
]) {
  check(
    !workflow.includes(forbidden),
    `legacy relay publication remains: ${forbidden}`,
  );
}
for (const [name, source] of [
  ["legacy Docker", legacyDocker],
  ["legacy chart", legacyChart],
]) {
  check(
    !triggers(source).includes("push:") &&
      !triggers(source).includes("workflow_dispatch:"),
    `${name} publisher remains enabled`,
  );
}
check(
  !legacyAutoTag.includes("relay-release/*") &&
    !legacyAutoTag.includes("chart-release/*") &&
    !legacyAutoTag.includes('TAG_PREFIX="chart-v"'),
  "legacy relay/chart auto-tag lane remains enabled",
);
check(
  dockerfile.includes('org.opencontainers.image.title="Nimino"'),
  "container identity drifted",
);
check(
  dockerfile.includes(
    "apt-get install -y --no-install-recommends ca-certificates",
  ) &&
    dockerfile.includes(
      "ENV NODE_EXTRA_CA_CERTS=/etc/ssl/certs/ca-certificates.crt",
    ),
  "web builder CA trust is missing",
);
check(
  dockerfile.includes("groupadd --gid 1000 nimino") &&
    dockerfile.includes("useradd  --uid 1000 --gid 1000"),
  "runtime non-root identity drifted",
);
check(chart.includes("name: nimino"), "chart name drifted");
check(
  values.includes(`repository: ${contract.image.repository}`),
  "chart image drifted",
);
check(
  compose.includes(`name: ${contract.compose.project}`),
  "Compose identity drifted",
);
check(
  compose.includes(
    `image: ${contract.image.repository}@\${NIMINO_IMAGE_DIGEST:?set NIMINO_IMAGE_DIGEST to sha256:<digest>}`,
  ),
  "Compose image drifted",
);
check(
  composeRunner.includes("^sha256:[0-9a-f]{64}$"),
  "Compose runner does not reject tags and shortened digests",
);
check(
  /^relay-release-contract:\n {4}node scripts\/test-nimino-relay-release-contract\.mjs$/m.test(
    justfile,
  ) && hooks.includes("relay-release-contract:"),
  "relay release source gates are missing",
);

console.log(
  "Nimino relay release contract passed: multi-arch image, chart, compose, evidence, signing",
);
