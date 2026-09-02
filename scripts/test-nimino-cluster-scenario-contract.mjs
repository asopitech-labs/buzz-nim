#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-cluster-scenarios/v1/contract.json", "utf8"),
);
const lifecycle = JSON.parse(
  readFileSync("contracts/nimino-cluster/v1/contract.json", "utf8"),
);
const control = JSON.parse(
  readFileSync("contracts/nimino-control-state/v1/contract.json", "utf8"),
);
const lease = JSON.parse(
  readFileSync("contracts/nimino-lease/v1/contract.json", "utf8"),
);
const scenario = readFileSync(
  "crates/nimino-chirps/tests/lifecycle_scenarios.rs",
  "utf8",
);
const runtime = readFileSync("crates/nimino-chirps/src/runtime.rs", "utf8");
const upstream = readFileSync("crates/nimino-chirps/src/upstream.rs", "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1 && contract.version === 1, "wrong scenario version");
check(contract.contract === "nimino.cluster-scenarios", "wrong scenario contract");
check(contract.compatibilityMode === false, "scenario compatibility mode is forbidden");
check(Number.isSafeInteger(contract.failureSeed), "failure seed must be reproducible");
check(
  JSON.stringify(contract.scenarios.map(({ nodeCount }) => nodeCount)) ===
    JSON.stringify([1, 3, 5]),
  "scenario topology matrix must be exactly 1/3/5 nodes",
);
check(
  lifecycle.scenarioContract === `${contract.contract}/v${contract.version}` &&
    contract.lifecycleContract === `${lifecycle.contract}/v1`,
  "cluster lifecycle and scenario contracts drifted",
);
check(
  control.scenarioHarnessOwner === 56 &&
    lease.scenarioHarnessOwner === 56 &&
    contract.dataConvergenceOwner === 59 &&
    contract.cutoverOwner === 12,
  "scenario ownership drifted",
);
for (const stage of [
  "partition",
  "drain",
  "kill",
  "rejoin",
  "stable-identity",
  "incremented-incarnation",
]) {
  check(contract.requiredStages.includes(stage), `missing scenario stage: ${stage}`);
}
for (const evidence of [
  "run_scenario",
  "MeshRuntime::start",
  "BoundaryRuntime::start",
  "victim_process",
  ".kill()",
  "wait_for_socket",
  "incarnation_before + 1",
  "vec![1, 3, 5]",
]) {
  check(scenario.includes(evidence), `missing real-mesh evidence: ${evidence}`);
}
check(
  runtime.includes("local_incarnation") && upstream.includes("mesh.incarnation()"),
  "Chirps incarnation recovery is not exposed as a transport fact",
);

console.log("Nimino 1/3/5-node real-mesh scenario contract verified");
