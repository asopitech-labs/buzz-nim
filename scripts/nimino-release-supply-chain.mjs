#!/usr/bin/env node

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
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
    join(root, "contracts/nimino-promotion/v1/contract.json"),
    "utf8",
  ),
);

function fail(message) {
  throw new Error(message);
}

function digest(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
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

function required(values, names, optional = []) {
  for (const name of names)
    if (!values.get(name)) fail(`--${name} is required`);
  for (const name of values.keys())
    if (![...names, ...optional].includes(name))
      fail(`unknown argument: --${name}`);
}

function spdxId(value) {
  return `SPDXRef-${value.replaceAll(/[^A-Za-z0-9.-]/g, "-")}`;
}

function sbomFor(releaseSet) {
  const releaseId = spdxId("nimino-release");
  const packages = [
    {
      name: "Nimino",
      SPDXID: releaseId,
      versionInfo: releaseSet.version,
      downloadLocation: "NOASSERTION",
      filesAnalyzed: false,
      licenseConcluded: "NOASSERTION",
      licenseDeclared: "NOASSERTION",
      copyrightText: "NOASSERTION",
      checksums: [
        { algorithm: "SHA256", checksumValue: releaseSet.releaseSetId },
      ],
    },
    ...releaseSet.components.map((component) => ({
      name: component.id,
      SPDXID: spdxId(`component-${component.id}`),
      versionInfo: component.version,
      downloadLocation: "NOASSERTION",
      filesAnalyzed: false,
      licenseConcluded: "NOASSERTION",
      licenseDeclared: "NOASSERTION",
      copyrightText: "NOASSERTION",
      checksums: [{ algorithm: "SHA256", checksumValue: component.sha256 }],
    })),
  ];
  const files = releaseSet.artifacts.map((artifact) => ({
    fileName: artifact.fileName,
    SPDXID: spdxId(`artifact-${artifact.id}`),
    checksums: [{ algorithm: "SHA256", checksumValue: artifact.sha256 }],
    licenseConcluded: "NOASSERTION",
    copyrightText: "NOASSERTION",
  }));
  return {
    spdxVersion: contract.sbom.format,
    dataLicense: "CC0-1.0",
    SPDXID: "SPDXRef-DOCUMENT",
    name: `Nimino ${releaseSet.version} release-set`,
    documentNamespace: `https://github.com/${contract.sourceRepository}/releases/sbom/${releaseSet.releaseSetId}`,
    creationInfo: {
      created: "1970-01-01T00:00:00.000Z",
      creators: [
        "Organization: AsoPi Tech",
        "Tool: nimino-release-supply-chain",
      ],
    },
    packages,
    files,
    relationships: [
      {
        spdxElementId: "SPDXRef-DOCUMENT",
        relationshipType: "DESCRIBES",
        relatedSpdxElement: releaseId,
      },
      ...releaseSet.components.map((component) => ({
        spdxElementId: releaseId,
        relationshipType: "DEPENDS_ON",
        relatedSpdxElement: spdxId(`component-${component.id}`),
      })),
      ...releaseSet.artifacts.map((artifact) => ({
        spdxElementId: releaseId,
        relationshipType: "CONTAINS",
        relatedSpdxElement: spdxId(`artifact-${artifact.id}`),
      })),
    ],
  };
}

function sbom(values) {
  required(values, ["release-set", "output"]);
  const releaseSet = JSON.parse(
    readFileSync(resolve(root, values.get("release-set")), "utf8"),
  );
  writeFileSync(
    resolve(root, values.get("output")),
    `${JSON.stringify(sbomFor(releaseSet), null, 2)}\n`,
  );
}

function indexedFiles(directory) {
  return readdirSync(directory)
    .filter(
      (name) =>
        name !== contract.checksums.fileName &&
        name !== contract.checksums.signature &&
        existsSync(join(directory, name)) &&
        statSync(join(directory, name)).isFile(),
    )
    .sort();
}

function checksums(values) {
  required(values, ["directory", "output"]);
  const directory = resolve(root, values.get("directory"));
  const output = resolve(root, values.get("output"));
  if (
    dirname(output) !== directory ||
    basename(output) !== contract.checksums.fileName
  )
    fail("checksum output must be SHA256SUMS inside the indexed directory");
  writeFileSync(
    output,
    indexedFiles(directory)
      .map((name) => `${digest(join(directory, name))}  ${name}\n`)
      .join(""),
  );
}

function verify(values) {
  required(
    values,
    ["release-set", "artifact-dir", "sbom", "checksums", "resolved-tag-commit"],
    ["source-root"],
  );
  const releaseSetPath = resolve(root, values.get("release-set"));
  const artifactDirectory = resolve(root, values.get("artifact-dir"));
  const releaseSet = JSON.parse(readFileSync(releaseSetPath, "utf8"));
  const sourceArguments = values.get("source-root")
    ? ["--source-root", values.get("source-root")]
    : [];
  const verified = spawnSync(
    process.execPath,
    [
      join(root, "scripts/nimino-release-set.mjs"),
      "verify",
      "--manifest",
      releaseSetPath,
      "--resolved-tag-commit",
      values.get("resolved-tag-commit"),
      "--artifact-dir",
      artifactDirectory,
      ...sourceArguments,
    ],
    { cwd: root, encoding: "utf8" },
  );
  if (verified.status !== 0)
    fail(verified.stderr.trim() || "release-set verification failed");
  const expectedSbom = `${JSON.stringify(sbomFor(releaseSet), null, 2)}\n`;
  if (readFileSync(resolve(root, values.get("sbom")), "utf8") !== expectedSbom)
    fail("SBOM does not match the release-set");
  const checksumPath = resolve(root, values.get("checksums"));
  const lines = readFileSync(checksumPath, "utf8").trimEnd().split("\n");
  const entries = new Map();
  for (const line of lines) {
    const match = line.match(
      /^([0-9a-f]{64}) {2}([A-Za-z0-9][A-Za-z0-9._-]*)$/,
    );
    if (!match || entries.has(match[2]))
      fail("checksum inventory is malformed");
    entries.set(match[2], match[1]);
  }
  const expectedFiles = indexedFiles(artifactDirectory);
  if (JSON.stringify([...entries.keys()]) !== JSON.stringify(expectedFiles))
    fail("checksum inventory is incomplete or unordered");
  for (const [name, expected] of entries)
    if (digest(join(artifactDirectory, name)) !== expected)
      fail(`checksum mismatch: ${name}`);
  console.log(releaseSet.releaseSetId);
}

try {
  const { command, values } = args(process.argv.slice(2));
  if (command === "sbom") sbom(values);
  else if (command === "checksums") checksums(values);
  else if (command === "verify") verify(values);
  else
    fail(
      "usage: nimino-release-supply-chain.mjs <sbom|checksums|verify> [options]",
    );
} catch (error) {
  console.error(`error: ${error instanceof Error ? error.message : error}`);
  process.exitCode = 1;
}
