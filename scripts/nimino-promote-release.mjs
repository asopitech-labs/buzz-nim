#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-promotion/v1/contract.json"),
    "utf8",
  ),
);
const platformContract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-platform-release/v1/contract.json"),
    "utf8",
  ),
);
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;
const releaseSetIdPattern = /^[0-9a-f]{64}$/;

function fail(message) {
  throw new Error(message);
}

function digest(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function run(command, parameters, allowFailure = false) {
  const result = spawnSync(command, parameters, {
    cwd: root,
    encoding: "utf8",
    env: process.env,
  });
  if (!allowFailure && result.status !== 0)
    fail(result.stderr.trim() || `${command} ${parameters.join(" ")} failed`);
  return result;
}

function gh(...parameters) {
  return run("gh", parameters).stdout.trim();
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

function preflight(releaseSetId) {
  if (process.env.GITHUB_REPOSITORY !== contract.sourceRepository)
    fail(`promotion is restricted to ${contract.sourceRepository}`);
  for (const name of contract.requiredEnvironment)
    if (!process.env[name]?.trim())
      fail(`required environment missing: ${name}`);
  if (!releaseSetIdPattern.test(releaseSetId))
    fail("release-set ID must be 64 lowercase hex characters");
}

function releaseSetAt(path) {
  const releaseSet = JSON.parse(readFileSync(path, "utf8"));
  if (
    releaseSet.authority !== contract.releaseSetAuthority ||
    !releaseSetIdPattern.test(releaseSet.releaseSetId) ||
    !semver.test(releaseSet.version) ||
    releaseSet.tag !== `${contract.tagPrefix}${releaseSet.version}`
  )
    fail("candidate release-set identity is invalid");
  return releaseSet;
}

function resolveCandidate(releaseSetId) {
  const releases = JSON.parse(
    gh(
      "release",
      "list",
      "--repo",
      contract.sourceRepository,
      "--limit",
      "1000",
      "--json",
      "tagName,isDraft,isPrerelease",
    ),
  ).filter(({ tagName }) =>
    new RegExp(`^${contract.tagPrefix}[0-9]+\\.[0-9]+\\.[0-9]+$`).test(tagName),
  );
  const work = mkdtempSync(join(tmpdir(), "nimino-promotion-resolve-"));
  const matches = [];
  try {
    for (const release of releases) {
      const directory = join(work, release.tagName);
      mkdirSync(directory);
      const downloaded = run(
        "gh",
        [
          "release",
          "download",
          release.tagName,
          "--repo",
          contract.sourceRepository,
          "--pattern",
          "release-set.json",
          "--dir",
          directory,
        ],
        true,
      );
      if (downloaded.status !== 0) continue;
      const releaseSet = releaseSetAt(join(directory, "release-set.json"));
      if (releaseSet.releaseSetId === releaseSetId)
        matches.push({ ...release, ...releaseSet });
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
  if (matches.length !== 1)
    fail(`release-set ID resolved to ${matches.length} candidates`);
  return matches[0];
}

function candidateAssetNames(releaseSet) {
  return [
    ...releaseSet.artifacts.map(({ fileName }) => fileName),
    ...contract.requiredCandidateAssets.map((name) =>
      name.replace("{version}", releaseSet.version),
    ),
  ].sort();
}

function verifyBlob(directory, fileName, bundleName, tag) {
  run("cosign", [
    "verify-blob",
    "--bundle",
    join(directory, bundleName),
    "--certificate-identity",
    `https://github.com/${contract.sourceRepository}/${contract.signatureWorkflow}@refs/tags/${tag}`,
    "--certificate-oidc-issuer",
    contract.signatureIssuer,
    join(directory, fileName),
  ]);
}

function verifyCandidate(directory, releaseSet) {
  const actual = JSON.parse(
    gh(
      "release",
      "view",
      releaseSet.tag,
      "--repo",
      contract.sourceRepository,
      "--json",
      "isDraft,isPrerelease,targetCommitish,assets",
    ),
  );
  if (actual.isPrerelease) fail("candidate release must not be a prerelease");
  const names = actual.assets.map(({ name }) => name).sort();
  if (JSON.stringify(names) !== JSON.stringify(candidateAssetNames(releaseSet)))
    fail(
      "candidate release asset inventory is incomplete or contains stale files",
    );
  const updater = JSON.parse(
    readFileSync(join(directory, "latest.json"), "utf8"),
  );
  const updaterKeys = platformContract.platforms
    .map(({ updaterKey }) => updaterKey)
    .sort();
  if (
    updater.version !== releaseSet.version ||
    updater.releaseSetId !== releaseSet.releaseSetId ||
    JSON.stringify(Object.keys(updater.platforms ?? {}).sort()) !==
      JSON.stringify(updaterKeys) ||
    updater.wsl?.platform !== platformContract.wsl.id ||
    !/^[0-9a-f]{64}$/.test(updater.wsl?.sha256 ?? "")
  )
    fail("updater manifest identity or platform matrix is invalid");
  for (const entry of [...Object.values(updater.platforms), updater.wsl]) {
    if (
      (entry !== updater.wsl &&
        (typeof entry.signature !== "string" || !entry.signature)) ||
      typeof entry.url !== "string"
    )
      fail("updater signature or URL is invalid");
    const url = new URL(entry.url);
    if (
      `${url.origin}${dirname(url.pathname)}` !==
        `https://github.com/${contract.sourceRepository}/releases/download/${releaseSet.tag}` ||
      !names.includes(decodeURIComponent(basename(url.pathname)))
    )
      fail("updater references a non-canonical or missing candidate asset");
  }
  const tagCommit = gh(
    "api",
    `repos/${contract.sourceRepository}/commits/${releaseSet.tag}`,
    "--jq",
    ".sha",
  );
  const targetCommit = gh(
    "api",
    `repos/${contract.sourceRepository}/commits/${actual.targetCommitish}`,
    "--jq",
    ".sha",
  );
  if (tagCommit !== releaseSet.sourceCommit || targetCommit !== tagCommit)
    fail("candidate tag, target, and release-set source differ");
  const sourceDirectory = join(dirname(directory), "source");
  run("git", [
    "clone",
    "--quiet",
    "--shared",
    "--no-checkout",
    root,
    sourceDirectory,
  ]);
  run("git", ["-C", sourceDirectory, "checkout", "--quiet", tagCommit]);
  run(process.execPath, [
    join(root, "scripts/nimino-release-supply-chain.mjs"),
    "verify",
    "--release-set",
    join(directory, "release-set.json"),
    "--artifact-dir",
    directory,
    "--sbom",
    join(directory, contract.sbom.fileName),
    "--checksums",
    join(directory, contract.checksums.fileName),
    "--resolved-tag-commit",
    tagCommit,
    "--source-root",
    sourceDirectory,
  ]);
  verifyBlob(
    directory,
    "release-set.json",
    "release-set.sigstore.json",
    releaseSet.tag,
  );
  verifyBlob(
    directory,
    contract.sbom.fileName,
    contract.sbom.signature,
    releaseSet.tag,
  );
  verifyBlob(directory, "latest.json", "latest.sigstore.json", releaseSet.tag);
  verifyBlob(
    directory,
    `nimino-${releaseSet.version}-wsl2-x86_64.tar.gz`,
    "nimino-wsl.sigstore.json",
    releaseSet.tag,
  );
  verifyBlob(
    directory,
    contract.checksums.fileName,
    contract.checksums.signature,
    releaseSet.tag,
  );
  gh(
    "attestation",
    "verify",
    join(directory, "release-set.json"),
    "--repo",
    contract.sourceRepository,
  );
  gh(
    "attestation",
    "verify",
    join(directory, "release-set.json"),
    "--repo",
    contract.sourceRepository,
    "--predicate-type",
    contract.sbom.predicateType,
  );
  return { release: actual, tagCommit };
}

function compareVersions(left, right) {
  const a = left.split(".").map(Number);
  const b = right.split(".").map(Number);
  for (let index = 0; index < 3; index += 1)
    if (a[index] !== b[index]) return a[index] - b[index];
  return 0;
}

function validSummary(value) {
  return (
    value &&
    JSON.stringify(Object.keys(value).sort()) ===
      JSON.stringify(
        [
          "checksumsSha256",
          "releaseSetId",
          "releaseSetSha256",
          "sbomSha256",
          "sourceCommit",
          "tag",
          "updaterSha256",
          "version",
        ].sort(),
      ) &&
    releaseSetIdPattern.test(value.releaseSetId) &&
    semver.test(value.version) &&
    value.tag === `${contract.tagPrefix}${value.version}` &&
    /^[0-9a-f]{40}$/.test(value.sourceCommit) &&
    [
      value.releaseSetSha256,
      value.updaterSha256,
      value.sbomSha256,
      value.checksumsSha256,
    ].every((entry) => releaseSetIdPattern.test(entry))
  );
}

function publishCandidate(releaseSet, release) {
  if (release.isDraft)
    gh(
      "release",
      "edit",
      releaseSet.tag,
      "--repo",
      contract.sourceRepository,
      "--draft=false",
    );
  const published = JSON.parse(
    gh(
      "release",
      "view",
      releaseSet.tag,
      "--repo",
      contract.sourceRepository,
      "--json",
      "isDraft",
    ),
  );
  if (published.isDraft) fail("candidate release remained a draft");
}

function readStable(work) {
  const viewed = run(
    "gh",
    [
      "release",
      "view",
      contract.stableReleaseTag,
      "--repo",
      contract.sourceRepository,
      "--json",
      "assets",
    ],
    true,
  );
  if (viewed.status !== 0) return null;
  const names = JSON.parse(viewed.stdout)
    .assets.map(({ name }) => name)
    .sort();
  if (
    JSON.stringify(names) !==
    JSON.stringify(["latest.json", contract.promotionRecord].sort())
  )
    fail("stable release is partially populated");
  const directory = join(work, "current");
  mkdirSync(directory, { recursive: true });
  gh(
    "release",
    "download",
    contract.stableReleaseTag,
    "--repo",
    contract.sourceRepository,
    "--dir",
    directory,
  );
  const record = JSON.parse(
    readFileSync(join(directory, contract.promotionRecord), "utf8"),
  );
  if (
    record.schemaVersion !== 1 ||
    record.authority !== contract.contract ||
    !validSummary(record.current) ||
    (record.previous !== null && !validSummary(record.previous)) ||
    record.current.updaterSha256 !== digest(join(directory, "latest.json"))
  )
    fail("stable promotion record is invalid");
  return { directory, record };
}

function rollbackStable(previous) {
  if (previous) {
    run("gh", [
      "release",
      "upload",
      contract.stableReleaseTag,
      join(previous.directory, "latest.json"),
      join(previous.directory, contract.promotionRecord),
      "--repo",
      contract.sourceRepository,
      "--clobber",
    ]);
  } else {
    run("gh", [
      "release",
      "delete",
      contract.stableReleaseTag,
      "--repo",
      contract.sourceRepository,
      "--yes",
    ]);
  }
}

function promote(values) {
  required(values, ["release-set-id", "tag"]);
  const releaseSetId = values.get("release-set-id");
  preflight(releaseSetId);
  const resolved = resolveCandidate(releaseSetId);
  if (resolved.tag !== values.get("tag"))
    fail("resolved candidate tag changed");
  const work = mkdtempSync(join(tmpdir(), "nimino-promotion-"));
  try {
    const candidateDirectory = join(work, "candidate");
    mkdirSync(candidateDirectory);
    gh(
      "release",
      "download",
      resolved.tag,
      "--repo",
      contract.sourceRepository,
      "--dir",
      candidateDirectory,
    );
    const releaseSet = releaseSetAt(
      join(candidateDirectory, "release-set.json"),
    );
    if (releaseSet.releaseSetId !== releaseSetId)
      fail("downloaded release-set changed during promotion");
    const { release, tagCommit } = verifyCandidate(
      candidateDirectory,
      releaseSet,
    );
    const latest = join(candidateDirectory, "latest.json");
    const sbom = join(candidateDirectory, contract.sbom.fileName);
    const summary = {
      releaseSetId,
      version: releaseSet.version,
      tag: releaseSet.tag,
      sourceCommit: tagCommit,
      releaseSetSha256: digest(join(candidateDirectory, "release-set.json")),
      updaterSha256: digest(latest),
      sbomSha256: digest(sbom),
      checksumsSha256: digest(
        join(candidateDirectory, contract.checksums.fileName),
      ),
    };
    const previous = readStable(work);
    if (previous?.record.current.releaseSetId === releaseSetId) {
      if (
        JSON.stringify(previous.record.current) !== JSON.stringify(summary) ||
        readFileSync(join(previous.directory, "latest.json"), "utf8") !==
          readFileSync(latest, "utf8")
      )
        fail("same release-set ID has different promoted bytes");
      publishCandidate(releaseSet, release);
      console.log(`already promoted ${releaseSetId}`);
      return;
    }
    if (previous) {
      const comparison = compareVersions(
        releaseSet.version,
        previous.record.current.version,
      );
      if (comparison < 0) fail("promotion downgrade rejected");
      if (comparison === 0) fail("same version has a different release-set ID");
    }
    publishCandidate(releaseSet, release);
    const staged = join(work, "staged");
    mkdirSync(staged);
    copyFileSync(latest, join(staged, "latest.json"));
    writeFileSync(
      join(staged, contract.promotionRecord),
      `${JSON.stringify({ schemaVersion: 1, authority: contract.contract, current: summary, previous: previous?.record.current ?? null }, null, 2)}\n`,
    );
    let stableCreated = false;
    try {
      if (!previous) {
        gh(
          "release",
          "create",
          contract.stableReleaseTag,
          "--repo",
          contract.sourceRepository,
          "--title",
          "Nimino stable updater",
          "--notes",
          "Managed by the release-set promotion gate.",
          "--latest=false",
        );
        stableCreated = true;
      }
      gh(
        "release",
        "upload",
        contract.stableReleaseTag,
        join(staged, "latest.json"),
        "--repo",
        contract.sourceRepository,
        "--clobber",
      );
      gh(
        "release",
        "upload",
        contract.stableReleaseTag,
        join(staged, contract.promotionRecord),
        "--repo",
        contract.sourceRepository,
        "--clobber",
      );
      const verified = readStable(join(work, "verify"));
      if (
        !verified ||
        verified.record.current.releaseSetId !== releaseSetId ||
        readFileSync(join(verified.directory, "latest.json"), "utf8") !==
          readFileSync(latest, "utf8")
      )
        fail("served promotion does not match the candidate");
    } catch (error) {
      if (previous || stableCreated) {
        try {
          rollbackStable(previous);
        } catch (rollbackError) {
          fail(
            `promotion failed and rollback failed: ${rollbackError instanceof Error ? rollbackError.message : rollbackError}`,
          );
        }
      }
      throw error;
    }
    console.log(`promoted ${releaseSetId}`);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

function resolveCommand(values) {
  required(values, ["release-set-id", "output"]);
  const releaseSetId = values.get("release-set-id");
  preflight(releaseSetId);
  const candidate = resolveCandidate(releaseSetId);
  writeFileSync(
    resolve(root, values.get("output")),
    `${JSON.stringify({ releaseSetId, tag: candidate.tag, version: candidate.version, sourceCommit: candidate.sourceCommit }, null, 2)}\n`,
  );
}

try {
  const { command, values } = args(process.argv.slice(2));
  if (command === "resolve") resolveCommand(values);
  else if (command === "promote") promote(values);
  else fail("usage: nimino-promote-release.mjs <resolve|promote> [options]");
} catch (error) {
  console.error(`error: ${error instanceof Error ? error.message : error}`);
  process.exitCode = 1;
}
