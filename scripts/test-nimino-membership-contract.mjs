#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-membership/v1/contract.json", "utf8"),
);
const corpus = JSON.parse(
  readFileSync("contracts/nimino-membership/v1/golden.json", "utf8"),
);
const nimPolicy = readFileSync(
  "nim/nimino_core/src/nimino_core/domain/membership_policy.nim",
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

check(contract.schemaVersion === 1, "wrong membership contract version");
check(
  contract.contract === "nimino.membership-policy",
  "wrong membership contract",
);
check(
  contract.compatibilityMode === false,
  "membership compatibility mode is forbidden",
);
check(contract.cutoverOwner === 12, "Issue #12 must own physical cutover");
check(
  contract.boundaryOperation === "domain.membership.policy" &&
    rustBoundary.includes('"domain.membership.policy"') &&
    nimWorker.includes('"domain.membership.policy"'),
  "typed membership policy operation is not wired on both boundary sides",
);
for (const symbol of [
  "proc roleCapabilities*",
  "proc decideChannelMembership*",
  "proc decideRelayMembership*",
  "proc decideInvite*",
  "proc decideOwnershipTransfer*",
]) {
  check(nimPolicy.includes(symbol), `Nim membership policy is missing ${symbol}`);
}

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
    JSON.stringify(["channel", "invite", "ownership_transfer", "relay"]),
  "golden corpus must exercise every membership policy decision",
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
for (const entry of contract.rustCompatibilityPathsToDelete) {
  check(
    readFileSync(entry.path, "utf8").includes(entry.marker),
    `${entry.path}: missing compatibility marker ${entry.marker}`,
  );
}

console.log(
  `Nimino membership contract verified (${corpus.cases.length} golden cases)`,
);
