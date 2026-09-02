#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-control-state/v1/contract.json", "utf8"),
);
const stateMachine = readFileSync(contract.module, "utf8");
const adapter = readFileSync(contract.runtimeAdapter, "utf8");
const realMeshScenario = readFileSync(contract.realMeshScenario, "utf8");
const stateMachineImports = stateMachine
  .split("\n")
  .filter((line) => /^(?:import|from)\s/.test(line))
  .join("\n");
const store = readFileSync("crates/nimino-store/src/control_log.rs", "utf8");
const model = JSON.parse(
  readFileSync("contracts/nimino-control/v1/model.json", "utf8"),
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong control-state contract version");
check(contract.version === 1, "wrong control-state protocol version");
check(
  contract.contract === "nimino.control-state-machine",
  "wrong control-state contract id",
);
check(contract.compatibilityMode === false, "compatibility mode is forbidden");
check(contract.owner === "nimino-core", "Nim must own control decisions");
for (const evidence of [
  "ControlPolicyRequest::Vote",
  "ControlPolicyRequest::Election",
  "ControlPolicyRequest::Replicate",
  "ControlPolicyRequest::Commit",
  "ControlPolicyRequest::Apply",
  "ControlPolicyRequest::Recover",
  "ControlLogStorePort",
  "WIRE_PREFIX",
]) {
  check(adapter.includes(evidence), `missing production control evidence: ${evidence}`);
}
for (const evidence of [
  "three_nodes_elect_commit_fail_closed_and_catch_up_over_real_chirps",
  "ControlRuntimeError::QuorumRequired",
  "wait_for_commit(&stores, 2)",
]) {
  check(realMeshScenario.includes(evidence), `missing real control scenario: ${evidence}`);
}
check(
  contract.formalModel === `${model.contract}/v${model.version}`,
  "formal model link drifted",
);
check(
  model.controlLog.runtimeStateMachineContract ===
    `${contract.contract}/v${contract.version}`,
  "control model does not point back to the runtime contract",
);
check(
  contract.authority.minorityCanElect === false &&
    contract.authority.minorityCanCommit === false &&
    contract.authority.transportProvidesAuthority === false,
  "minority or transport authority is forbidden",
);
check(
  contract.durability.port === "ControlLogStorePort" &&
    contract.durability.planBeforeMutation === true &&
    contract.durability.settleAfterPersistence === true &&
    contract.durability.storeFailure === "retain-pre-transition-state" &&
    store.includes("pub trait ControlLogStorePort"),
  "persistence-first fail-closed contract drifted",
);
for (const symbol of [
  "planVote*",
  "planElection*",
  "planAppend*",
  "planReplication*",
  "planCommit*",
  "checkControlQuorum*",
  "planApply*",
  "planSnapshot*",
  "recoverControlState*",
  "settleControlPlan*",
]) {
  check(stateMachine.includes(symbol), `missing Nim control symbol: ${symbol}`);
}
check(
  !/\b(?:alopex|chirps|redb|redis|postgres|sqlx)\b/i.test(
    stateMachineImports,
  ),
  "pure control state machine imports an adapter or transport",
);
check(contract.leaseOwner === 52, "wrong lease owner");
check(contract.scenarioHarnessOwner === 56, "wrong scenario owner");
check(contract.cutoverOwner === 12, "wrong cutover owner");

console.log("Nimino control state-machine contract verified");
