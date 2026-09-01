#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import path from "node:path";

const binary = process.argv[2];
assert(binary, "usage: test-nimino-mcp-framing.mjs <dev-mcp-binary>");

const child = spawn(path.resolve(binary), [], {
  cwd: process.cwd(),
  env: { ...process.env, NIMINO_MCP_CAPABILITIES: "" },
  stdio: ["pipe", "pipe", "pipe"],
});
const pending = new Map();
let nextId = 1;
let stdout = "";
let stderr = "";

child.stderr.setEncoding("utf8");
child.stderr.on("data", (chunk) => {
  stderr += chunk;
});
child.stdout.setEncoding("utf8");
child.stdout.on("data", (chunk) => {
  stdout += chunk;
  for (;;) {
    const newline = stdout.indexOf("\n");
    if (newline < 0) break;
    const line = stdout.slice(0, newline).trim();
    stdout = stdout.slice(newline + 1);
    if (!line) continue;
    const message = JSON.parse(line);
    pending.get(message.id)?.(message);
    pending.delete(message.id);
  }
});

function request(method, params) {
  const id = nextId++;
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(
      () => {
        pending.delete(id);
        reject(new Error(`MCP response timeout: ${method}`));
      },
      10_000,
    );
    pending.set(id, (message) => {
      clearTimeout(timer);
      resolve(message);
    });
  });
}

try {
  const initialized = await request("initialize", {
    protocolVersion: "2024-11-05",
    capabilities: {},
    clientInfo: { name: "nimino-mcp-framing-test", version: "1" },
  });
  assert.equal(initialized.result.serverInfo.name, "nimino-dev-mcp");
  child.stdin.write(
    `${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`,
  );

  const listed = await request("tools/list", {});
  assert(listed.result.tools.some((tool) => tool.name === "shell"));

  const denied = await request("tools/call", {
    name: "shell",
    arguments: { command: "echo must-not-run" },
  });
  assert.match(denied.error.message, /CAPABILITY_DENIED/);
  assert.equal(denied.error.data.contract, "nimino.mcp-capability-audit/v1");
  assert.equal(denied.error.data.capability, "process.exec");
  assert.match(stderr, /nimino\.mcp-capability-audit\/v1/);
} finally {
  child.stdin.end();
  await new Promise((resolve) => {
    const timer = setTimeout(() => {
      child.kill();
      resolve();
    }, 2_000);
    child.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });
}

console.log("nimino MCP framing and capability denial contract: ok");
