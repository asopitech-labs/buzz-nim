#!/usr/bin/env node

import { readFileSync } from "node:fs";

const read = (path) => readFileSync(path, "utf8");
const contract = JSON.parse(read("contracts/nimino-agent/v1/contract.json"));
const corpus = JSON.parse(read("contracts/nimino-agent/v1/golden.json"));
const nimPolicy = read("nim/nimino_core/src/nimino_core/domain/agent_policy.nim");
const nimWorker = read("nim/nimino_core/src/nimino_core_worker.nim");
const rustBoundary = read("crates/nimino-boundary/src/contract.rs");
const acpAdapter = read("crates/buzz-acp/src/acp.rs");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong agent contract version");
check(contract.contract === "nimino.agent-policy", "wrong agent contract name");
check(contract.compatibilityMode === false, "agent compatibility mode is forbidden");
check(contract.physicalRemovalOwner === 66 && contract.cutoverOwner === 12, "agent lifecycle owners drifted");
check(
  JSON.stringify(contract.decisions) === JSON.stringify(["persona", "trigger", "lifecycle"]),
  "agent decision inventory drifted",
);
check(
  contract.boundaryOperation === "domain.agent.policy" &&
    nimWorker.includes('"domain.agent.policy"') &&
    rustBoundary.includes('"domain.agent.policy"'),
  "typed agent policy operation is not wired on both boundary sides",
);
for (const symbol of ["resolvePersonaBehavior*", "decideAgentTrigger*", "decideAgentLifecycle*"]) {
  check(nimPolicy.includes(symbol), `Nim agent policy is missing ${symbol}`);
}

check(contract.acpAdapter.transport === "json-rpc-2.0-ndjson", "ACP transport drifted");
check(contract.acpAdapter.maxLineBytes === 10_000_000, "ACP line bound drifted");
for (const marker of [
  "const MAX_LINE_SIZE: usize = 10_000_000",
  "LinesCodec::new_with_max_length(MAX_LINE_SIZE)",
  "cancel_with_cleanup",
  "kill_process_group",
  "impl Drop for AcpClient",
]) {
  check(acpAdapter.includes(marker), `ACP adapter lost ${marker}`);
}

const names = new Set();
const decisions = new Set();
const lifecycleActions = new Set();
for (const testCase of corpus.cases) {
  check(!names.has(testCase.name), `duplicate golden case: ${testCase.name}`);
  names.add(testCase.name);
  check(testCase.invariant.length > 0, `${testCase.name}: missing invariant`);
  check(testCase.input.decision === testCase.expected.decision, `${testCase.name}: decision mismatch`);
  decisions.add(testCase.input.decision);
  if (testCase.input.decision === "lifecycle") lifecycleActions.add(testCase.expected.action);
}
check(
  JSON.stringify([...decisions].sort()) === JSON.stringify(["lifecycle", "persona", "trigger"]),
  "golden corpus must exercise every agent decision",
);
for (const action of ["spawn", "send_cancel", "reap_and_wait", "stop"]) {
  check(lifecycleActions.has(action), `golden corpus does not exercise ${action}`);
}
check(names.has("trigger-filter-error-fails-closed"), "trigger fail-closed proof is missing");

for (const entry of contract.rustPolicySitesToDelete) {
  const source = read(entry.path);
  for (const symbol of entry.symbols) {
    check(source.includes(symbol), `${entry.path}: missing ${symbol}`);
  }
}

console.log(`Nimino agent contract verified (${corpus.cases.length} golden cases)`);
