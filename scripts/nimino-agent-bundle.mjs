#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-agent-bundle/v1/contract.json"),
    "utf8",
  ),
);

function fail(message) {
  throw new Error(message);
}

function exactKeys(value, keys, context) {
  if (
    !value ||
    typeof value !== "object" ||
    JSON.stringify(Object.keys(value).sort()) !==
      JSON.stringify([...keys].sort())
  ) {
    fail(`${context} fields are invalid`);
  }
}

exactKeys(
  contract,
  [
    "schemaVersion",
    "contract",
    "releaseSetAuthority",
    "compatibilityAliases",
    "components",
    "legacyAllInOne",
    "physicalDeletionOwner",
  ],
  "agent bundle contract",
);
if (
  contract.schemaVersion !== 1 ||
  contract.contract !== "nimino.agent-bundle" ||
  contract.releaseSetAuthority !== "nimino.release-set" ||
  contract.compatibilityAliases !== false ||
  contract.legacyAllInOne !== "forbidden" ||
  contract.physicalDeletionOwner !== 66 ||
  !Array.isArray(contract.components) ||
  contract.components.length !== 4
) {
  fail("agent bundle contract is invalid");
}

for (const component of contract.components) {
  exactKeys(component, ["role", "artifactId", "installName"], "component");
  if (
    !/^[a-z][a-z0-9-]*$/.test(component.role) ||
    !/^[a-z][a-z0-9-]*$/.test(component.artifactId) ||
    !/^nimino(?:-[a-z0-9-]+)?$/.test(component.installName)
  ) {
    fail("agent bundle component is invalid");
  }
}
if (
  new Set(contract.components.map(({ role }) => role)).size !== 4 ||
  new Set(contract.components.map(({ artifactId }) => artifactId)).size !== 4 ||
  new Set(contract.components.map(({ installName }) => installName)).size !== 4
) {
  fail("agent bundle component fields must be unique");
}

function parseArgs(argv) {
  const command = argv[0];
  const values = new Map();
  for (let index = 1; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!flag?.startsWith("--") || value === undefined)
      fail(`invalid argument: ${flag ?? ""}`);
    const key = flag.slice(2);
    if (values.has(key)) fail(`duplicate argument: ${flag}`);
    values.set(key, value);
  }
  return { command, values };
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function compose(values) {
  const allowed = [
    "release-set",
    "resolved-tag-commit",
    "artifact-dir",
    "output",
  ];
  for (const key of values.keys()) {
    if (!allowed.includes(key)) fail(`unknown argument: --${key}`);
  }
  const releaseSetPath = values.get("release-set");
  const resolvedTagCommit = values.get("resolved-tag-commit");
  const artifactDirectory = values.get("artifact-dir");
  const output = values.get("output");
  if (!releaseSetPath || !resolvedTagCommit || !artifactDirectory || !output)
    fail(
      "release-set, resolved-tag-commit, artifact-dir, and output are required",
    );

  const verified = spawnSync(
    process.execPath,
    [
      join(root, "scripts/nimino-release-set.mjs"),
      "verify",
      "--manifest",
      releaseSetPath,
      "--resolved-tag-commit",
      resolvedTagCommit,
    ],
    { cwd: root, encoding: "utf8" },
  );
  if (verified.status !== 0)
    fail(verified.stderr.trim() || "release set verification failed");

  const releaseSet = JSON.parse(
    readFileSync(resolve(root, releaseSetPath), "utf8"),
  );
  if (releaseSet.authority !== contract.releaseSetAuthority)
    fail("release set authority does not match bundle contract");
  const artifacts = new Map(
    releaseSet.artifacts.map((artifact) => [artifact.id, artifact]),
  );
  const selected = contract.components.map((component) => {
    const artifact = artifacts.get(component.artifactId);
    if (!artifact)
      fail(`required bundle artifact is missing: ${component.artifactId}`);
    const source = resolve(root, artifactDirectory, artifact.fileName);
    if (
      !existsSync(source) ||
      !statSync(source).isFile() ||
      statSync(source).size !== artifact.size ||
      sha256(source) !== artifact.sha256
    ) {
      fail(`bundle artifact digest mismatch: ${component.artifactId}`);
    }
    return { ...component, artifact, source };
  });

  const outputPath = resolve(root, output);
  if (existsSync(outputPath)) fail("bundle output already exists");
  const bin = join(outputPath, "bin");
  mkdirSync(bin, { recursive: true });
  for (const { installName, source } of selected) {
    const destination = join(bin, installName);
    copyFileSync(source, destination);
    chmodSync(destination, 0o755);
  }
  const manifest = {
    schemaVersion: 1,
    contract: contract.contract,
    releaseSetId: releaseSet.releaseSetId,
    version: releaseSet.version,
    sourceCommit: releaseSet.sourceCommit,
    components: selected.map(({ role, artifactId, installName, artifact }) => ({
      role,
      artifactId,
      installName,
      version: artifact.version,
      sourceFile: artifact.fileName,
      sha256: artifact.sha256,
      size: artifact.size,
    })),
  };
  writeFileSync(
    join(outputPath, "nimino-agent-bundle.json"),
    `${JSON.stringify(manifest, null, 2)}\n`,
  );
  console.log(releaseSet.releaseSetId);
}

try {
  const { command, values } = parseArgs(process.argv.slice(2));
  if (command === "compose") compose(values);
  else fail("usage: nimino-agent-bundle.mjs compose [options]");
} catch (error) {
  console.error(`error: ${error instanceof Error ? error.message : error}`);
  process.exitCode = 1;
}
