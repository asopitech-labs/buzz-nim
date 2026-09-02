#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-wsl-bundle/v1/contract.json"),
    "utf8",
  ),
);

function fail(message) {
  throw new Error(message);
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
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

function compose(values) {
  const allowed = [
    "release-set",
    "resolved-tag-commit",
    "artifact-dir",
    "output",
  ];
  for (const key of values.keys())
    if (!allowed.includes(key)) fail(`unknown argument: --${key}`);
  const releaseSetPath = values.get("release-set");
  const resolvedTagCommit = values.get("resolved-tag-commit");
  const artifactDirectory = values.get("artifact-dir");
  const output = values.get("output");
  if (!releaseSetPath || !resolvedTagCommit || !artifactDirectory || !output) {
    fail(
      "release-set, resolved-tag-commit, artifact-dir, and output are required",
    );
  }

  const verified = spawnSync(
    process.execPath,
    [
      join(root, "scripts/nimino-release-set.mjs"),
      "verify",
      "--manifest",
      releaseSetPath,
      "--resolved-tag-commit",
      resolvedTagCommit,
      "--artifact-dir",
      artifactDirectory,
    ],
    { cwd: root, encoding: "utf8" },
  );
  if (verified.status !== 0)
    fail(verified.stderr.trim() || "release set verification failed");

  const releaseSetSource = resolve(root, releaseSetPath);
  const releaseSet = JSON.parse(readFileSync(releaseSetSource, "utf8"));
  if (releaseSet.authority !== contract.releaseSetAuthority)
    fail("release set authority mismatch");
  const artifacts = new Map(
    releaseSet.artifacts.map((artifact) => [artifact.id, artifact]),
  );
  const selected = contract.components.map((component) => {
    const artifact = artifacts.get(component.artifactId);
    if (!artifact)
      fail(`required WSL artifact is missing: ${component.artifactId}`);
    const source = resolve(root, artifactDirectory, artifact.fileName);
    if (!existsSync(source) || !statSync(source).isFile())
      fail(`artifact is missing: ${artifact.fileName}`);
    return { ...component, artifact, source };
  });

  const outputPath = resolve(root, output);
  if (existsSync(outputPath)) fail("bundle output already exists");
  mkdirSync(dirname(outputPath), { recursive: true });
  let staging = mkdtempSync(join(dirname(outputPath), ".nimino-wsl-bundle-"));
  try {
    const bin = join(staging, "bin");
    mkdirSync(bin);
    for (const { installName, source } of selected) {
      const destination = join(bin, installName);
      copyFileSync(source, destination);
      chmodSync(destination, 0o755);
    }
    copyFileSync(releaseSetSource, join(staging, "release-set.json"));
    writeFileSync(
      join(staging, "release-set-id"),
      `${releaseSet.releaseSetId}\n`,
    );
    const manifest = {
      schemaVersion: 1,
      contract: contract.contract,
      platformContract: contract.platformContract,
      releaseSetId: releaseSet.releaseSetId,
      version: releaseSet.version,
      sourceCommit: releaseSet.sourceCommit,
      components: selected.map(
        ({ role, artifactId, installName, artifact }) => ({
          role,
          artifactId,
          installName,
          version: artifact.version,
          sourceFile: artifact.fileName,
          sha256: artifact.sha256,
          size: artifact.size,
        }),
      ),
    };
    writeFileSync(
      join(staging, "nimino-wsl-bundle.json"),
      `${JSON.stringify(manifest, null, 2)}\n`,
    );
    const checksummed = [
      ...contract.components.map(({ installName }) =>
        join(staging, "bin", installName),
      ),
      join(staging, "nimino-wsl-bundle.json"),
      join(staging, "release-set-id"),
      join(staging, "release-set.json"),
    ];
    writeFileSync(
      join(staging, "SHA256SUMS"),
      checksummed
        .map((path) => `${sha256(path)}  ${relative(staging, path)}\n`)
        .join(""),
    );
    renameSync(staging, outputPath);
    staging = "";
    console.log(releaseSet.releaseSetId);
  } finally {
    if (staging) rmSync(staging, { recursive: true, force: true });
  }
}

try {
  const { command, values } = parseArgs(process.argv.slice(2));
  if (command === "compose") compose(values);
  else fail("usage: nimino-wsl-bundle.mjs compose [options]");
} catch (error) {
  console.error(`error: ${error instanceof Error ? error.message : error}`);
  process.exitCode = 1;
}
