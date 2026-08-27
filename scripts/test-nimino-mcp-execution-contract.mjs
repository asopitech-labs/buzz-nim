#!/usr/bin/env node

import { readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-mcp-execution/v1/contract.json", "utf8"),
);
const capability = readFileSync("crates/buzz-dev-mcp/src/capability.rs", "utf8");
const shell = readFileSync("crates/buzz-dev-mcp/src/shell.rs", "utf8");
const readFile = readFileSync("crates/buzz-dev-mcp/src/read_file.rs", "utf8");
const replace = readFileSync("crates/buzz-dev-mcp/src/str_replace.rs", "utf8");
const image = readFileSync("crates/buzz-dev-mcp/src/view_image.rs", "utf8");
const output = readFileSync("crates/buzz-dev-mcp/src/output.rs", "utf8");
const agent = readFileSync("crates/buzz-agent/src/mcp.rs", "utf8");
const framing = readFileSync("scripts/test-nimino-mcp-framing.mjs", "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function rustInteger(source, name) {
  const value = source.match(new RegExp(`const ${name}: [^=]+ = ([\\d_]+)`))?.[1];
  return value ? Number(value.replaceAll("_", "")) : undefined;
}

check(contract.schemaVersion === 1 && contract.version === 1, "wrong MCP contract version");
check(contract.contract === "nimino.mcp-execution", "wrong MCP contract name");
check(contract.compatibilityMode === false, "MCP compatibility mode is forbidden");
check(contract.owner === "rust-adapter", "MCP I/O must remain a Rust adapter");
check(
  JSON.stringify(contract.capabilities) ===
    JSON.stringify([
      "process.exec",
      "filesystem.read",
      "filesystem.write",
      "network.read",
    ]),
  "MCP capability set drifted",
);
for (const value of contract.capabilities) {
  check(capability.includes(`"${value}"`), `missing capability implementation: ${value}`);
}
check(
  capability.includes(contract.configuration.environment) &&
    agent.includes(`"${contract.configuration.environment}"`) &&
    capability.includes(contract.audit.contract),
  "MCP configuration passthrough or audit contract drifted",
);
check(
  shell.indexOf('authorize("shell"') < shell.indexOf("cmd.spawn()") &&
    replace.indexOf('authorize(\n        "str_replace"') < replace.indexOf("atomic_write") &&
    image.indexOf('authorize(\n            "view_image"') < image.indexOf("fetch_url(src).await"),
  "capability checks must precede external effects",
);
check(
  rustInteger(shell, "DEFAULT_TIMEOUT_MS") === contract.limits.defaultTimeoutMs &&
    rustInteger(shell, "MAX_TIMEOUT_MS") === contract.limits.maxTimeoutMs &&
    shell.includes("ct.cancelled()") &&
    shell.includes("kill_immediate()") &&
    shell.includes("child.wait()"),
  "shell timeout/cancellation contract drifted",
);
check(
  output.includes("MAX_TEXT_RESULT_BYTES: usize = 256 * 1024") &&
    readFile.includes("output::bounded_text(out)") &&
    replace.includes("output::bounded_text(format!("),
  "filesystem output limit is not enforced",
);
for (const test of [
  "missing_process_capability_rejects_before_spawn",
  "cancellation_stops_running_command",
  "large_output_is_truncated_and_saved",
  "missing_read_capability_rejects_access",
  "read_output_is_bounded",
  "missing_write_capability_leaves_file_unchanged",
  "missing_network_capability_rejects_before_fetch",
]) {
  check(`${shell}\n${readFile}\n${replace}\n${image}`.includes(test), `missing MCP test: ${test}`);
}
check(
  framing.includes('request("initialize"') &&
    framing.includes('request("tools/list"') &&
    framing.includes('request("tools/call"'),
  "real MCP stdio framing test drifted",
);
check(contract.compositionOwner === 42 && contract.cutoverOwner === 12, "wrong MCP lifecycle owner");

console.log("Nimino MCP execution contract verified");
