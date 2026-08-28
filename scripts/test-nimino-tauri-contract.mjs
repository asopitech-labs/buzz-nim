#!/usr/bin/env node

import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

const manifest = JSON.parse(
  readFileSync("contracts/nimino-tauri/v1/manifest.json", "utf8"),
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function filesUnder(root) {
  const files = [];
  for (const name of readdirSync(root)) {
    const path = join(root, name);
    if (statSync(path).isDirectory()) files.push(...filesUnder(path));
    else files.push(path);
  }
  return files.sort();
}

function withoutComments(source) {
  return source
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/^\s*\/\/.*$/gm, "");
}

function commandDefinitions(path) {
  const source = withoutComments(readFileSync(path, "utf8"));
  const command =
    /^\s*#\[(?:tauri|command_attr)::command\][\s\S]*?\bfn\s+([A-Za-z0-9_]+)/gm;
  return [...source.matchAll(command)].map((match) => match[1]);
}

function handlerBodies(source) {
  const bodies = [];
  const marker = "generate_handler![";
  for (let offset = 0; ; ) {
    const markerAt = source.indexOf(marker, offset);
    if (markerAt === -1) return bodies;
    const start = markerAt + marker.length - 1;
    let depth = 0;
    let end = start;
    for (; end < source.length; end += 1) {
      if (source[end] === "[") depth += 1;
      if (source[end] === "]") depth -= 1;
      if (depth === 0) break;
    }
    check(depth === 0, "unterminated generate_handler! invocation");
    bodies.push(source.slice(start, end + 1));
    offset = end + 1;
  }
}

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

check(manifest.schemaVersion === 1, "wrong Tauri contract version");
check(
  manifest.contract === "nimino.tauri-adapter-boundary" &&
    manifest.compatibilityMode === false &&
    manifest.publicationState === "cutover-ready",
  "Tauri hard-cut contract drifted",
);
check(
  JSON.stringify(manifest.target.rust) ===
    JSON.stringify(["platform-adapter", "network-adapter", "performance-adapter"]) &&
    manifest.target.nim === "domain-and-product-policy" &&
    manifest.target.typescript === "presentation-and-state-adapter",
  "target responsibility split drifted",
);

const rustFiles = filesUnder("desktop/src-tauri/src").filter((path) =>
  path.endsWith(".rs"),
);
const definitions = rustFiles.flatMap((path) =>
  commandDefinitions(path).map((name) => ({ name, path })),
);
const commandFiles = [...new Set(definitions.map(({ path }) => path))].sort();

const groups = [
  ...Object.entries(manifest.surviving).map(([kind, value]) => ({
    ...value,
    kind,
    ownerIssue: null,
  })),
  ...Object.entries(manifest.cutover).map(([kind, value]) => ({ ...value, kind })),
];
const classifiedFiles = groups.flatMap((group) => group.files);
check(
  new Set(classifiedFiles).size === classifiedFiles.length,
  "a Tauri command file has more than one classification",
);
check(
  JSON.stringify(classifiedFiles.slice().sort()) === JSON.stringify(commandFiles),
  `Tauri command coverage drifted\ndiscovered=${JSON.stringify(commandFiles)}\nclassified=${JSON.stringify(classifiedFiles.slice().sort())}`,
);
check(
  definitions.length === manifest.inventory.commandCount &&
    commandFiles.length === manifest.inventory.sourceFileCount,
  `Tauri command inventory drifted: ${definitions.length} commands in ${commandFiles.length} files`,
);

for (const group of groups) {
  check(group.files.length > 0, `empty Tauri classification: ${group.kind}`);
  check(typeof group.target === "string" && group.target.length > 0, `missing target: ${group.kind}`);
  if (Object.hasOwn(manifest.surviving, group.kind)) {
    check(group.action === "shrink", `surviving ${group.kind} must shrink to an adapter`);
  } else {
    check(["move", "delete"].includes(group.action), `invalid cutover action: ${group.kind}`);
    check(Number.isInteger(group.ownerIssue), `missing cutover owner: ${group.kind}`);
  }
  for (const path of group.files) {
    check(existsSync(path), `classified Tauri path does not exist: ${path}`);
  }
}

const handlers = rustFiles.flatMap((path) =>
  handlerBodies(withoutComments(readFileSync(path, "utf8"))),
);
for (const { name, path } of definitions) {
  const registered = handlers.some((body) =>
    new RegExp(`(?:^|[^A-Za-z0-9_])(?:[A-Za-z0-9_]+::)*${escapeRegex(name)}(?:[^A-Za-z0-9_]|$)`).test(body),
  );
  check(registered, `unregistered Tauri command: ${path}::${name}`);
}

const cargo = readFileSync("desktop/src-tauri/Cargo.toml", "utf8");
for (const dependency of manifest.forbiddenRustPolicyDependencies) {
  const name = escapeRegex(dependency);
  check(
    !new RegExp(`(?:^|\\n)${name}\\s*=|package\\s*=\\s*"${name}"`, "m").test(cargo),
    `forbidden Rust product-policy dependency: ${dependency}`,
  );
}

for (const hotPath of manifest.hotPaths) {
  const source = readFileSync(hotPath.path, "utf8");
  const commands = new Set(commandDefinitions(hotPath.path));
  for (const command of hotPath.commands) {
    check(commands.has(command), `missing hot-path command: ${hotPath.path}::${command}`);
  }
  for (const test of hotPath.tests) {
    check(
      new RegExp(`\\b(?:async\\s+)?fn\\s+${escapeRegex(test)}\\b`).test(source),
      `missing hot-path evidence: ${hotPath.path}::${test}`,
    );
  }
}
check(
  readFileSync("desktop/src-tauri/src/native_websocket.rs", "utf8").includes(
    "BATCH_MAX_SERIALIZED_BYTES",
  ) &&
    readFileSync("desktop/src-tauri/src/observed_unread.rs", "utf8").includes(
      "spawn_blocking",
    ) &&
    readFileSync("desktop/src-tauri/src/terminal_runtime.rs", "utf8").includes(
      "FramePublisher",
    ),
  "hot-path implementation evidence drifted",
);
check(
  manifest.physicalRemovalOwner === 12 &&
    manifest.duplicatePolicyRemovalOwner === 66,
  "Tauri cutover ownership drifted",
);

console.log(
  `Nimino Tauri contract verified: ${definitions.length} commands in ${commandFiles.length} files`,
);
