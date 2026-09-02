#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-effect-ledger/v1/contract.json", "utf8"),
);
const workflow = JSON.parse(
  readFileSync("contracts/nimino-workflow/v1/contract.json", "utf8"),
);
const lease = JSON.parse(
  readFileSync("contracts/nimino-lease/v1/contract.json", "utf8"),
);
const data = JSON.parse(
  readFileSync("contracts/nimino-data/v1/contract.json", "utf8"),
);
const policy = readFileSync(contract.module, "utf8");
const store = readFileSync("crates/nimino-store/src/types.rs", "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1 && contract.version === 1, "wrong ledger version");
check(contract.contract === "nimino.effect-ledger", "wrong ledger contract");
check(contract.compatibilityMode === false, "ledger compatibility mode is forbidden");
check(contract.owner === "nimino-core", "Nim must own effect lifecycle policy");
check(
  workflow.effectLedgerContract === `${contract.contract}/v${contract.version}` &&
    contract.workflowContract === `${workflow.contract}/v1` &&
    contract.leaseContract === `${lease.contract}/v${lease.version}` &&
    contract.dataContract === `${data.protocol}/v${data.version}` &&
    data.effectLedgerContract === `${contract.contract}/v${contract.version}`,
  "effect ledger dependency contracts drifted",
);
check(
  contract.recordType === "workflow_effect" &&
    data.classes.canonical.recordTypes.includes(contract.recordType) &&
    data.classes.log.recordTypes.includes(contract.persistence.operationalEvidence),
  "effect ledger must remain replicated canonical truth",
);
check(
  contract.identity.length === 6 &&
    JSON.stringify(contract.states) ===
      JSON.stringify(["pending", "claimed", "executing", "succeeded", "failed", "unknown"]),
  "effect ledger identity or state lifecycle drifted",
);
check(
  contract.execution.liveQuorumLeaseRequired === true &&
    contract.execution.executionMarkerPersistedFirst === true &&
    contract.execution.idempotencyKeyPersisted === true &&
    contract.execution.receiptPersisted === true,
  "effect execution safety contract drifted",
);
check(
  contract.recovery.executingWithoutReceipt === "unknown" &&
    contract.recovery.unknownAutomaticRetry === false &&
    contract.recovery.manualReconcileRequired === true &&
    contract.recovery.manualRetryKeepsIdempotencyKey === true,
  "unknown/manual reconciliation contract drifted",
);
for (const symbol of [
  "planEffectClaim*",
  "planEffectExecution*",
  "planEffectReceipt*",
  "planEffectRecovery*",
  "planEffectReconcile*",
  "settleEffectLedger*",
  "authorizeSingletonEffect",
  "efeExecuteExternal",
  "eleManualReconcileRequired",
]) {
  check(policy.includes(symbol), `missing effect ledger symbol: ${symbol}`);
}
check(
  contract.persistence.adapter === "NodeStorePort.commit_canonical" &&
    store.includes("fn commit_canonical("),
  "canonical effect persistence adapter drifted",
);
check(contract.workflowDecisionOwner === 29, "wrong workflow decision owner");
check(contract.repairOwner === 59, "wrong repair owner");
check(contract.legacyRemovalOwner === 12, "wrong cutover owner");

console.log("Nimino workflow effect ledger contract verified");
