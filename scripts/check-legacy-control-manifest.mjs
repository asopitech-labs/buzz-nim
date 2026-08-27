#!/usr/bin/env node

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const manifest = JSON.parse(
  readFileSync("contracts/nimino-legacy-control/v1/manifest.json", "utf8"),
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function filesUnder(root, predicate = () => true) {
  const result = [];
  for (const name of readdirSync(root)) {
    const path = join(root, name);
    if (statSync(path).isDirectory()) result.push(...filesUnder(path, predicate));
    else if (predicate(path)) result.push(path);
  }
  return result.sort();
}

function paths(entries) {
  return entries.map((entry) => entry.path).sort();
}

function samePaths(actual, expected, label) {
  check(
    JSON.stringify(actual.sort()) === JSON.stringify(expected.sort()),
    `${label} coverage drifted\nactual=${JSON.stringify(actual)}\nmanifest=${JSON.stringify(expected)}`,
  );
}

check(manifest.schemaVersion === 1 && manifest.version === 1, "wrong manifest version");
check(
  manifest.contract === "nimino.legacy-control-replacement" &&
    manifest.compatibilityMode === false &&
    manifest.publicationState === "cutover-ready",
  "legacy control cutover contract drifted",
);
check(manifest.physicalRemovalOwner === 66, "#66 must own physical removal");

const meshFiles = filesUnder("crates/buzz-relay-mesh/src", (path) => path.endsWith(".rs"));
samePaths(meshFiles, paths(manifest.mesh.crateFiles), "legacy mesh crate");
const redisFiles = filesUnder("crates/buzz-pubsub/src", (path) => path.endsWith(".rs"));
samePaths(redisFiles, paths(manifest.redis.crateFiles), "Redis crate");

const relayFiles = filesUnder("crates/buzz-relay/src", (path) => path.endsWith(".rs"));
const meshConsumers = relayFiles.filter((path) => readFileSync(path, "utf8").includes("buzz_relay_mesh"));
samePaths(meshConsumers, paths(manifest.mesh.consumers), "legacy mesh consumers");

const redisConsumerRoots = ["crates/buzz-relay/src", "crates/buzz-admin/src"];
const redisPattern = /buzz_pubsub|PubSubManager|RedisNip98ReplayGuard|RedisRateLimiter/;
const redisConsumers = redisConsumerRoots
  .flatMap((root) => filesUnder(root, (path) => path.endsWith(".rs")))
  .filter((path) => redisPattern.test(readFileSync(path, "utf8")))
  .sort();
samePaths(redisConsumers, paths(manifest.redis.consumers), "Redis consumers");

const allEntries = [
  ...manifest.mesh.crateFiles,
  ...manifest.mesh.consumers,
  ...manifest.redis.crateFiles,
  ...manifest.redis.consumers,
  ...manifest.buildWiring,
];
for (const entry of allEntries) {
  check(existsSync(entry.path), `manifest path does not exist: ${entry.path}`);
  check(["replace", "keep", "delete"].includes(entry.action), `invalid action: ${entry.path}`);
  check(Number.isInteger(entry.ownerIssue) && entry.ownerIssue > 0, `missing owner: ${entry.path}`);
  check(typeof entry.target === "string" && entry.target.length > 0, `missing target: ${entry.path}`);
}
check(
  [...manifest.mesh.crateFiles, ...manifest.redis.crateFiles].every(
    (entry) => entry.action !== "keep",
  ),
  "legacy crate wrappers may not be kept",
);
check(manifest.redis.sharedTargetTopologyAllowed === false, "shared Redis must leave target topology");
check(
  manifest.redis.optionalLocalCaches.every(
    (cache) =>
      cache.authoritative === false &&
      cache.scope === "single-node" &&
      cache.ownerIssue === 10,
  ),
  "retained cache semantics must remain non-authoritative, node-local, and owned by #10",
);

const targetFiles = [
  ...filesUnder("nim", (path) => path.endsWith(".nim")),
  ...filesUnder("crates/nimino-boundary", (path) => /\.(rs|toml)$/.test(path)),
  ...filesUnder("crates/nimino-chirps", (path) => /\.(rs|toml)$/.test(path)),
  ...filesUnder("crates/nimino-store", (path) => /\.(rs|toml)$/.test(path)),
  ...filesUnder("crates/nimino-object-store", (path) => /\.(rs|toml)$/.test(path)),
  ...filesUnder("crates/nimino-data-ops", (path) => /\.(rs|toml)$/.test(path)),
];
for (const symbol of manifest.forbiddenTargetSymbols) {
  const leaks = targetFiles.filter((path) => readFileSync(path, "utf8").includes(symbol));
  check(leaks.length === 0, `legacy symbol ${symbol} leaked into target: ${leaks.join(", ")}`);
}
check(
  manifest.ownerReview.status === "ready-for-owner-review-at-issue-66",
  "owner review handoff must remain explicit",
);

console.log(
  `Legacy control replacement manifest verified: ${allEntries.length} paths, no shared Redis target`,
);
