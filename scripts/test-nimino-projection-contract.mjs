#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-projection/v1/contract.json", "utf8"),
);
const data = JSON.parse(
  readFileSync("contracts/nimino-data/v1/contract.json", "utf8"),
);
const convergence = JSON.parse(
  readFileSync("contracts/nimino-convergence/v1/contract.json", "utf8"),
);
const policy = readFileSync(contract.module, "utf8");
const stage = readFileSync("crates/nimino-store/src/projection_stage.rs", "utf8");
const store = readFileSync("crates/nimino-store/src/types.rs", "utf8");
const dataOps = readFileSync("crates/nimino-data-ops/src/lib.rs", "utf8");
const command = readFileSync("crates/nimino-data-ops/src/main.rs", "utf8");
const scenario = readFileSync(
  "crates/nimino-data-ops/tests/projection_rebuild.rs",
  "utf8",
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong projection contract schema");
check(contract.version === 1, "wrong projection contract version");
check(contract.contract === "nimino.projection-lifecycle", "wrong projection id");
check(contract.compatibilityMode === false, "projection compatibility mode is forbidden");
check(contract.owner === "nimino-core", "Nim must own projection decisions");
check(
  contract.dataContract === `${data.protocol}/v${data.version}` &&
    data.projectionLifecycleContract ===
      `${contract.contract}/v${contract.version}` &&
    contract.convergenceContract ===
      `${convergence.contract}/v${convergence.version}`,
  "projection dependency contracts drifted",
);
check(
  contract.identity.length === 7 &&
    contract.source.fixedCheckpointRequired === true &&
    contract.source.batchLimit === 1000,
  "projection identity or bounds drifted",
);
for (const symbol of [
  "startProjection*",
  "planProjectionBatch*",
  "settleProjectionBatch*",
  "planProjectionPublish*",
  "settleProjectionPublish*",
  "MaxProjectionBatch* = 1_000",
]) {
  check(policy.includes(symbol), `missing Nim projection symbol: ${symbol}`);
}
for (const evidence of [
  "PROJECTION_META",
  "PROJECTION_ROWS",
  "begin_projection_stage",
  "stage_projection_batch",
  "recover_projection_stage",
  "discard_projection_stage",
  "ProjectionStageRevisionConflict",
  "ProjectionStageCursorConflict",
]) {
  check(stage.includes(evidence), `missing projection stage evidence: ${evidence}`);
}
check(
  store.includes("fn replace_cache(") &&
    contract.publish.adapter === "NodeStorePort.replace_cache" &&
    contract.publish.atomic === true &&
    contract.publish.idempotent === true,
  "atomic projection publication drifted",
);
check(contract.staging.productTruth === false, "staging became product truth");
check(
  contract.runtime.boundaryOperation === "domain.projection.policy" &&
    contract.runtime.adapter === "nimino-data-ops.rebuild_projections" &&
    dataOps.includes("pub async fn rebuild_projections(") &&
    dataOps.includes("ProjectionPolicyRequest::Batch") &&
    dataOps.includes("store.replace_cache(CacheReplacement") &&
    command.includes("ProjectionRebuild") &&
    scenario.includes("partial_stage_then_drop_and_rebuild_is_query_equivalent"),
  "production projection rebuild path or scenario drifted",
);
check(contract.repairOwner === 59, "wrong repair owner");
check(contract.legacyRemovalOwner === 12, "wrong cutover owner");

console.log("Nimino projection rebuild contract verified");
