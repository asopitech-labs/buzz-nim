#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  existsSync,
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
    join(root, "contracts/nimino-release-set/v1/contract.json"),
    "utf8",
  ),
);
const hex40 = /^[0-9a-f]{40}$/;
const hex64 = /^[0-9a-f]{64}$/;
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

function fail(message) {
  throw new Error(message);
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function stable(value) {
  if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.keys(value)
      .sort()
      .map((key) => `${JSON.stringify(key)}:${stable(value[key])}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
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

function requireArgs(values, allowed) {
  for (const key of values.keys()) {
    if (!allowed.includes(key)) fail(`unknown argument: --${key}`);
  }
}

exactKeys(
  contract,
  [
    "schemaVersion",
    "contract",
    "compatibilityMode",
    "tagPrefix",
    "requiredComponents",
    "digestAlgorithm",
    "artifactMinimum",
    "sameVersionPolicy",
    "downgradePolicy",
    "promotionInput",
    "physicalPromotionOwner",
  ],
  "release-set contract",
);
if (
  contract.schemaVersion !== 1 ||
  contract.contract !== "nimino.release-set" ||
  contract.compatibilityMode !== false ||
  contract.digestAlgorithm !== "sha256" ||
  contract.promotionInput !== "releaseSetId" ||
  contract.physicalPromotionOwner !== 68
)
  fail("release-set contract is invalid");

function versionFrom(path, section) {
  const source = readFileSync(join(root, path), "utf8");
  const body = section
    ? source.match(
        new RegExp(
          `\\[${section.replace(".", "\\.")}\\]([\\s\\S]*?)(?=\\n\\[|$)`,
        ),
      )?.[1]
    : source;
  const version = body?.match(/^version\s*=\s*"([^"]+)"/m)?.[1];
  if (!version) fail(`version missing from ${path}`);
  return version;
}

function digestFiles(paths) {
  const hash = createHash("sha256");
  for (const path of [...paths].sort()) {
    hash.update(`${path}\0${sha256(readFileSync(join(root, path)))}\n`);
  }
  return hash.digest("hex");
}

function filesUnder(directory) {
  const files = [];
  for (const entry of readdirSync(join(root, directory), {
    withFileTypes: true,
  })) {
    if (entry.name === "nimcache") continue;
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...filesUnder(path));
    else if (entry.isFile()) files.push(path);
  }
  return files;
}

function sourceComponents(commit) {
  const chirps = JSON.parse(
    readFileSync(join(root, "contracts/chirps-v0.6.3.json"), "utf8"),
  );
  const boundary = readFileSync(
    join(root, "contracts/nim-rust-boundary/v1/schema.sha256"),
    "utf8",
  ).match(/^# bundle-sha256: ([0-9a-f]{64})$/m)?.[1];
  if (!boundary) fail("boundary schema bundle digest is missing");
  return [
    {
      id: "alopex-chirps",
      version: chirps.upstream.version,
      sourceSha: chirps.upstream.checksum,
      sha256: chirps.upstream.checksum,
    },
    {
      id: "boundary-schema",
      version: "1.0.0",
      sourceSha: commit,
      sha256: boundary,
    },
    {
      id: "nim-core",
      version: versionFrom("nim/nimino_core/nimino_core.nimble"),
      sourceSha: commit,
      sha256: digestFiles(filesUnder("nim/nimino_core")),
    },
    {
      id: "rust-workspace",
      version: versionFrom("Cargo.toml", "workspace.package"),
      sourceSha: commit,
      sha256: digestFiles(["Cargo.lock", "Cargo.toml"]),
    },
  ];
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
    if (key === "artifact") {
      values.set(key, [...(values.get(key) ?? []), value]);
    } else {
      if (values.has(key)) fail(`duplicate argument: ${flag}`);
      values.set(key, value);
    }
  }
  return { command, values };
}

function artifactFrom(specification) {
  const [id, version, ...pathParts] = specification.split(":");
  const path = pathParts.join(":");
  if (!/^[a-z0-9][a-z0-9-]*$/.test(id) || !semver.test(version) || !path) {
    fail(`invalid artifact specification: ${specification}`);
  }
  const resolved = resolve(root, path);
  if (!existsSync(resolved) || !statSync(resolved).isFile()) {
    fail(`artifact is missing: ${path}`);
  }
  return {
    id,
    version,
    fileName: basename(resolved),
    sha256: sha256(readFileSync(resolved)),
    size: statSync(resolved).size,
  };
}

function releaseSetId(releaseSet) {
  const { releaseSetId: _ignored, ...content } = releaseSet;
  return sha256(stable(content));
}

function validateIdentity(releaseSet) {
  exactKeys(
    releaseSet,
    [
      "schemaVersion",
      "authority",
      "releaseSetId",
      "version",
      "tag",
      "sourceCommit",
      "components",
      "artifacts",
    ],
    "release set",
  );
  if (
    releaseSet.schemaVersion !== 1 ||
    releaseSet.authority !== contract.contract ||
    !semver.test(releaseSet.version) ||
    releaseSet.tag !== `${contract.tagPrefix}${releaseSet.version}` ||
    !hex40.test(releaseSet.sourceCommit) ||
    !hex64.test(releaseSet.releaseSetId) ||
    releaseSet.releaseSetId !== releaseSetId(releaseSet)
  ) {
    fail("release set identity is invalid");
  }
}

function validateEntries(releaseSet) {
  const componentIds = [];
  for (const component of releaseSet.components) {
    exactKeys(
      component,
      ["id", "version", "sourceSha", "sha256"],
      `component ${component.id}`,
    );
    if (
      !/^[a-z0-9][a-z0-9-]*$/.test(component.id) ||
      !semver.test(component.version) ||
      !/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(component.sourceSha) ||
      !hex64.test(component.sha256)
    )
      fail(`component is invalid: ${component.id}`);
    componentIds.push(component.id);
  }
  if (
    JSON.stringify(componentIds) !==
      JSON.stringify(contract.requiredComponents) ||
    JSON.stringify(releaseSet.components) !==
      JSON.stringify(sourceComponents(releaseSet.sourceCommit))
  )
    fail("source component pins do not match this checkout");

  if (
    !Array.isArray(releaseSet.artifacts) ||
    releaseSet.artifacts.length < contract.artifactMinimum
  ) {
    fail("release set has no artifacts");
  }
  const artifactIds = [];
  for (const artifact of releaseSet.artifacts) {
    exactKeys(
      artifact,
      ["id", "version", "fileName", "sha256", "size"],
      `artifact ${artifact.id}`,
    );
    if (
      !/^[a-z0-9][a-z0-9-]*$/.test(artifact.id) ||
      !semver.test(artifact.version) ||
      basename(artifact.fileName) !== artifact.fileName ||
      !hex64.test(artifact.sha256) ||
      !Number.isSafeInteger(artifact.size) ||
      artifact.size < 1
    )
      fail(`artifact is invalid: ${artifact.id}`);
    artifactIds.push(artifact.id);
  }
  if (
    new Set(artifactIds).size !== artifactIds.length ||
    artifactIds.join() !== [...artifactIds].sort().join() ||
    new Set(releaseSet.artifacts.map(({ fileName }) => fileName)).size !==
      releaseSet.artifacts.length
  ) {
    fail("artifact IDs must be unique and sorted; filenames must be unique");
  }
}

function compareVersion(left, right) {
  const a = left.split(".").map(Number);
  const b = right.split(".").map(Number);
  for (let index = 0; index < 3; index += 1) {
    if (a[index] !== b[index]) return a[index] - b[index];
  }
  return 0;
}

function create(values) {
  requireArgs(values, ["version", "tag", "commit", "output", "artifact"]);
  const version = values.get("version");
  const tag = values.get("tag");
  const sourceCommit = values.get("commit");
  const output = values.get("output");
  const artifactSpecs = values.get("artifact") ?? [];
  if (
    !semver.test(version) ||
    tag !== `${contract.tagPrefix}${version}` ||
    !hex40.test(sourceCommit)
  ) {
    fail("version, tag, or commit is invalid");
  }
  if (!output || artifactSpecs.length < contract.artifactMinimum)
    fail("output and artifact are required");
  const artifacts = artifactSpecs
    .map(artifactFrom)
    .sort((left, right) => left.id.localeCompare(right.id));
  if (new Set(artifacts.map(({ id }) => id)).size !== artifacts.length)
    fail("duplicate artifact ID");
  if (
    new Set(artifacts.map(({ fileName }) => fileName)).size !== artifacts.length
  )
    fail("duplicate artifact filename");
  const releaseSet = {
    schemaVersion: 1,
    authority: contract.contract,
    releaseSetId: "",
    version,
    tag,
    sourceCommit,
    components: sourceComponents(sourceCommit),
    artifacts,
  };
  releaseSet.releaseSetId = releaseSetId(releaseSet);
  writeFileSync(
    resolve(root, output),
    `${JSON.stringify(releaseSet, null, 2)}\n`,
  );
  console.log(releaseSet.releaseSetId);
}

function verify(values) {
  requireArgs(values, [
    "manifest",
    "resolved-tag-commit",
    "artifact-dir",
    "previous",
  ]);
  const manifestPath = values.get("manifest");
  const resolvedTagCommit = values.get("resolved-tag-commit");
  if (!manifestPath || !hex40.test(resolvedTagCommit))
    fail("manifest and resolved tag commit are required");
  const releaseSet = JSON.parse(
    readFileSync(resolve(root, manifestPath), "utf8"),
  );
  validateIdentity(releaseSet);
  validateEntries(releaseSet);
  if (resolvedTagCommit !== releaseSet.sourceCommit)
    fail("tag resolved to a different commit");

  const artifactDirectory = values.get("artifact-dir");
  if (artifactDirectory) {
    for (const artifact of releaseSet.artifacts) {
      const path = resolve(root, artifactDirectory, artifact.fileName);
      if (
        !existsSync(path) ||
        statSync(path).size !== artifact.size ||
        sha256(readFileSync(path)) !== artifact.sha256
      )
        fail(`artifact digest mismatch: ${artifact.id}`);
    }
  }

  const previousPath = values.get("previous");
  if (previousPath) {
    const previous = JSON.parse(
      readFileSync(resolve(root, previousPath), "utf8"),
    );
    validateIdentity(previous);
    const comparison = compareVersion(releaseSet.version, previous.version);
    if (comparison < 0) fail("release version downgrade rejected");
    if (comparison === 0 && releaseSet.releaseSetId !== previous.releaseSetId) {
      fail("same version has different release set content");
    }
  }
  console.log(releaseSet.releaseSetId);
}

try {
  const { command, values } = parseArgs(process.argv.slice(2));
  if (command === "create") create(values);
  else if (command === "verify") verify(values);
  else fail("usage: nimino-release-set.mjs <create|verify> [options]");
} catch (error) {
  console.error(`error: ${error instanceof Error ? error.message : error}`);
  process.exitCode = 1;
}
