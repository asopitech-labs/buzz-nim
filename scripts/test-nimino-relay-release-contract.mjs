#!/usr/bin/env node

import { existsSync, readFileSync } from "node:fs";

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
const dockerfile = readFileSync("Dockerfile", "utf8");
const chart = readFileSync("deploy/charts/nimino/Chart.yaml", "utf8");
const values = readFileSync("deploy/charts/nimino/values.yaml", "utf8");
const compose = readFileSync("deploy/compose/compose.yml", "utf8");
const composeRunner = readFileSync("deploy/compose/run.sh", "utf8");
const justfile = readFileSync("Justfile", "utf8");
const hooks = readFileSync("lefthook.yml", "utf8");
const relayMain = readFileSync("crates/nimino-relay/src/main.rs", "utf8");
const clusterRuntime = readFileSync(
  "crates/nimino-relay/src/cluster_runtime.rs",
  "utf8",
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function canonical(id) {
  return naming.surfaces.find((surface) => surface.id === id)?.canonical;
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
  "content-addressed-object-repair",
  "keyless-signatures",
  "provenance-attestation",
]) {
  check(
    contract.requiredProofs.includes(proof),
    `missing relay release proof: ${proof}`,
  );
}
for (const signal of [
  "workflow_call:",
  "linux/amd64",
  "linux/arm64",
  "push-by-digest=true",
  "CANONICAL_REPOSITORY: asopitech-labs/nimino",
  "just nimino-cluster-scenarios target/nim/nimino-cluster-scenarios.json",
  "just nimino-sync-scenarios target/nim/nimino-sync-scenarios.json",
  "nimino-object-scenarios",
  "nimino-projection-scenarios",
  "nimino-data-ops-scenarios",
  "helm upgrade --install nimino",
  "rollout status statefulset/nimino",
  "cosign sign --yes",
  "cosign sign-blob --yes",
  "nimino-relay-candidate",
]) {
  check(
    workflow.includes(signal),
    `missing relay release pipeline signal: ${signal}`,
  );
}
for (const forbidden of [
  "ghcr.io/block/buzz",
  "ghcr.io/block/nimino",
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
for (const path of [
  ".github/workflows/docker.yml",
  ".github/workflows/helm-chart.yml",
  ".github/workflows/auto-tag-on-release-pr-merge.yml",
]) {
  check(
    !existsSync(path),
    `legacy publisher remains: ${path}`,
  );
}
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
check(
  dockerfile.includes("nimino-core-worker") &&
    dockerfile.includes("NIMINO_BOUNDARY_WORKER=/usr/local/bin/nimino-core-worker") &&
    dockerfile.includes("EXPOSE 7443/udp"),
  "container does not compose the Nim worker and Chirps transport",
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
  compose.includes("NIMINO_CHIRPS_BIND_ADDR") &&
    compose.includes("nimino-cluster-data:/var/lib/nimino/cluster") &&
    compose.includes("./chirps:/etc/nimino/chirps:ro"),
  "Compose does not persist and configure the cluster runtime",
);
check(
  relayMain.includes("RelayClusterRuntime::start") &&
    /cluster_runtime\s*\.stop\(\)/.test(relayMain) &&
    clusterRuntime.includes("SyncRuntime::start") &&
    clusterRuntime.includes("MeshRuntime::start") &&
    clusterRuntime.includes("BoundaryRuntime::start"),
  "released relay does not own the Nim/Chirps/sync lifecycle",
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
