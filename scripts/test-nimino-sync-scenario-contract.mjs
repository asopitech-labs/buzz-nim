#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-sync-scenarios/v1/contract.json", "utf8"),
);
const runtime = readFileSync("crates/nimino-sync/src/lib.rs", "utf8");
const test = readFileSync("crates/nimino-sync/tests/three_node.rs", "utf8");
const manifest = readFileSync("crates/nimino-sync/Cargo.toml", "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong sync scenario schema version");
check(contract.contract === "nimino.sync-scenarios", "wrong sync scenario contract");
check(contract.compatibilityMode === false, "sync compatibility mode is forbidden");
check(contract.nodeCount === 3, "sync evidence must use exactly three nodes");
check(
  contract.policyOwner === 50 &&
    contract.convergencePolicyOwner === 53 &&
    contract.releaseOwner === 61,
  "issue ownership drifted",
);
for (const stage of [
  "bounded-bootstrap",
  "durable-checkpoint-resume",
  "slow-peer-backpressure-recovery",
  "duplicate-idempotency",
  "divergent-history-convergence",
  "identity-collision-quarantine",
  "community-isolation",
  "clean-shutdown",
]) {
  check(contract.requiredStages.includes(stage), `missing sync stage: ${stage}`);
}
for (const signal of [
  "BoundaryRequest::sync_policy",
  "canonical_state_digest",
  "commit_canonical",
  "MeshClient",
  "SyncRuntime::start",
]) {
  check(runtime.includes(signal), `missing production sync signal: ${signal}`);
}
for (const signal of [
  "wait_for_digest",
  "expected_checkpoint: 4",
  "community-private",
  "MeshRuntimeError::SubscriberLagged",
  "slowPeerRecoveredCheckpoint",
  "divergentHistoriesConverged",
  "identityCollisionQuarantined",
  "releasedUdpSockets",
]) {
  check(test.includes(signal), `missing real three-node evidence: ${signal}`);
}
check(!manifest.includes("alopex-chirps"), "sync runtime must use the thin Chirps adapter");

console.log("Nimino real-Chirps sync scenario contract verified");
