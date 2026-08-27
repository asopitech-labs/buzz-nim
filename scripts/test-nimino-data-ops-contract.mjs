#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-data-ops/v1/contract.json", "utf8"),
);
const data = JSON.parse(readFileSync("contracts/nimino-data/v1/contract.json", "utf8"));
const sync = JSON.parse(readFileSync("contracts/nimino-sync/v1/contract.json", "utf8"));
const convergence = JSON.parse(
  readFileSync("contracts/nimino-convergence/v1/contract.json", "utf8"),
);
const projection = JSON.parse(
  readFileSync("contracts/nimino-projection/v1/contract.json", "utf8"),
);
const objects = JSON.parse(
  readFileSync("contracts/nimino-object-sync/v1/contract.json", "utf8"),
);
const effects = JSON.parse(
  readFileSync("contracts/nimino-effect-ledger/v1/contract.json", "utf8"),
);
const cluster = JSON.parse(
  readFileSync("contracts/nimino-cluster-scenarios/v1/contract.json", "utf8"),
);
const policy = readFileSync(contract.module, "utf8");
const cli = readFileSync("crates/nimino-data-ops/src/main.rs", "utf8");
const adapter = readFileSync("crates/nimino-data-ops/src/lib.rs", "utf8");
const scenarios = readFileSync(
  "crates/nimino-data-ops/tests/convergence_scenarios.rs",
  "utf8",
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1 && contract.version === 1, "wrong data-ops version");
check(contract.contract === "nimino.data-ops", "wrong data-ops contract");
check(contract.compatibilityMode === false, "data-ops compatibility mode is forbidden");
check(contract.owner === "nimino-core", "Nim must own repair authority");
check(
  contract.cli.crate === "nimino-data-ops" &&
    JSON.stringify(contract.cli.commands) === JSON.stringify(["verify", "repair"]) &&
    cli.includes("Command::Verify") &&
    cli.includes("Command::Repair"),
  "operator CLI contract drifted",
);

const id = `${contract.contract}/v${contract.version}`;
check(
  data.repairContract === id &&
    sync.repairContract === id &&
    projection.repairContract === id &&
    objects.repairContract === id &&
    effects.repairContract === id,
  "repair owner cross-links drifted",
);
check(
  contract.contracts.data === `${data.protocol}/v${data.version}` &&
    contract.contracts.sync === `${sync.contract}/v${sync.version}` &&
    contract.contracts.convergence === `${convergence.contract}/v${convergence.version}` &&
    contract.contracts.projection === `${projection.contract}/v${projection.version}` &&
    contract.contracts.objects === `${objects.contract}/v${objects.version}` &&
    contract.contracts.effects === `${effects.contract}/v${effects.version}` &&
    contract.contracts.clusterScenarios === `${cluster.contract}/v${cluster.version}`,
  "data-ops dependency contracts drifted",
);
check(
  contract.authority.selection === "strict-majority-identical-snapshot" &&
    JSON.stringify(contract.authority.nodeCounts) === JSON.stringify([1, 3, 5]) &&
    contract.authority.noQuorum === "fail-closed",
  "repair authority contract drifted",
);
check(
  contract.repair.durableMarkerBeforeAdapters === true &&
    contract.repair.unknownEffects === "manual-reconcile-required" &&
    contract.repair.postRepairVerificationRequired === true &&
    contract.repair.idempotent === true,
  "repair lifecycle contract drifted",
);
for (const symbol of [
  "planRepairVerification*",
  "planManualRepair*",
  "settleRepairPlan*",
  "settleRepairAdapters*",
  "recoverInterruptedRepair*",
  "rpoCommitRepairMarker",
]) {
  check(policy.includes(symbol), `missing Nim repair symbol: ${symbol}`);
}
for (const symbol of [
  "verify_replica",
  "repair_replica",
  "backup_to",
  "quarantine_to",
  "copy_from",
]) {
  check(adapter.includes(symbol), `missing repair adapter symbol: ${symbol}`);
}
for (const scenario of [
  "large_backlog",
  "capacity_failure",
  "batch_kill",
  "corrupt chunk",
  "repeated",
]) {
  check(scenarios.includes(scenario), `missing data-ops scenario: ${scenario}`);
}
check(contract.legacyRemovalOwner === 12, "wrong cutover owner");

console.log("Nimino data verify/repair contract verified");
