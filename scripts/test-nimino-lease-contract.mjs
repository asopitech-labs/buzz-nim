#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-lease/v1/contract.json", "utf8"),
);
const control = JSON.parse(
  readFileSync("contracts/nimino-control-state/v1/contract.json", "utf8"),
);
const policy = readFileSync(contract.module, "utf8");
const imports = policy
  .split("\n")
  .filter((line) => /^(?:import|from)\s/.test(line))
  .join("\n");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong lease contract schema");
check(contract.version === 1, "wrong lease contract version");
check(contract.contract === "nimino.lease-fencing", "wrong lease contract id");
check(contract.compatibilityMode === false, "lease compatibility mode is forbidden");
check(contract.owner === "nimino-core", "Nim must own lease decisions");
check(
  contract.controlStateMachine === `${control.contract}/v${control.version}` &&
    control.leaseContract === `${contract.contract}/v${contract.version}`,
  "control and lease contracts are not linked",
);
check(
  contract.grant.authority === "committed-control-entry" &&
    contract.grant.planMutatesState === false &&
    contract.grant.uncommittedActivation === false,
  "uncommitted lease activation is forbidden",
);
check(
  contract.serving.allSingletonConsumerGate === "authorizeSingletonEffect" &&
    contract.serving.liveQuorumRequired === true &&
    contract.serving.grantingTermRequired === true &&
    contract.serving.grantingVoterEpochRequired === true,
  "singleton serving gate drifted",
);
check(
  contract.clock.source === "adapter-supplied-monotonic-ticks" &&
    contract.clock.wallClockAuthority === false &&
    contract.clock.recoveryReactivatesLease === false,
  "clock or recovery safety drifted",
);
check(contract.transportProvidesAuthority === false, "Chirps cannot grant authority");
for (const symbol of [
  "planLeaseGrant*",
  "applyCommittedLease*",
  "routeSingleton*",
  "authorizeSingletonEffect*",
  "lfeStaleFence",
  "lfeFutureFence",
  "lfeOwnerMismatch",
  "lfeQuorumUnavailable",
]) {
  check(policy.includes(symbol), `missing Nim lease symbol: ${symbol}`);
}
check(
  !/\b(?:alopex|chirps|redb|redis|postgres|sqlx)\b/i.test(imports),
  "pure lease policy imports an adapter or transport",
);
check(contract.conflictResolutionOwner === 53, "wrong conflict owner");
check(contract.scenarioHarnessOwner === 56, "wrong scenario owner");
check(contract.legacyRemovalOwner === 12, "wrong cutover owner");

console.log("Nimino lease and fencing contract verified");
