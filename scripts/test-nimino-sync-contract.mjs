#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-sync/v1/contract.json", "utf8"),
);
const data = JSON.parse(
  readFileSync("contracts/nimino-data/v1/contract.json", "utf8"),
);
const policy = readFileSync(contract.module, "utf8");
const imports = policy
  .split("\n")
  .filter((line) => /^(?:import|from)\s/.test(line))
  .join("\n");
const store = readFileSync("crates/nimino-store/src/types.rs", "utf8");
const digestAdapter = readFileSync(
  "crates/nimino-store/src/sync_digest.rs",
  "utf8",
);
const chirps = readFileSync("crates/nimino-chirps/src/runtime.rs", "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong sync contract schema");
check(contract.version === 1, "wrong sync protocol version");
check(contract.contract === "nimino.sync", "wrong sync contract id");
check(contract.compatibilityMode === false, "sync compatibility mode is forbidden");
check(contract.owner === "nimino-core", "Nim must own sync decisions");
check(
  contract.dataContract === `${data.protocol}/v${data.version}` &&
    data.antiEntropyContract === `${contract.contract}/v${contract.version}`,
  "data and sync contracts are not linked",
);
check(
  contract.transport.adapter === "nimino-chirps" &&
    contract.transport.providesReplicationPolicy === false &&
    contract.transport.providesConflictPolicy === false,
  "Chirps acquired sync policy",
);
check(
  contract.bounds.maxRecords === 1000 &&
    contract.bounds.maxEncodedBytes === 1048576 &&
    contract.bounds.inflightBatches === 1 &&
    policy.includes("MaxSyncRecords* = 1_000'u16") &&
    policy.includes("MaxSyncEncodedBytes* = 1_048_576'u32") &&
    store.includes("pub const MAX_PAGE_SIZE: usize = 1_000") &&
    chirps.includes("const MAX_MESSAGE_BYTES: usize = 1024 * 1024"),
  "store, transport, and sync bounds drifted",
);
for (const method of ["canonical_checkpoint", "changes", "commit_canonical"]) {
  check(store.includes(`fn ${method}(`), `missing store capability: ${method}`);
}
for (const helper of [
  "canonical_prefix_digest",
  "canonical_record_digest",
  "extend_prefix_digest",
  "verify_range_digest",
]) {
  check(digestAdapter.includes(`fn ${helper}(`), `missing digest helper: ${helper}`);
}
check(
  contract.digest.adapter === "nimino-store.sync_digest" &&
    contract.digest.adapterVerifiesCanonicalRange === true &&
    digestAdapter.includes("Sha256"),
  "SHA-256 adapter contract drifted",
);
for (const symbol of [
  "acceptRemoteDigest*",
  "nextRangeRequest*",
  "planRangeRead*",
  "planRangeBatch*",
  "settleRangeBatch*",
  "stopSync*",
  "cancelSync*",
  "checkSyncDeadline*",
  "seBackpressure",
  "seScopeMismatch",
]) {
  check(policy.includes(symbol), `missing Nim sync symbol: ${symbol}`);
}
check(
  contract.communityIsolationEveryFrame === true &&
    contract.scopeFields.length === 6,
  "frame isolation contract drifted",
);
check(
  contract.resume.advanceBeforeCommit === false &&
    contract.resume.duplicateFinalBatch === "idempotent-ack",
  "durable resume contract drifted",
);
check(
  !/\b(?:alopex|chirps|redb|redis|postgres|sqlx)\b/i.test(imports),
  "pure sync policy imports a transport or store adapter",
);
check(contract.conflictResolutionOwner === 53, "wrong conflict owner");
check(contract.scenarioHarnessOwner === 56, "wrong scenario owner");
check(contract.legacyRemovalOwner === 12, "wrong cutover owner");

console.log("Nimino bounded anti-entropy contract verified");
