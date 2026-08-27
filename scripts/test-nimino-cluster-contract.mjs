#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-cluster/v1/contract.json", "utf8"),
);
const corpus = JSON.parse(
  readFileSync("contracts/nimino-cluster/v1/golden.json", "utf8"),
);
const policy = readFileSync(
  "nim/nimino_core/src/nimino_core/domain/cluster_lifecycle.nim",
  "utf8",
);
const rustBoundary = readFileSync(
  "crates/nimino-boundary/src/contract.rs",
  "utf8",
);
const worker = readFileSync(
  "nim/nimino_core/src/nimino_core_worker.nim",
  "utf8",
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong cluster lifecycle contract version");
check(contract.contract === "nimino.cluster-lifecycle", "wrong cluster contract");
check(contract.compatibilityMode === false, "cluster compatibility mode is forbidden");
check(contract.requiredProductCapability === "nimino-v1", "product capability drifted");
check(contract.controlModel === "nimino.control-log/v1", "control model link drifted");
check(
  contract.admission.chirpsPeerViewProvidesAuthority === false &&
    contract.admission.revalidatedThroughReady === true,
  "admission revalidation or Chirps authority contract drifted",
);
check(contract.runtimeStateMachineOwner === 51, "wrong runtime state-machine owner");
check(contract.leaseSemanticsOwner === 52, "wrong lease owner");
check(contract.scenarioHarnessOwner === 56, "wrong scenario owner");
check(contract.cutoverOwner === 12, "wrong cutover owner");
check(
  contract.boundaryOperation === "domain.cluster.lifecycle" &&
    rustBoundary.includes('"domain.cluster.lifecycle"') &&
    worker.includes('"domain.cluster.lifecycle"'),
  "typed cluster lifecycle operation is not wired on both boundary sides",
);
for (const symbol of [
  "decideLifecycleTransition*",
  "decideClusterLane*",
  "RequiredProductCapability*",
]) {
  check(policy.includes(symbol), `Nim cluster policy is missing ${symbol}`);
}

const names = new Set();
const commands = new Set();
const laneStates = new Set();
for (const testCase of corpus.cases) {
  check(!names.has(testCase.name), `duplicate golden case: ${testCase.name}`);
  names.add(testCase.name);
  check(testCase.invariant.length > 0, `${testCase.name}: missing invariant`);
  check(
    testCase.input.decision === testCase.expected.decision,
    `${testCase.name}: decision mismatch`,
  );
  if (testCase.input.decision === "transition") {
    commands.add(testCase.input.request.command);
  } else if (testCase.input.decision === "lane") {
    laneStates.add(testCase.input.request.state);
  } else {
    throw new Error(`${testCase.name}: unknown decision`);
  }
}
check(
  JSON.stringify([...commands].sort()) ===
    JSON.stringify(Object.keys(contract.commands).sort()),
  "golden corpus must exercise every lifecycle command",
);
check(
  JSON.stringify([...laneStates].sort()) ===
    JSON.stringify(Object.keys(contract.lanes).sort()),
  "golden corpus must exercise every lifecycle state lane gate",
);

console.log(
  `Nimino cluster lifecycle contract verified (${corpus.cases.length} golden cases)`,
);
