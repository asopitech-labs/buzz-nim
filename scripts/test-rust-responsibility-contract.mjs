#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(
  readFileSync(
    join(root, "contracts/rust-responsibility/v1/manifest.json"),
    "utf8",
  ),
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function sorted(values) {
  return [...values].sort();
}

function same(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function cargoMetadata(manifestPath) {
  const args = ["metadata", "--locked", "--no-deps", "--format-version", "1"];
  if (manifestPath) args.push("--manifest-path", manifestPath);
  return JSON.parse(
    execFileSync("cargo", args, {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 32 * 1024 * 1024,
    }),
  );
}

function rustFiles(directory) {
  const files = [];
  const skipped = new Set([
    ".codex",
    ".git",
    ".hermit",
    ".spec-workflow",
    "node_modules",
    "target",
  ]);
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isDirectory() && skipped.has(entry.name)) continue;
    const path = join(directory, entry.name);
    if (entry.isDirectory()) files.push(...rustFiles(path));
    else if (entry.isFile() && path.endsWith(".rs")) files.push(path);
  }
  return files;
}

check(
  manifest.schemaVersion === 1 &&
    manifest.contract === "nimino.rust-responsibility" &&
    manifest.compatibilityMode === false,
  "Rust responsibility hard-cut contract drifted",
);
check(
  same(Object.keys(manifest.actions).sort(), [
    "delete",
    "keep",
    "move",
    "replace",
    "shrink",
  ]),
  "Rust action inventory must be keep/shrink/move/replace/delete",
);
check(
  manifest.classificationUnit.dependency === "cargo-package" &&
    manifest.classificationUnit.source === "nearest-cargo-package" &&
    manifest.classificationUnit.tauriCommand ===
      "contracts/nimino-tauri/v1/manifest.json",
  "Rust classification units drifted",
);

const metadata = [
  cargoMetadata(),
  cargoMetadata("desktop/src-tauri/Cargo.toml"),
];
const packages = new Map();
for (const item of metadata.flatMap((value) => value.packages)) {
  check(!packages.has(item.name), `duplicate Cargo package name: ${item.name}`);
  packages.set(item.name, item);
}

const classified = new Map();
for (const [action, group] of Object.entries(manifest.actions)) {
  check(
    Object.keys(group).sort().join(",") === "packages,target" &&
      typeof group.target === "string" &&
      Array.isArray(group.packages),
    `invalid Rust action group: ${action}`,
  );
  for (const name of group.packages) {
    check(!classified.has(name), `Cargo package has multiple actions: ${name}`);
    classified.set(name, action);
  }
}
check(
  same(sorted(classified.keys()), sorted(packages.keys())) &&
    classified.size === manifest.inventory.packageCount,
  `Cargo package classification drifted: discovered=${packages.size}, classified=${classified.size}`,
);

const packageRoots = [...packages.values()]
  .map((item) => ({ name: item.name, path: dirname(item.manifest_path) }))
  .sort((left, right) => right.path.length - left.path.length);
const sources = rustFiles(root);
const standaloneSources = new Map(
  manifest.standaloneSources.map((item) => [join(root, item.path), item]),
);
check(
  standaloneSources.size === manifest.standaloneSources.length,
  "duplicate standalone Rust source classification",
);
for (const source of sources) {
  const owner = packageRoots.find(
    (candidate) =>
      source === candidate.path || source.startsWith(`${candidate.path}${sep}`),
  );
  if (owner) {
    check(
      classified.has(owner.name),
      `Rust source owner is unclassified: ${owner.name}`,
    );
    continue;
  }
  const standalone = standaloneSources.get(source);
  check(
    standalone,
    `Rust source has no classification: ${relative(root, source)}`,
  );
  check(
    standalone.action === "delete" && standalone.removalIssue === 66,
    `standalone Rust source must be removed by #66: ${standalone.path}`,
  );
}
check(
  [...standaloneSources.keys()].every((path) => sources.includes(path)),
  "standalone Rust source inventory contains a missing path",
);

const cutoverEdges = new Map(
  manifest.cutoverDependencyEdges.map((edge) => [
    `${edge.from}->${edge.to}`,
    edge,
  ]),
);
const observedCutoverEdges = new Set();
for (const [name, item] of packages) {
  const dependencies = new Set(
    item.dependencies
      .filter((dependency) => dependency.path && packages.has(dependency.name))
      .map((dependency) => dependency.name),
  );
  for (const dependency of dependencies) {
    const dependencyAction = classified.get(dependency);
    const edgeName = `${name}->${dependency}`;
    if (["move", "replace", "delete"].includes(dependencyAction)) {
      const edge = cutoverEdges.get(edgeName);
      check(
        edge?.removalIssue === 66,
        `untracked dependency on cutover package: ${edgeName}`,
      );
      observedCutoverEdges.add(edgeName);
    }
    if (classified.get(name) === "keep") {
      check(
        dependencyAction === "keep",
        `kept package depends upward: ${edgeName}`,
      );
    }
    if (name.startsWith("nimino-")) {
      check(
        !dependency.startsWith("buzz-"),
        `Nimino adapter depends on legacy Buzz: ${edgeName}`,
      );
    }
  }
}
check(
  same(sorted(observedCutoverEdges), sorted(cutoverEdges.keys())),
  "cutover dependency edge inventory drifted",
);

const policy = manifest.policyOwnership;
const domainRoot = join(root, policy.nimDomainRoot);
const domainModules = readdirSync(domainRoot)
  .filter(
    (name) =>
      name.endsWith(".nim") && statSync(join(domainRoot, name)).isFile(),
  )
  .sort();
check(
  same(domainModules, sorted(policy.nimDomainModules)),
  "Nim domain owner inventory drifted",
);
check(
  Array.isArray(policy.targetRustPolicyPackages) &&
    policy.targetRustPolicyPackages.length === 0,
  "target must not contain a Rust product-policy package",
);

const requestSchema = readFileSync(
  join(root, "contracts/nim-rust-boundary/v1/request.schema.json"),
  "utf8",
);
const schemaOperations = sorted(
  new Set(
    [...requestSchema.matchAll(/"const": "(domain\.[^"]+\.policy)"/g)].map(
      (match) => match[1],
    ),
  ),
);
check(
  same(schemaOperations, sorted(policy.operations)),
  "Nim policy operation inventory drifted",
);
for (const source of sources) {
  const owner = packageRoots.find((candidate) =>
    source.startsWith(`${candidate.path}${sep}`),
  );
  if (owner?.name === policy.rustOperationLiteralOwner) continue;
  const text = readFileSync(source, "utf8");
  for (const operation of policy.operations) {
    check(
      !text.includes(operation),
      `Rust policy operation escaped boundary: ${relative(root, source)}`,
    );
  }
}

const tauri = JSON.parse(
  readFileSync(join(root, manifest.classificationUnit.tauriCommand), "utf8"),
);
check(
  tauri.contract === "nimino.tauri-adapter-boundary" &&
    tauri.compatibilityMode === false &&
    tauri.physicalRemovalOwner === 12,
  "Tauri command responsibility contract drifted",
);

const workflow = readFileSync(join(root, ".github/workflows/ci.yml"), "utf8");
const nimJob =
  workflow.match(/^ {2}nim:\n([\s\S]*?)(?=^ {2}[a-z0-9-]+:\n)/m)?.[0] ?? "";
const rustJob =
  workflow.match(/^ {2}rust-lint:\n([\s\S]*?)(?=^ {2}[a-z0-9-]+:\n)/m)?.[0] ??
  "";
const laneTest = readFileSync(join(root, "scripts/test-ci-lanes.mjs"), "utf8");
check(
  same(manifest.developerExperience.baselinePolicyLanguages, ["nim", "rust"]) &&
    same(manifest.developerExperience.targetPolicyLanguages, ["nim"]) &&
    manifest.developerExperience.pureNimRustJobs === 0 &&
    same(manifest.developerExperience.boundaryRustPackages, [
      "nimino-boundary",
    ]) &&
    manifest.developerExperience.fullRustWorkspaceForPolicyOnlyChange === false,
  "developer-experience target drifted",
);
check(!/\bcargo\b/.test(nimJob), "pure Nim CI job must not compile Rust");
check(
  !rustJob.includes("needs.changes.outputs.nim"),
  "Nim changes must not select workspace Rust lint",
);
check(
  laneTest.includes("Nim domain-only change must stay Rust-free"),
  "missing Nim-only CI lane proof",
);
check(
  manifest.physicalRemovalOwner === 66,
  "physical cleanup must remain owned by #66",
);

console.log(
  `Rust responsibility contract passed: ${packages.size} packages, ${sources.length} source modules, ${policy.operations.length} Nim policy operations`,
);
