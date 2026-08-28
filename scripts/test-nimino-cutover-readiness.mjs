#!/usr/bin/env node

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";

const manifest = JSON.parse(
  readFileSync("contracts/nimino-cutover-readiness/v1/manifest.json", "utf8"),
);
const justfile = readFileSync("Justfile", "utf8");

assert.equal(manifest.schemaVersion, 1);
assert.equal(manifest.contract, "nimino.cutover-readiness");
assert.equal(manifest.issue, 17);
assert.equal(manifest.phase, "frozen-for-cleanup");
assert.equal(manifest.compatibilityMode, false);
assert.deepEqual(
  manifest.workstreams.map(({ epic }) => epic),
  [2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
);
for (const stream of manifest.workstreams) {
  assert.ok(stream.name && stream.keep.length && stream.delete.length);
  assert.ok(stream.children.length && stream.owner.implementation.length);
  assert.equal(stream.owner.cleanup, 66);
  for (const path of stream.proof.contracts)
    assert.ok(existsSync(path), `${stream.name} proof missing: ${path}`);
  for (const command of stream.proof.commands)
    assert.ok(
      new RegExp(`^${command.replaceAll("-", "\\-")}:`, "m").test(justfile),
      `${stream.name} proof recipe missing: ${command}`,
    );
}
assert.deepEqual(manifest.sourceBlockers, []);
assert.deepEqual(
  manifest.nextGates.map(({ issue }) => issue),
  [66, 67, 68],
);
assert.equal(
  manifest.trackerAudit.requiredEpicStates,
  "CLOSED before issue 68",
);
assert.equal(
  manifest.trackerAudit.publicationIsNotPerformedByThisContract,
  true,
);

console.log(
  "Unified cutover readiness verified: Epics #2-#11, 0 source blockers",
);
