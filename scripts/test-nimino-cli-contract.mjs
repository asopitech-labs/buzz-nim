#!/usr/bin/env node

import { readFileSync } from "node:fs";

const read = (path) => readFileSync(path, "utf8");
const contract = JSON.parse(read("contracts/nimino-cli/v1/contract.json"));
const commands = JSON.parse(read("contracts/nimino-cli/v1/commands.json"));
const golden = JSON.parse(read("contracts/nimino-cli/v1/golden.json"));
const naming = JSON.parse(read("contracts/nimino-naming-v1.json"));
const nimPolicy = read("nim/nimino_core/src/nimino_core/domain/cli_policy.nim");
const nimWorker = read("nim/nimino_core/src/nimino_core_worker.nim");
const rustBoundary = read("crates/nimino-boundary/src/contract.rs");
const cargo = read("crates/nimino-cli/Cargo.toml");
const cliSource = read("crates/nimino-cli/src/lib.rs");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1 && contract.version === 1, "wrong CLI contract version");
check(contract.contract === "nimino.cli", "wrong CLI contract name");
check(contract.compatibilityMode === false, "CLI compatibility mode is forbidden");
check(contract.binary === "nimino", "canonical CLI binary must be nimino");
check(
  JSON.stringify(contract.forbiddenBinaries) === JSON.stringify(["buzz"]),
  "legacy CLI binary must be forbidden",
);
check(contract.physicalRemovalOwner === 66 && contract.cutoverOwner === 12, "CLI lifecycle owners drifted");

const namingCli = naming.surfaces.find(({ id }) => id === "binary.cli");
check(
  namingCli?.canonical === contract.binary && namingCli.legacy.includes("buzz"),
  "CLI contract disagrees with the naming contract",
);
check(commands.schemaVersion === 1, "wrong command grammar version");
check(commands.paths.length === 115, "v1 command grammar must contain 115 leaf commands");
check(new Set(commands.paths).size === commands.paths.length, "command grammar contains duplicates");

const nimPathsBlock = nimPolicy.match(/CliCommandPaths\* = \[([\s\S]*?)\n  \]/)?.[1];
check(nimPathsBlock, "Nim CLI command table is missing");
const nimPaths = [...nimPathsBlock.matchAll(/"([^"]+)"/g)].map((match) => match[1]);
check(
  JSON.stringify(nimPaths) === JSON.stringify(commands.paths),
  "Nim command policy disagrees with commands.json",
);

check(
  contract.boundaryOperation === "domain.cli.policy" &&
    nimWorker.includes('"domain.cli.policy"') &&
    rustBoundary.includes('"domain.cli.policy"'),
  "typed CLI policy operation is not wired on both boundary sides",
);
check(/\[\[bin\]\]\s+name = "nimino"/m.test(cargo), "nimino binary target is missing");
check(!/\[\[bin\]\]\s+name = "buzz"/m.test(cargo), "legacy buzz binary target is forbidden");
check(!cliSource.includes("Examples:\\n  buzz "), "legacy buzz invocation remains in CLI help");

const expectedAdapters = Object.fromEntries(
  ["list", "get", "create", "update", "trigger", "runs", "approve"].map((name) => [
    `workflows.${name}`,
    "domain.workflow.policy",
  ]),
);
expectedAdapters["workflows.delete"] = "domain.event.policy";
check(
  JSON.stringify(Object.entries(contract.workflowAdapters).sort()) ===
    JSON.stringify(Object.entries(expectedAdapters).sort()),
  "workflow adapter map drifted",
);

const names = new Set();
const failureKinds = new Set();
for (const testCase of golden.cases) {
  check(!names.has(testCase.name), `duplicate golden case: ${testCase.name}`);
  names.add(testCase.name);
  check(testCase.invariant.length > 0, `${testCase.name}: missing invariant`);
  check(testCase.input.decision === testCase.expected.decision, `${testCase.name}: decision mismatch`);
  if (testCase.input.decision === "failure") failureKinds.add(testCase.input.kind);
}
for (const kind of ["usage", "relay", "network", "delivery_unknown", "conflict"]) {
  check(failureKinds.has(kind), `golden corpus does not exercise ${kind}`);
}
check(names.has("old-buzz-command-rejected"), "golden corpus must reject the old Buzz grammar");

console.log(`Nimino CLI contract verified (${commands.paths.length} commands, ${golden.cases.length} cases)`);
