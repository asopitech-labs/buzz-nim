#!/usr/bin/env node

import { readFileSync } from "node:fs";

const eventContract = JSON.parse(
  readFileSync("contracts/nimino-event/v1/contract.json", "utf8"),
);
const corpus = JSON.parse(
  readFileSync("contracts/nimino-event/v1/golden.json", "utf8"),
);
const protocol = JSON.parse(
  readFileSync("contracts/nimino-protocol-v1.json", "utf8"),
);
const nimPolicy = readFileSync(
  "nim/nimino_core/src/nimino_core/domain/event_policy.nim",
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

function sorted(values) {
  return [...values].sort((left, right) => left - right);
}

check(eventContract.contract === "nimino.event-policy", "wrong event contract");
check(eventContract.schemaVersion === 1, "wrong event contract version");
check(
  eventContract.compatibilityMode === false,
  "event compatibility mode is forbidden",
);
check(
  eventContract.cutoverOwner === 12,
  "Issue #12 must own physical event cutover",
);
check(
  eventContract.boundaryOperation === "domain.event.policy" &&
    rustBoundary.includes('"domain.event.policy"') &&
    nimWorker.includes('"domain.event.policy"'),
  "typed event policy operation is not wired on both boundary sides",
);

const registry = protocol.surfaces.find(
  (surface) => surface.id === "kind.current-registry",
)?.values;
check(Array.isArray(registry), "missing Nimino kind registry");
const nimKinds = [
  ...(
    nimPolicy.match(/SupportedKinds\* = \[([\s\S]*?)\n {2}\]/)?.[1] ?? ""
  ).matchAll(/\b\d+\b/g),
].map((match) => Number(match[0]));
check(
  JSON.stringify(sorted(nimKinds)) === JSON.stringify(sorted(registry)),
  "Nim event policy kinds differ from the v1 protocol registry",
);
check(new Set(nimKinds).size === nimKinds.length, "duplicate Nim event kind");

const decisions = new Set();
const names = new Set();
for (const testCase of corpus.cases) {
  check(!names.has(testCase.name), `duplicate golden case: ${testCase.name}`);
  names.add(testCase.name);
  check(
    testCase.standard.length > 0,
    `${testCase.name}: missing NIP reference`,
  );
  check(
    testCase.input.decision === testCase.expected.decision,
    `${testCase.name}: input/result decision mismatch`,
  );
  decisions.add(testCase.input.decision);
}
check(
  JSON.stringify([...decisions].sort()) ===
    JSON.stringify([
      "classify",
      "deletion",
      "reaction",
      "replacement",
      "thread",
    ]),
  "golden corpus must exercise every event policy decision",
);

for (const entry of eventContract.rustDuplicatesToDelete) {
  const source = readFileSync(entry.path, "utf8");
  for (const qualified of entry.symbols) {
    const symbol = qualified.split("::").at(-1);
    check(
      new RegExp(
        `\\b${symbol.replaceAll(/[$()*+.?[\\\]^{|}]/g, "\\$&")}\\b`,
      ).test(source),
      `${entry.path}: missing cutover symbol ${qualified}`,
    );
  }
}

console.log(
  `Nimino event contract verified (${nimKinds.length} kinds, ${corpus.cases.length} golden cases)`,
);
