#!/usr/bin/env node

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-cutover-rehearsal/v1/contract.json", "utf8"),
);
const promotion = JSON.parse(
  readFileSync("contracts/nimino-promotion/v1/contract.json", "utf8"),
);
const cli = readFileSync("crates/nimino-data-ops/src/main.rs", "utf8");
const adapter = readFileSync("crates/nimino-data-ops/src/lib.rs", "utf8");
const scenarios = readFileSync(
  "crates/nimino-data-ops/tests/convergence_scenarios.rs",
  "utf8",
);
const runbook = readFileSync(
  "contracts/nimino-cutover-rehearsal/README.md",
  "utf8",
);

assert.equal(contract.schemaVersion, 1);
assert.equal(contract.contract, "nimino.cutover-rehearsal");
assert.equal(contract.issue, 64);
assert.equal(contract.compatibilityMode, false);
assert.equal(contract.backup.format, "nimino.cutover-backup/v1");
assert.equal(contract.backup.overwritePolicy, "reject");
assert.equal(contract.restore.targetPolicy, "new-paths-only");
assert.equal(contract.restore.success, "exact-before-after-match");
assert.equal(contract.failedPromotionContract, `${promotion.contract}/v1`);
assert.equal(promotion.partialFailurePolicy, "restore-last-known-good");
assert.equal(contract.physicalCutoverOwner, 68);
for (const forbidden of [
  "buzz-compatibility-shim",
  "dual-write",
  "down-migration",
  "in-place-restore",
])
  assert.ok(contract.rollback.forbidden.includes(forbidden));
for (const signal of [
  "Command::Backup",
  "Command::Restore",
  "backup_replica",
  "restore_replica",
])
  assert.ok(`${cli}\n${adapter}`.includes(signal), `missing ${signal}`);
for (const signal of [
  "backup_restore_rehearsal_preserves_inventory_and_rejects_corruption",
  "NIMINO_CUTOVER_EVIDENCE",
  "corruptBackupRejected",
  "failedPromotionVerified",
])
  assert.ok(scenarios.includes(signal), `missing rehearsal signal: ${signal}`);
for (const signal of [
  "Freeze all store and object writes",
  "nimino-data-ops backup",
  "nimino-data-ops restore",
  "just nimino-cutover-rehearsal",
  "Never restore in place",
])
  assert.ok(runbook.includes(signal), `missing runbook signal: ${signal}`);

console.log("Nimino cutover recovery rehearsal contract verified");
