#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-moderation/v1/contract.json", "utf8"),
);
const corpus = JSON.parse(
  readFileSync("contracts/nimino-moderation/v1/golden.json", "utf8"),
);
const nimPolicy = readFileSync(
  "nim/nimino_core/src/nimino_core/domain/moderation_policy.nim",
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
const auth = readFileSync(
  "crates/nimino-relay/src/handlers/auth.rs",
  "utf8",
);
const commands = readFileSync(
  "crates/nimino-relay/src/handlers/moderation_commands.rs",
  "utf8",
);
const report = readFileSync(
  "crates/nimino-relay/src/handlers/report.rs",
  "utf8",
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong moderation contract version");
check(
  contract.contract === "nimino.moderation-policy",
  "wrong moderation contract",
);
check(
  ingest.includes("BoundaryRequest::moderation_policy") &&
    ingest.includes("ModerationEnforcementOperation::Write") &&
    auth.includes("ModerationEnforcementOperation::Authenticate") &&
    commands.includes("ModerationPolicyRequest::Restriction") &&
    commands.includes("ModerationPolicyRequest::Resolution") &&
    report.includes("ModerationPolicyRequest::Report"),
  "production moderation report/restriction/resolution/enforcement paths do not all call Nim",
);
check(
  contract.compatibilityMode === false,
  "moderation compatibility mode is forbidden",
);
check(contract.cutoverOwner === 12, "Issue #12 must own physical cutover");
check(
  contract.boundaryOperation === "domain.moderation.policy" &&
    rustBoundary.includes('"domain.moderation.policy"') &&
    nimWorker.includes('"domain.moderation.policy"'),
  "typed moderation operation is not wired on both boundary sides",
);
for (const symbol of [
  "proc decideModerationReport*",
  "proc decideModerationRestriction*",
  "proc decideModerationResolution*",
  "proc decideModerationEnforcement*",
]) {
  check(nimPolicy.includes(symbol), `Nim moderation policy is missing ${symbol}`);
}

const decisions = new Set();
const restrictionCommands = new Set();
const resolutionActions = new Set();
const enforcementOperations = new Set();
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
  if (testCase.input.decision === "restriction") {
    restrictionCommands.add(testCase.input.request.command);
  }
  if (testCase.input.decision === "resolution") {
    resolutionActions.add(testCase.input.request.action);
  }
  if (testCase.input.decision === "enforcement") {
    enforcementOperations.add(testCase.input.request.operation);
  }
}
check(
  JSON.stringify([...decisions].sort()) ===
    JSON.stringify(["enforcement", "report", "resolution", "restriction"]),
  "golden corpus must exercise every moderation decision",
);
check(
  JSON.stringify([...restrictionCommands].sort()) ===
    JSON.stringify(["ban", "timeout", "unban", "untimeout"]),
  "golden corpus must exercise every restriction transition",
);
check(
  JSON.stringify([...resolutionActions].sort()) ===
    JSON.stringify(["ban", "delete", "dismiss", "escalate", "kick", "timeout"]),
  "golden corpus must exercise every resolution action",
);
check(
  JSON.stringify([...enforcementOperations].sort()) ===
    JSON.stringify(["authenticate", "write"]),
  "golden corpus must exercise every enforcement surface",
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
for (const symbol of contract.deletedRustSymbols) {
  check(
    !ingest.includes(symbol) &&
      !auth.includes(symbol) &&
      !commands.includes(symbol) &&
      !report.includes(symbol),
    `deleted Rust moderation policy returned: ${symbol}`,
  );
}

console.log(
  `Nimino moderation contract verified (${corpus.cases.length} golden cases)`,
);
