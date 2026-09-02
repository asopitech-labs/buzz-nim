#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-community/v1/contract.json", "utf8"),
);
const corpus = JSON.parse(
  readFileSync("contracts/nimino-community/v1/golden.json", "utf8"),
);
const nimPolicy = readFileSync(
  "nim/nimino_core/src/nimino_core/domain/community_policy.nim",
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

check(contract.schemaVersion === 1, "wrong community contract version");
check(
  contract.contract === "nimino.community-policy",
  "wrong community contract",
);
check(
  contract.compatibilityMode === false,
  "community compatibility mode is forbidden",
);
check(
  contract.cutoverOwner === 12,
  "Issue #12 must own physical community cutover",
);
check(
  contract.boundaryOperation === "domain.community.policy" &&
    rustBoundary.includes('"domain.community.policy"') &&
    nimWorker.includes('"domain.community.policy"'),
  "typed community policy operation is not wired on both boundary sides",
);
check(
  nimPolicy.includes("proc decideLifecycle*") &&
    nimPolicy.includes("proc decideScope*"),
  "Nim community policy does not own both decisions",
);

const decisions = new Set();
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
}
check(
  JSON.stringify([...decisions].sort()) ===
    JSON.stringify(["lifecycle", "scope"]),
  "golden corpus must exercise every community policy decision",
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
for (const entry of contract.productionPolicyCallers) {
  const source = readFileSync(entry.path, "utf8");
  for (const marker of entry.markers) {
    check(
      source.includes(marker),
      `${entry.path}: production community policy call is missing ${marker}`,
    );
  }
}

console.log(
  `Nimino community contract verified (${corpus.cases.length} golden cases)`,
);
