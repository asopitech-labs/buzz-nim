#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-convergence/v1/contract.json", "utf8"),
);
const sync = JSON.parse(
  readFileSync("contracts/nimino-sync/v2/contract.json", "utf8"),
);
const event = JSON.parse(
  readFileSync("contracts/nimino-event/v1/contract.json", "utf8"),
);
const moderation = JSON.parse(
  readFileSync("contracts/nimino-moderation/v1/contract.json", "utf8"),
);
const policy = readFileSync(contract.module, "utf8");
const imports = policy
  .split("\n")
  .filter((line) => /^(?:import|from)\s/.test(line))
  .join("\n");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong convergence contract schema");
check(contract.version === 1, "wrong convergence contract version");
check(contract.contract === "nimino.convergence", "wrong convergence id");
check(contract.compatibilityMode === false, "convergence compatibility mode is forbidden");
check(contract.owner === "nimino-core", "Nim must own convergence policy");
check(
  contract.syncContract === `${sync.contract}/v${sync.version}` &&
    sync.conflictContract === `${contract.contract}/v${contract.version}`,
  "sync and convergence contracts are not linked",
);
check(
  contract.eventPolicyContract === `${event.contract}/v1` &&
    event.convergenceContract === `${contract.contract}/v${contract.version}` &&
    contract.moderationPolicyContract === `${moderation.contract}/v1` &&
    moderation.convergenceContract === `${contract.contract}/v${contract.version}`,
  "event/moderation convergence ownership drifted",
);
check(
  contract.identity.sameIdDifferentDigest ===
    "quarantine-and-remove-from-canonical-truth" &&
    contract.tombstones.staleReplayCanResurrect === false,
  "identity quarantine or anti-resurrection drifted",
);
check(
  JSON.stringify(contract.restrictions.equalRevisionOrder) ===
    JSON.stringify(["ban", "timeout", "release"]),
  "restrictive equal-revision order drifted",
);
check(
  contract.retention.merge === "component-wise-max" &&
    contract.retention.invariant ===
      "prunedThrough<=tombstoneProtectedThrough",
  "retention convergence drifted",
);
check(
  contract.propertyDomain === "valid-non-colliding-inputs" &&
    contract.properties.length === 3,
  "convergence algebra domain drifted",
);
for (const symbol of [
  "observeIdentity*",
  "mergeReplicaRecord*",
  "mergeRestriction*",
  "mergeRetention*",
  "cpeIdentityCollision",
  "tskPermanent",
]) {
  check(policy.includes(symbol), `missing Nim convergence symbol: ${symbol}`);
}
check(
  imports.includes("nimino_core/domain/event_policy") &&
    !/\b(?:alopex|chirps|redb|redis|postgres|sqlx)\b/i.test(imports),
  "convergence policy duplicated or imported an adapter",
);
check(contract.projectionRebuildOwner === 55, "wrong projection owner");
check(contract.scenarioHarnessOwner === 56, "wrong scenario owner");
check(contract.legacyRemovalOwner === 12, "wrong cutover owner");

console.log("Nimino deterministic convergence contract verified");
