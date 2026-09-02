#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-dm/v1/contract.json", "utf8"),
);
const corpus = JSON.parse(
  readFileSync("contracts/nimino-dm/v1/golden.json", "utf8"),
);
const nimPolicy = readFileSync(
  "nim/nimino_core/src/nimino_core/domain/dm_policy.nim",
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
const ingest = readFileSync(
  "crates/nimino-relay/src/handlers/ingest.rs",
  "utf8",
);
const commands = readFileSync(
  "crates/nimino-relay/src/handlers/command_executor.rs",
  "utf8",
);
const req = readFileSync(
  "crates/nimino-relay/src/handlers/req.rs",
  "utf8",
);
const fanout = readFileSync(
  "crates/nimino-relay/src/handlers/event.rs",
  "utf8",
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong DM contract version");
check(contract.contract === "nimino.dm-policy", "wrong DM contract");
check(contract.compatibilityMode === false, "DM compatibility mode is forbidden");
check(contract.cutoverOwner === 12, "Issue #12 must own physical DM cutover");
check(
  contract.boundaryOperation === "domain.dm.policy" &&
    rustBoundary.includes('"domain.dm.policy"') &&
    nimWorker.includes('"domain.dm.policy"'),
  "typed DM policy operation is not wired on both boundary sides",
);
check(
  ingest.includes("BoundaryRequest::dm_policy") &&
    commands.includes("DmPolicyRequest::Mutation") &&
    ingest.includes("DmAccessOperation::Write") &&
    req.includes("DmAccessOperation::Read") &&
    req.includes("DmAccessOperation::Visibility") &&
    fanout.includes("DmAccessOperation::Read"),
  "production DM mutation/read/write/visibility paths do not all call Nim",
);
check(
  nimPolicy.includes("proc decideDmMutation*") &&
    nimPolicy.includes("proc decideDmAccess*"),
  "Nim DM policy does not own both decisions",
);

const decisions = new Set();
const mutationCommands = new Set();
const accessOperations = new Set();
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
  if (testCase.input.decision === "mutation") {
    mutationCommands.add(testCase.input.request.command);
  } else {
    accessOperations.add(testCase.input.request.operation);
  }
}
check(
  JSON.stringify([...decisions].sort()) === JSON.stringify(["access", "mutation"]),
  "golden corpus must exercise every DM policy decision",
);
check(
  JSON.stringify([...mutationCommands].sort()) ===
    JSON.stringify(["add", "hide", "open"]),
  "golden corpus must exercise every DM lifecycle mutation",
);
check(
  JSON.stringify([...accessOperations].sort()) ===
    JSON.stringify(["read", "visibility", "write"]),
  "golden corpus must exercise every DM access surface",
);

for (const entry of contract.rustPolicySitesToShrink) {
  const source = readFileSync(entry.path, "utf8");
  for (const symbol of entry.symbols) {
    check(
      new RegExp(`\\b${symbol}\\b`).test(source),
      `${entry.path}: missing ${symbol}`,
    );
  }
}

console.log(`Nimino DM contract verified (${corpus.cases.length} golden cases)`);
