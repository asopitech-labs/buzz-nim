#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contract = JSON.parse(
  readFileSync(join(root, "contracts/chirps-v0.6.3.json"), "utf8"),
);
const wrapperRoot = join(root, "crates", contract.wrapperCrate);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

const skippedRoots = [
  ".codex",
  ".git",
  ".hermit",
  ".spec-workflow",
  "target",
  "node_modules",
  "desktop/dist",
  "desktop/node_modules",
  "desktop/src-tauri/target",
  "web/dist",
  "web/node_modules",
].map((path) => join(root, path));

function isSkipped(path) {
  return skippedRoots.some(
    (skipped) => path === skipped || path.startsWith(`${skipped}/`),
  );
}

function rustFiles(directory) {
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isDirectory() && !isSkipped(path)) {
      files.push(...rustFiles(path));
    } else if (entry.isFile() && path.endsWith(".rs")) files.push(path);
  }
  return files;
}

function cargoMetadata(manifestPath, noDependencies = false) {
  const args = ["metadata", "--locked", "--format-version", "1"];
  if (manifestPath) args.push("--manifest-path", manifestPath);
  if (noDependencies) args.push("--no-deps");
  return JSON.parse(
    execFileSync("cargo", args, {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 32 * 1024 * 1024,
    }),
  );
}

function codeOnly(source) {
  const mask = (start, end) => source.slice(start, end).replace(/[^\n]/g, " ");
  let result = "";
  let index = 0;

  while (index < source.length) {
    if (source.startsWith("//", index)) {
      const newline = source.indexOf("\n", index + 2);
      const end = newline === -1 ? source.length : newline;
      result += mask(index, end);
      index = end;
      continue;
    }
    if (source.startsWith("/*", index)) {
      let depth = 1;
      let end = index + 2;
      while (end < source.length && depth > 0) {
        if (source.startsWith("/*", end)) {
          depth += 1;
          end += 2;
        } else if (source.startsWith("*/", end)) {
          depth -= 1;
          end += 2;
        } else end += 1;
      }
      result += mask(index, end);
      index = end;
      continue;
    }
    if (source[index] === "r") {
      let hashes = 0;
      let quote = index + 1;
      while (source[quote] === "#") {
        hashes += 1;
        quote += 1;
      }
      if (source[quote] === '"') {
        const terminator = `"${"#".repeat(hashes)}`;
        const close = source.indexOf(terminator, quote + 1);
        const end = close === -1 ? source.length : close + terminator.length;
        result += mask(index, end);
        index = end;
        continue;
      }
    }
    if (source[index] === '"') {
      let end = index + 1;
      while (end < source.length) {
        if (source[end] === "\\") end += 2;
        else if (source[end] === '"') {
          end += 1;
          break;
        } else end += 1;
      }
      result += mask(index, end);
      index = end;
      continue;
    }
    result += source[index];
    index += 1;
  }

  return result;
}

const lexerProbe = codeOnly(
  'before /* outer /* nested */ hidden */ after "hidden" r#"hidden"# // hidden\nkept',
);
const delimiterProbe = codeOnly(
  'let open = "/*"; kept_between; let close = "*/"; let url = "http://"; kept_after;',
);
check(
  lexerProbe.includes("before") &&
    lexerProbe.includes("after") &&
    lexerProbe.includes("kept") &&
    !lexerProbe.includes("hidden"),
  "Rust source masking self-check failed",
);
check(
  delimiterProbe.includes("kept_between") &&
    delimiterProbe.includes("kept_after"),
  "string delimiter masking self-check failed",
);

check(existsSync(wrapperRoot), `missing ${contract.wrapperCrate} wrapper crate`);

const metadata = cargoMetadata();
const desktopMetadata = cargoMetadata("desktop/src-tauri/Cargo.toml");
const wrapper = metadata.packages.find(
  (candidate) => candidate.name === contract.wrapperCrate,
);
check(wrapper, `missing ${contract.wrapperCrate} workspace package`);
check(
  Object.keys(wrapper.features).length === 0,
  `wrapper features must stay empty: ${Object.keys(wrapper.features).join(",")}`,
);

const dependency = wrapper.dependencies.find(
  (candidate) => candidate.name === contract.upstream.package,
);
check(dependency, `missing ${contract.upstream.package} direct dependency`);
check(dependency.rename === null, "Chirps dependency must not be aliased");
check(dependency.optional === false, "Chirps dependency must stay required");
check(
  dependency.req === contract.upstream.requirement,
  `expected ${contract.upstream.requirement}, got ${dependency.req}`,
);
check(
  dependency.source === contract.upstream.source,
  `unexpected source ${dependency.source}`,
);
check(
  dependency.uses_default_features === contract.upstream.defaultFeatures,
  "Chirps default features must stay disabled",
);
check(
  JSON.stringify(dependency.features) === JSON.stringify(contract.upstream.features),
  `unexpected Chirps features: ${dependency.features.join(",")}`,
);

const lockEntry = [
  `name = "${contract.upstream.package}"`,
  `version = "${contract.upstream.version}"`,
  `source = "${contract.upstream.source}"`,
  `checksum = "${contract.upstream.checksum}"`,
].join("\n");

function checkResolvedContract(graph, lockPath, label, required) {
  const graphWrapper = graph.packages.find(
    (candidate) => candidate.name === contract.wrapperCrate,
  );
  if (!graphWrapper) {
    check(!required, `missing ${contract.wrapperCrate} in ${label}`);
    return false;
  }
  check(
    Object.keys(graphWrapper.features).length === 0,
    `${label} wrapper features must stay empty`,
  );
  const resolved = graph.packages.find(
    (candidate) =>
      candidate.name === contract.upstream.package &&
      candidate.version === contract.upstream.version,
  );
  check(
    resolved,
    `missing resolved ${contract.upstream.package} ${contract.upstream.version} in ${label}`,
  );
  check(resolved.source === contract.upstream.source, `${label} source drifted`);
  const resolvedNode = graph.resolve.nodes.find(
    (candidate) => candidate.id === resolved.id,
  );
  check(resolvedNode, `missing resolved Chirps dependency node in ${label}`);
  check(
    JSON.stringify(resolvedNode.features) ===
      JSON.stringify(contract.upstream.features),
    `${label} resolved Chirps features drifted: ${resolvedNode.features.join(",")}`,
  );
  check(
    readFileSync(lockPath, "utf8").includes(lockEntry),
    `${label} Cargo.lock Chirps checksum drifted`,
  );
  return true;
}

checkResolvedContract(metadata, join(root, "Cargo.lock"), "root", true);
const desktopUsesWrapper = checkResolvedContract(
  desktopMetadata,
  join(root, "desktop", "src-tauri", "Cargo.lock"),
  "desktop",
  false,
);

const workspaceMembers = new Set(metadata.workspace_members);
const desktopWorkspaceMembers = new Set(desktopMetadata.workspace_members);
const workspacePackages = [
  ...metadata.packages.filter((candidate) => workspaceMembers.has(candidate.id)),
  ...desktopMetadata.packages.filter((candidate) =>
    desktopWorkspaceMembers.has(candidate.id),
  ),
];
const directUsers = workspacePackages
  .filter((candidate) =>
    candidate.dependencies.some(
      (dep) => dep.name === contract.upstream.package,
    ),
  )
  .map((candidate) => candidate.name);
check(
  JSON.stringify(directUsers) === JSON.stringify([contract.wrapperCrate]),
  `direct Chirps users must be only ${contract.wrapperCrate}: ${directUsers.join(",")}`,
);
for (const forbiddenPackage of contract.forbiddenDirectPackages) {
  const users = workspacePackages
    .filter((candidate) =>
      candidate.dependencies.some((dep) => dep.name === forbiddenPackage),
    )
    .map((candidate) => candidate.name);
  check(
    users.length === 0,
    `forbidden direct package ${forbiddenPackage}: ${users.join(",")}`,
  );
}

const wrapperFiles = rustFiles(join(wrapperRoot, "src"));
const adapterPath = join(wrapperRoot, "src", "upstream.rs");
check(wrapperFiles.includes(adapterPath), "missing private Chirps adapter module");
const adapterCode = codeOnly(readFileSync(adapterPath, "utf8"));
const libCode = codeOnly(
  readFileSync(join(wrapperRoot, "src", "lib.rs"), "utf8"),
);
const publicCode = codeOnly(
  wrapperFiles
    .filter((path) => path !== adapterPath)
    .map((path) => readFileSync(path, "utf8"))
    .join("\n"),
);
const wrapperCode = `${adapterCode}\n${publicCode}`;
const identity = contract.productionIdentity;
check(
  identity.mtlsRequired === true &&
    JSON.stringify(identity.identityByteLengths) === JSON.stringify([16, 24]) &&
    identity.unixSecretMode === "0600" &&
    identity.runtimeMessagingOwner === 43 &&
    identity.admissionPolicyOwner === 48,
  "production identity ownership contract drifted",
);
check(
  JSON.stringify(contract.publicIdentityApi) ===
    JSON.stringify(["NodeConfig", "NodeConfigError", "NodeId", "NodeIdentity"]),
  "public identity API contract drifted",
);
for (const type of contract.publicIdentityApi) {
  check(
    new RegExp(`\\bpub\\s+(?:struct|enum)\\s+${type}\\b`).test(publicCode),
    `missing public identity API: ${type}`,
  );
}
for (const error of identity.typedErrors) {
  check(new RegExp(`\\b${error}\\b`).test(publicCode), `missing typed identity error: ${error}`);
}
check(
  !publicCode.includes("alopex_chirps"),
  "raw Chirps types must stay in the private adapter module",
);
check(
  !/\bpub(?:\([^)]*\))?\s+use\b[^;]*\bupstream\b/.test(publicCode),
  "private Chirps adapter must not be re-exported",
);
const upstreamDeclarations = [
  ...libCode.matchAll(
    /\b(?:(pub(?:\([^)]*\))?)\s+)?mod\s+upstream\s*;/g,
  ),
];
check(
  upstreamDeclarations.length === 1 && !upstreamDeclarations[0][1],
  "Chirps adapter module must be declared exactly private",
);
const visibilityTokens = [...adapterCode.matchAll(/\bpub\b/g)];
const allowedVisibility = [
  ...adapterCode.matchAll(/\bpub\(crate\)\s+fn\b/g),
];
check(
  visibilityTokens.length === allowedVisibility.length,
  "Chirps adapter visibility must be exactly pub(crate) fn",
);
const chirpsTokens = [...adapterCode.matchAll(/\balopex_chirps\b/g)];
const chirpsReferences = [
  ...adapterCode.matchAll(/\balopex_chirps::([A-Za-z][A-Za-z0-9_]*)/g),
];
check(
  chirpsTokens.length === chirpsReferences.length,
  "Chirps imports must name one top-level symbol",
);
for (const reference of chirpsReferences) {
  const symbol = reference[1];
  check(
    contract.allowedTopLevelApi.includes(symbol),
    `Chirps API is not allowlisted: ${symbol}`,
  );
}
for (const method of contract.forbiddenMethods) {
  const escaped = method.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(`(?:\\.|::)\\s*(?:r#)?${escaped}\\b`);
  check(pattern.test(`MeshHandle::${method}`), "forbidden method self-check failed");
  check(
    pattern.test(`handle.r#${method}`),
    "raw forbidden method self-check failed",
  );
  check(
    !pattern.test(wrapperCode),
    `forbidden Chirps method in wrapper: ${method}`,
  );
}
for (const variant of contract.forbiddenVariants) {
  const escaped = variant.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const pattern = new RegExp(`\\b(?:r#)?${escaped}\\b`);
  check(pattern.test(`use Frame::{${variant}};`), "forbidden variant self-check failed");
  check(
    pattern.test(`use Frame::{r#${variant}};`),
    "raw forbidden variant self-check failed",
  );
  check(
    !pattern.test(wrapperCode),
    `forbidden Chirps variant in wrapper: ${variant}`,
  );
}

for (const path of rustFiles(root)) {
  if (path.startsWith(`${wrapperRoot}/`)) continue;
  check(
    !codeOnly(readFileSync(path, "utf8")).includes("alopex_chirps"),
    `direct Chirps import outside wrapper: ${relative(root, path)}`,
  );
}

function checkFeatureTree(manifestPath) {
  const args = ["tree", "-p", contract.wrapperCrate, "-e", "features", "--locked"];
  if (manifestPath) args.push("--manifest-path", manifestPath);
  const tree = execFileSync("cargo", args, { cwd: root, encoding: "utf8" });
  check(
    !tree.includes(`${contract.upstream.package} feature`),
    "alopex-chirps feature unexpectedly enabled",
  );
}

checkFeatureTree();
if (desktopUsesWrapper) checkFeatureTree("desktop/src-tauri/Cargo.toml");

console.log(
  `Chirps contract passed: ${contract.upstream.package} ${contract.upstream.requirement}, explicit production mTLS identity`,
);
