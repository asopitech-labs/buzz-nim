#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
contract_dir="$repo_root/contracts/nim-rust-boundary/v1"
rust_contract="$repo_root/crates/nimino-boundary/src/contract.rs"
nim_contract="$repo_root/nim/nimino_core/src/nimino_core/boundary/protocol.nim"

expected_bundle="$(sed -n 's/^# bundle-sha256: //p' "$contract_dir/schema.sha256")"
node - "$contract_dir" <<'NODE'
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const root = process.argv[2];
const repoRoot = path.resolve(root, "../../..");
const names = [
  "errors.json",
  "lifecycle.json",
  "request.schema.json",
  "response.schema.json",
];
const lines = [];
for (const name of names) {
  const bytes = fs.readFileSync(path.join(root, name));
  JSON.parse(bytes.toString("utf8"));
  const digest = crypto.createHash("sha256").update(bytes).digest("hex");
  lines.push(`${digest}  ${name}`);
  process.stdout.write(`${name}: OK\n`);
}
const manifest = fs.readFileSync(path.join(root, "schema.sha256"), "utf8");
const expectedLines = manifest
  .split(/\r?\n/)
  .filter((line) => line && !line.startsWith("#"));
if (expectedLines.join("\n") !== lines.join("\n")) {
  throw new Error("boundary schema file checksum mismatch");
}
const expectedBundle = manifest.match(/^# bundle-sha256: ([0-9a-f]{64})$/m)?.[1];
const actualBundle = crypto
  .createHash("sha256")
  .update(`${lines.join("\n")}\n`)
  .digest("hex");
if (expectedBundle !== actualBundle) {
  throw new Error("boundary schema bundle hash mismatch");
}

function valid(schema, value, root = schema) {
  if (schema.$ref) {
    const target = schema.$ref
      .slice(2)
      .split("/")
      .map((part) => part.replaceAll("~1", "/").replaceAll("~0", "~"))
      .reduce((node, part) => node?.[part], root);
    return target !== undefined && valid(target, value, root);
  }
  if (schema.oneOf) {
    if (schema.oneOf.filter((branch) => valid(branch, value, root)).length !== 1) return false;
  }
  if (Object.hasOwn(schema, "const") && JSON.stringify(value) !== JSON.stringify(schema.const)) {
    return false;
  }
  if (schema.enum && !schema.enum.some((item) => JSON.stringify(item) === JSON.stringify(value))) {
    return false;
  }
  if (schema.type === "object" || schema.properties) {
    if (value === null || Array.isArray(value) || typeof value !== "object") return false;
    if (schema.required?.some((key) => !Object.hasOwn(value, key))) return false;
    if (schema.additionalProperties === false && Object.keys(value).some((key) => !Object.hasOwn(schema.properties ?? {}, key))) return false;
    return Object.entries(schema.properties ?? {}).every(([key, child]) => !Object.hasOwn(value, key) || valid(child, value[key], root));
  }
  if (schema.type === "array") {
    if (!Array.isArray(value)) return false;
    if (schema.items && !value.every((item) => valid(schema.items, item, root))) return false;
    return !schema.contains || value.some((item) => valid(schema.contains, item, root));
  }
  if (schema.type === "null") return value === null;
  if (schema.type === "boolean" && typeof value !== "boolean") return false;
  if (schema.type === "string" && typeof value !== "string") return false;
  if (schema.type === "integer" && !Number.isInteger(value)) return false;
  if (typeof value === "number") {
    if (schema.minimum !== undefined && value < schema.minimum) return false;
    if (schema.maximum !== undefined && value > schema.maximum) return false;
  }
  if (typeof value === "string") {
    if (schema.minLength !== undefined && value.length < schema.minLength) return false;
    if (schema.maxLength !== undefined && value.length > schema.maxLength) return false;
    if (schema.pattern && !(new RegExp(schema.pattern)).test(value)) return false;
  }
  return true;
}

for (const [schemaName, fixtureName] of [
  ["request.schema.json", "echo.request.json"],
  ["request.schema.json", "event-policy.request.json"],
  ["request.schema.json", "community-policy.request.json"],
  ["request.schema.json", "membership-policy.request.json"],
  ["request.schema.json", "dm-policy.request.json"],
  ["request.schema.json", "moderation-policy.request.json"],
  ["request.schema.json", "workflow-policy.request.json"],
  ["request.schema.json", "cli-policy.request.json"],
  ["request.schema.json", "agent-policy.request.json"],
  ["request.schema.json", "cluster-lifecycle.request.json"],
    ["request.schema.json", "admission-policy.request.json"],
    ["request.schema.json", "authorization-invalidation-policy.request.json"],
    ["request.schema.json", "ephemeral-policy.request.json"],
  ["request.schema.json", "control-policy.request.json"],
  ["request.schema.json", "lease-policy.request.json"],
  ["request.schema.json", "effect-policy.request.json"],
  ["request.schema.json", "object-policy.request.json"],
  ["request.schema.json", "projection-policy.request.json"],
  ["request.schema.json", "sync-policy.request.json"],
  ["response.schema.json", "echo.response.json"],
  ["response.schema.json", "event-policy.response.json"],
  ["response.schema.json", "community-policy.response.json"],
  ["response.schema.json", "membership-policy.response.json"],
  ["response.schema.json", "dm-policy.response.json"],
  ["response.schema.json", "moderation-policy.response.json"],
  ["response.schema.json", "workflow-policy.response.json"],
  ["response.schema.json", "cli-policy.response.json"],
  ["response.schema.json", "agent-policy.response.json"],
  ["response.schema.json", "cluster-lifecycle.response.json"],
    ["response.schema.json", "admission-policy.response.json"],
    ["response.schema.json", "authorization-invalidation-policy.response.json"],
    ["response.schema.json", "ephemeral-policy.response.json"],
  ["response.schema.json", "control-policy.response.json"],
  ["response.schema.json", "lease-policy.response.json"],
  ["response.schema.json", "effect-policy.response.json"],
  ["response.schema.json", "object-policy.response.json"],
  ["response.schema.json", "projection-policy.response.json"],
  ["response.schema.json", "sync-policy.response.json"],
  ["response.schema.json", "unknown-operation.response.json"],
]) {
  const schema = JSON.parse(fs.readFileSync(path.join(root, schemaName), "utf8"));
  const fixture = JSON.parse(fs.readFileSync(path.join(root, "fixtures", fixtureName), "utf8"));
  if (!valid(schema, fixture)) throw new Error(`${fixtureName} does not match ${schemaName}`);
  process.stdout.write(`${fixtureName}: schema OK\n`);
}

const manifestText = fs.readFileSync(path.join(repoRoot, "crates/nimino-boundary/Cargo.toml"), "utf8");
const dependencyBlock = manifestText.match(/^\[dependencies\]\n([\s\S]*?)(?=^\[|\z)/m)?.[1] ?? "";
const allowedDependencies = new Set(["serde", "serde_json", "thiserror", "tokio", "tokio-util", "uuid"]);
for (const line of dependencyBlock.split(/\r?\n/)) {
  const dependency = line.match(/^([A-Za-z0-9_-]+)\s*=/)?.[1];
  if (dependency && !allowedDependencies.has(dependency)) {
    throw new Error(`boundary adapter dependency is not allowed: ${dependency}`);
  }
}
NODE

grep -Fq "$expected_bundle" "$rust_contract"
grep -Fq "$expected_bundle" "$nim_contract"
grep -Fq 'nimino.core.boundary' "$rust_contract"
grep -Fq 'nimino.core.boundary' "$nim_contract"

if grep -REn --include='*.rs' --exclude-dir=bin \
  '^[[:space:]]*(use|extern crate)[[:space:]]+(nimino_|sqlx|redis|nostr|iroh|chirps)' \
  "$repo_root/crates/nimino-boundary/src"; then
  echo "boundary adapter imports a forbidden domain/storage/cluster owner" >&2
  exit 1
fi

if grep -REni --include='*.rs' 'fallback|legacy|dual.?runtime' \
  "$repo_root/crates/nimino-boundary/src"; then
  echo "boundary adapter contains a compatibility path" >&2
  exit 1
fi

if grep -En 'pub fn new\(' "$rust_contract"; then
  echo "BoundaryRequest must expose typed constructors, not a generic operation/value constructor" >&2
  exit 1
fi

if grep -En 'println!|print!' \
  "$repo_root/crates/nimino-boundary/src/lib.rs" \
  "$repo_root/crates/nimino-boundary/src/agent.rs" \
  "$repo_root/crates/nimino-boundary/src/admission.rs" \
  "$repo_root/crates/nimino-boundary/src/ephemeral.rs" \
  "$repo_root/crates/nimino-boundary/src/cli.rs" \
  "$repo_root/crates/nimino-boundary/src/codec.rs" \
  "$repo_root/crates/nimino-boundary/src/community.rs" \
  "$repo_root/crates/nimino-boundary/src/control.rs" \
  "$repo_root/crates/nimino-boundary/src/contract.rs" \
  "$repo_root/crates/nimino-boundary/src/dm.rs" \
  "$repo_root/crates/nimino-boundary/src/moderation.rs" \
  "$repo_root/crates/nimino-boundary/src/error.rs" \
  "$repo_root/crates/nimino-boundary/src/lease.rs" \
  "$repo_root/crates/nimino-boundary/src/membership.rs" \
  "$repo_root/crates/nimino-boundary/src/runtime.rs" \
  "$repo_root/crates/nimino-boundary/src/sync.rs" \
  "$repo_root/crates/nimino-boundary/src/workflow.rs"; then
  echo "library code must not write to the worker protocol stream" >&2
  exit 1
fi

echo "Nim/Rust boundary contract verified"
