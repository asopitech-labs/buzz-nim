#!/usr/bin/env node

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contract = JSON.parse(
  readFileSync(join(root, "contracts/nimino-control/v1/model.json"), "utf8"),
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function read(path) {
  return readFileSync(join(root, path), "utf8");
}

function sha256(path) {
  return createHash("sha256").update(read(path)).digest("hex");
}

check(contract.schemaVersion === 1, "control model schema version drifted");
check(contract.contract === "nimino.control-log", "control contract id drifted");
check(contract.version === 1, "control contract version drifted");
check(contract.compatibilityMode === false, "compatibility mode is forbidden");
check(contract.owner === "nimino-control-plane", "control authority owner drifted");
check(
  contract.transport.adapter === "nimino-chirps" &&
    contract.transport.providesAuthority === false,
  "Chirps must remain a non-authoritative transport adapter",
);
check(
  contract.quorum.formula === "floor(voter-count/2)+1" &&
    contract.quorum.joint === "majority-of-old-and-majority-of-new" &&
    contract.quorum.minorityCanElect === false &&
    contract.quorum.minorityCanCommit === false &&
    contract.quorum.certificatesMustIntersect === true,
  "quorum contract drifted",
);
check(
  contract.epochs.clock === "monotonic-election-timeout-only" &&
    contract.epochs.clockCanAuthorizeCommit === false,
  "clock authority contract drifted",
);
check(
  JSON.stringify(contract.voterTransition) ===
    JSON.stringify([
      "stable-old",
      "committed-begin-joint",
      "joint",
      "committed-finalize",
      "stable-new",
    ]),
  "voter transition contract drifted",
);
check(contract.controlLog.runtimeStateMachineOwner === 51, "wrong runtime owner");
check(contract.controlLog.durableStoreOwner === 49, "wrong durable-store owner");
check(contract.controlLog.antiEntropyOwner === 50, "wrong anti-entropy owner");
check(
  contract.durableStore.adapter === "crates/nimino-store" &&
    contract.durableStore.eventAntiEntropyTables === false &&
    contract.durableStore.metadata.includes("revision-CAS") &&
    contract.durableStore.log.includes("uncommitted suffix"),
  "control-log durability must remain isolated and atomic",
);
check(contract.snapshot.mayContainUncommittedState === false, "unsafe snapshot");
check(contract.cutoverOwner === 12, "wrong cutover owner");

const formal = contract.formal;
check(sha256(formal.model) === formal.modelSha256, "TLA+ model hash drifted");
check(sha256(formal.scenario) === formal.scenarioSha256, "TLC scenario hash drifted");

const model = read(formal.model);
const scenario = read(formal.scenario);
const store = read("crates/nimino-store/src/control_log.rs");
for (const table of contract.durableStore.tables) {
  check(store.includes(`"${table}"`), `missing control store table: ${table}`);
}
check(
  store.includes("pub trait ControlLogStorePort") &&
    store.includes("set_quick_repair(true)") &&
    !/\b(?:CANONICAL|CHANGES|Chirps)\b/.test(store),
  "control store must remain a durable adapter isolated from data and transport",
);
for (const invariant of formal.invariants) {
  check(model.includes(`${invariant} ==`), `missing model invariant: ${invariant}`);
  check(
    scenario.includes(`INVARIANT ${invariant}`),
    `scenario does not check invariant: ${invariant}`,
  );
}

const evidence = read("formal/evidence/FM-cluster-control-log_20260828_summary.md");
check(evidence.includes(formal.modelSha256), "evidence model hash drifted");
check(evidence.includes(formal.scenarioSha256), "evidence scenario hash drifted");
check(
  evidence.includes("Model checking completed. No error has been found."),
  "successful TLC evidence is missing",
);

const inventory = read("formal/inventory/FM-cluster-control-log.md");
check(inventory.includes("Status: active"), "formal model is not active");
check(inventory.includes("just control-model-check"), "verification command drifted");

const adr = read("docs/adr/nimino-control-log-v1.md");
for (const owner of ["#48", "#49", "#51", "#52", "#12"]) {
  check(adr.includes(owner), `ADR is missing owner ${owner}`);
}

console.log(
  `Nimino control model v${contract.version} passed: ${formal.invariants.length} invariants, hashes and evidence fixed`,
);
