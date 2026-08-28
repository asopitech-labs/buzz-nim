#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-platform-release/v1/contract.json"),
    "utf8",
  ),
);
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

function fail(message) {
  throw new Error(message);
}

function args(argv) {
  const values = new Map();
  for (let index = 1; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!flag?.startsWith("--") || value === undefined)
      fail(`invalid argument: ${flag ?? ""}`);
    if (values.has(flag.slice(2))) fail(`duplicate argument: ${flag}`);
    values.set(flag.slice(2), value);
  }
  return { command: argv[0], values };
}

function required(values, names) {
  for (const name of names)
    if (!values.get(name)) fail(`--${name} is required`);
  for (const name of values.keys())
    if (!names.includes(name)) fail(`unknown argument: --${name}`);
}

function platform(id) {
  return (
    contract.platforms.find((candidate) => candidate.id === id) ??
    fail(`unsupported platform: ${id}`)
  );
}

function files(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = join(directory, entry.name);
    return entry.isDirectory() ? files(path) : entry.isFile() ? [path] : [];
  });
}

function digest(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function preflight(values) {
  required(values, ["platform"]);
  const selected = platform(values.get("platform"));
  const missing = selected.requiredEnvironment.filter(
    (name) => !process.env[name]?.trim(),
  );
  if (missing.length)
    fail(`signing environment missing: ${missing.join(", ")}`);
  if (process.env.NIMINO_UPDATER_ENDPOINT !== contract.updaterEndpoint)
    fail("updater endpoint does not match the canonical contract");
  console.log(selected.id);
}

function stage(values) {
  required(values, ["platform", "version", "bundle-dir", "output"]);
  const selected = platform(values.get("platform"));
  const version = values.get("version");
  if (!semver.test(version)) fail("version must be stable semver");
  const sourceDirectory = resolve(root, values.get("bundle-dir"));
  const outputDirectory = resolve(root, values.get("output"));
  const candidates = files(sourceDirectory);
  mkdirSync(outputDirectory, { recursive: true });
  const staged = [];
  for (const artifact of selected.artifacts) {
    const matches = candidates.filter((path) =>
      new RegExp(artifact.sourcePattern).test(path),
    );
    if (matches.length !== 1)
      fail(
        `${selected.id} ${artifact.role} expected exactly one artifact, found ${matches.length}`,
      );
    const fileName = artifact.fileName.replace("{version}", version);
    const destination = join(outputDirectory, fileName);
    copyFileSync(matches[0], destination);
    staged.push({
      role: artifact.role,
      fileName,
      sha256: digest(destination),
      size: statSync(destination).size,
    });
    if (artifact.signature) {
      const signature = `${matches[0]}.sig`;
      if (!existsSync(signature) || !readFileSync(signature, "utf8").trim())
        fail(`${selected.id} ${artifact.role} signature is missing or empty`);
      copyFileSync(signature, `${destination}.sig`);
      staged.push({
        role: `${artifact.role}-signature`,
        fileName: `${fileName}.sig`,
        sha256: digest(`${destination}.sig`),
        size: statSync(`${destination}.sig`).size,
      });
    }
  }
  writeFileSync(
    join(outputDirectory, "platform-artifacts.json"),
    `${JSON.stringify({ schemaVersion: 1, platform: selected.id, artifacts: staged }, null, 2)}\n`,
  );
}

function manifest(values) {
  required(values, [
    "release-set",
    "artifact-dir",
    "base-url",
    "wsl-bundle-dir",
    "wsl-archive",
    "output",
  ]);
  const releaseSetPath = resolve(root, values.get("release-set"));
  const artifactDirectory = resolve(root, values.get("artifact-dir"));
  const releaseSet = JSON.parse(readFileSync(releaseSetPath, "utf8"));
  const expectedBase = `https://github.com/${contract.sourceRepository}/releases/download/${releaseSet.tag}`;
  if (values.get("base-url") !== expectedBase)
    fail("artifact base URL is not canonical");
  const verified = spawnSync(
    process.execPath,
    [
      join(root, "scripts/nimino-release-set.mjs"),
      "verify",
      "--manifest",
      releaseSetPath,
      "--resolved-tag-commit",
      releaseSet.sourceCommit,
      "--artifact-dir",
      artifactDirectory,
    ],
    { cwd: root, encoding: "utf8" },
  );
  if (verified.status !== 0)
    fail(verified.stderr.trim() || "release-set verification failed");
  const artifacts = new Map(
    releaseSet.artifacts.map((artifact) => [artifact.id, artifact]),
  );
  for (const id of contract.wsl.componentArtifactIds)
    if (!artifacts.has(id)) fail(`release-set is missing WSL component: ${id}`);
  const platforms = {};
  for (const selected of contract.platforms) {
    for (const expected of selected.artifacts) {
      const expectedFile = expected.fileName.replace(
        "{version}",
        releaseSet.version,
      );
      const expectedId = `desktop-${selected.id}-${expected.role}`;
      const artifact = artifacts.get(expectedId);
      if (!artifact || artifact.fileName !== expectedFile)
        fail(`release-set desktop artifacts do not match ${selected.id}`);
      if (expected.signature) {
        const signature = artifacts.get(`${expectedId}-signature`);
        if (!signature || signature.fileName !== `${expectedFile}.sig`)
          fail(`release-set desktop signature does not match ${selected.id}`);
      }
    }
    const updater = selected.artifacts.find((artifact) => artifact.signature);
    assert(updater);
    const fileName = updater.fileName.replace("{version}", releaseSet.version);
    const id = `desktop-${selected.id}-${updater.role}`;
    const artifact = artifacts.get(id);
    const signature = artifacts.get(`${id}-signature`);
    if (
      !artifact ||
      artifact.fileName !== fileName ||
      !signature ||
      signature.fileName !== `${fileName}.sig`
    )
      fail(`release-set desktop artifacts do not match ${selected.id}`);
    platforms[selected.updaterKey] = {
      signature: readFileSync(
        join(artifactDirectory, signature.fileName),
        "utf8",
      ).trim(),
      url: `${expectedBase}/${encodeURIComponent(artifact.fileName)}`,
    };
    if (!platforms[selected.updaterKey].signature)
      fail(`empty updater signature: ${selected.id}`);
  }
  const wslManifest = JSON.parse(
    readFileSync(
      join(
        resolve(root, values.get("wsl-bundle-dir")),
        "nimino-wsl-bundle.json",
      ),
      "utf8",
    ),
  );
  if (wslManifest.releaseSetId !== releaseSet.releaseSetId)
    fail("WSL bundle release-set mismatch");
  const wslArchive = resolve(root, values.get("wsl-archive"));
  if (
    !existsSync(wslArchive) ||
    basename(wslArchive) !==
      contract.wsl.archive.replace("{version}", releaseSet.version)
  )
    fail("WSL archive does not match the platform contract");
  writeFileSync(
    resolve(root, values.get("output")),
    `${JSON.stringify({ version: releaseSet.version, notes: `Nimino ${releaseSet.version}`, pub_date: new Date(0).toISOString(), releaseSetId: releaseSet.releaseSetId, platforms, wsl: { platform: contract.wsl.id, url: `${expectedBase}/${encodeURIComponent(basename(wslArchive))}`, sha256: digest(wslArchive) } }, null, 2)}\n`,
  );
}

try {
  const { command, values } = args(process.argv.slice(2));
  if (command === "preflight") preflight(values);
  else if (command === "stage") stage(values);
  else if (command === "manifest") manifest(values);
  else
    fail(
      "usage: nimino-platform-artifacts.mjs <preflight|stage|manifest> [options]",
    );
} catch (error) {
  console.error(`error: ${error instanceof Error ? error.message : error}`);
  process.exitCode = 1;
}
