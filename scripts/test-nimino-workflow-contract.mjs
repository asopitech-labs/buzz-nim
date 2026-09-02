#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-workflow/v1/contract.json", "utf8"),
);
const corpus = JSON.parse(
  readFileSync("contracts/nimino-workflow/v1/golden.json", "utf8"),
);
const nimPolicy = readFileSync(
  "nim/nimino_core/src/nimino_core/domain/workflow_policy.nim",
  "utf8",
);
const rustBoundary = readFileSync(
  "crates/nimino-boundary/src/contract.rs",
  "utf8",
);
const nimWorker = readFileSync(
  "nim/nimino_core/src/nimino_core_worker.nim",
  "utf8",
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong workflow contract version");
check(contract.contract === "nimino.workflow-policy", "wrong workflow contract");
check(contract.compatibilityMode === false, "workflow compatibility mode is forbidden");
check(contract.cutoverOwner === 12, "Issue #12 must own physical workflow cutover");
check(contract.effectLedgerOwner === 57, "Issue #57 must own the effect ledger");
check(
  contract.effectLedgerContract === "nimino.effect-ledger/v1",
  "workflow effect ledger contract link drifted",
);
check(
  contract.boundaryOperation === "domain.workflow.policy" &&
    rustBoundary.includes('"domain.workflow.policy"') &&
    nimWorker.includes('"domain.workflow.policy"'),
  "typed workflow policy operation is not wired on both boundary sides",
);
for (const symbol of [
  "validateWorkflowDefinition*",
  "evaluateWorkflowCondition*",
  "planWorkflowStep*",
  "decideWorkflowTransition*",
]) {
  check(nimPolicy.includes(symbol), `Nim workflow policy is missing ${symbol}`);
}

const decisions = new Set();
const transitions = new Set();
const names = new Set();
for (const testCase of corpus.cases) {
  check(!names.has(testCase.name), `duplicate golden case: ${testCase.name}`);
  names.add(testCase.name);
  check(testCase.invariant.length > 0, `${testCase.name}: missing invariant`);
  check(
    testCase.input.decision === testCase.expected.decision,
    `${testCase.name}: input/result decision mismatch`,
  );
  decisions.add(testCase.input.decision);
  if (testCase.input.decision === "transition") {
    transitions.add(testCase.input.request.command);
  }
}
check(
  JSON.stringify([...decisions].sort()) ===
    JSON.stringify(["condition", "definition", "plan", "transition"]),
  "golden corpus must exercise every workflow decision",
);
check(
  JSON.stringify([...transitions].sort()) ===
    JSON.stringify([...contract.transitionCommands].sort()),
  "golden corpus must exercise every workflow transition command",
);
for (const action of contract.actionKinds) {
  check(nimPolicy.includes(`wa${action.split("_").map((part) => part[0].toUpperCase() + part.slice(1)).join("")}`), `Nim workflow policy is missing ${action}`);
}

for (const entry of contract.productionCutover) {
  const source = readFileSync(entry.path, "utf8");
  for (const symbol of entry.symbols) {
    check(source.includes(symbol), `${entry.path}: missing ${symbol}`);
  }
}

const productionSources = [
  "crates/nimino-relay/src/main.rs",
  "crates/nimino-relay/src/handlers/command_executor.rs",
  "crates/nimino-relay/src/api/bridge.rs",
  "crates/nimino-workflow/src/lib.rs",
  "crates/nimino-workflow/src/executor.rs",
  "crates/nimino-db/src/lib.rs",
  "crates/nimino-db/src/workflow.rs",
].map((path) => readFileSync(path, "utf8")).join("\n");
for (const symbol of contract.forbiddenProductionSymbols) {
  check(!productionSources.includes(symbol), `forbidden Rust workflow path remains: ${symbol}`);
}
check(
  !readFileSync("crates/nimino-workflow/src/lib.rs", "utf8").includes(
    "executor::evaluate_condition(",
  ),
  "Rust condition evaluator remains reachable from the workflow engine",
);
check(
  !readFileSync("crates/nimino-workflow/src/lib.rs", "utf8").includes(
    "executor::resolve_step_templates(",
  ),
  "Rust step planner remains reachable from the workflow engine",
);

console.log(
  `Nimino workflow contract verified (${corpus.cases.length} golden cases)`,
);
