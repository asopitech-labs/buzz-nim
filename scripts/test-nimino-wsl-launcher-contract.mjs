#!/usr/bin/env node

import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";

const contract = JSON.parse(
  readFileSync("contracts/nimino-wsl-launcher/v1/contract.json", "utf8"),
);
assert.deepEqual(Object.keys(contract).sort(), [
  "compatibilityMode",
  "contract",
  "distribution",
  "forbiddenPathPrefixes",
  "issue",
  "launcher",
  "livePathPrefixes",
  "plaintextLegacyAction",
  "platformContract",
  "processIdentity",
  "schemaVersion",
  "secretBackend",
  "secretForbiddenCarriers",
  "secretHandoff",
  "shellInterpolation",
]);
assert.equal(contract.contract, "nimino.wsl-launcher");
assert.equal(contract.distribution, "Ubuntu-24.04");
assert.equal(contract.shellInterpolation, false);
assert.deepEqual(contract.processIdentity, [
  "distribution",
  "user",
  "hostPid",
  "linuxPid",
]);
assert.equal(contract.secretHandoff, "length-prefixed-stdin");
assert.deepEqual(contract.secretForbiddenCarriers, [
  "argv",
  "windows-environment",
  "file",
]);
assert.equal(contract.plaintextLegacyAction, "delete-without-read");
assert.equal(contract.compatibilityMode, false);

const launcher = "crates/nimino-wsl-launcher/src/lib.rs";
assert(existsSync(launcher), "typed WSL launcher adapter is missing");
const launcherSource = readFileSync(launcher, "utf8");
for (const forbidden of ["sh -c", "bash -c", "cmd /C", "powershell -Command"]) {
  assert(!launcherSource.includes(forbidden), `shell interpolation escaped: ${forbidden}`);
}
for (const signal of [
  'arg("--distribution")',
  'arg("--user")',
  'arg("--exec")',
  "stdin(Stdio::piped())",
  "host_pid",
  "linux_pid",
]) {
  assert(launcherSource.includes(signal), `launcher signal missing: ${signal}`);
}

const secretSources = [
  "desktop/src-tauri/src/app_state.rs",
  "desktop/src-tauri/src/commands/identity.rs",
  "desktop/src-tauri/src/commands/pairing.rs",
  "desktop/src-tauri/src/identity_storage.rs",
  "desktop/src-tauri/src/secret_store.rs",
].map((path) => [path, readFileSync(path, "utf8")]);
for (const [path, source] of secretSources) {
  for (const forbidden of [
    "save_key_file",
    "load_key_file",
    "IdentityStorage::LocalFile",
    "falling back to identity.key",
    "fall back to their own `0o600` file storage",
  ]) {
    assert(!source.includes(forbidden), `${path} retains plaintext fallback: ${forbidden}`);
  }
}
const appState = secretSources[0][1];
assert.equal(
  [...appState.matchAll(/identity\.key/g)].length,
  2,
  "identity.key may appear only in deletion-only function and its diagnostic",
);
assert(appState.includes("remove_plaintext_identity"));
assert(appState.includes("std::fs::remove_file"));

console.log("Nimino WSL launcher contract passed: typed argv, PID ownership, fail-closed secret");
