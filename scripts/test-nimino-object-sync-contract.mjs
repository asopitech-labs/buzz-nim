#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-object-sync/v1/contract.json", "utf8"),
);
const sync = JSON.parse(
  readFileSync("contracts/nimino-sync/v1/contract.json", "utf8"),
);
const convergence = JSON.parse(
  readFileSync("contracts/nimino-convergence/v1/contract.json", "utf8"),
);
const data = JSON.parse(
  readFileSync("contracts/nimino-data/v1/contract.json", "utf8"),
);
const policy = readFileSync(contract.module, "utf8");
const adapter = readFileSync(`${contract.byteAdapter}/src/lib.rs`, "utf8");
const workspace = readFileSync("Cargo.toml", "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong object sync schema");
check(contract.version === 1, "wrong object sync version");
check(contract.contract === "nimino.object-sync", "wrong object sync id");
check(contract.compatibilityMode === false, "object compatibility mode is forbidden");
check(contract.owner === "nimino-core", "Nim must own object policy");
check(
  contract.syncContract === `${sync.contract}/v${sync.version}` &&
    contract.convergenceContract ===
      `${convergence.contract}/v${convergence.version}`,
  "object sync dependency contracts drifted",
);
check(
  workspace.includes('"crates/nimino-object-store"') &&
    contract.partialInstall.maxChunkBytes === 1048576 &&
    contract.manifest.maxObjectBytes === 68719476736 &&
    adapter.includes("pub const MAX_CHUNK_BYTES: usize = 1_048_576") &&
    adapter.includes("pub const MAX_OBJECT_BYTES: u64 = 68_719_476_736"),
  "object adapter bounds drifted",
);
for (const evidence of [
  "Sha256",
  "sync_data()",
  "sync_all()",
  "fs::hard_link",
  "OffsetMismatch",
  "DigestMismatch",
]) {
  check(adapter.includes(evidence), `missing atomic object evidence: ${evidence}`);
}
for (const symbol of [
  "planObjectSync*",
  "initPinState*",
  "decidePin*",
  "planObjectGc*",
  "ofmEager",
  "ofmLazy",
  "opeMissingOrigin",
]) {
  check(policy.includes(symbol), `missing Nim object policy symbol: ${symbol}`);
}
for (const recordType of contract.canonicalRecordTypes) {
  check(
    data.classes.canonical.recordTypes.includes(recordType),
    `missing canonical object record type: ${recordType}`,
  );
}
check(
  contract.gc.requiresCrossCommunityReferenceSnapshot === true &&
    contract.partialInstall.overwrite === false,
  "GC isolation or no-overwrite contract drifted",
);
check(contract.repairOwner === 59, "wrong repair owner");
check(contract.legacyRemovalOwner === 12, "wrong cutover owner");

console.log("Nimino object sync contract verified");
