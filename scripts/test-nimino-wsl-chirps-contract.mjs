#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contract = JSON.parse(
  readFileSync(join(root, "contracts/nimino-wsl-chirps/v1/contract.json"), "utf8"),
);
const chirps = JSON.parse(
  readFileSync(join(root, "contracts/chirps-v0.6.3.json"), "utf8"),
);
const identityTests = readFileSync(
  join(root, "crates/nimino-chirps/tests/identity_lifecycle.rs"),
  "utf8",
);
const runtimeTests = readFileSync(
  join(root, "crates/nimino-chirps/tests/runtime_messaging.rs"),
  "utf8",
);
const upstream = readFileSync(
  join(root, "crates/nimino-chirps/src/upstream.rs"),
  "utf8",
);
const justfile = readFileSync(join(root, "Justfile"), "utf8");
const workflow = readFileSync(join(root, ".github/workflows/ci.yml"), "utf8");
const hooks = readFileSync(join(root, "lefthook.yml"), "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(contract.schemaVersion === 1, "wrong WSL Chirps schema version");
check(contract.contract === "nimino.wsl-chirps-certification", "wrong contract name");
check(contract.issue === 45, "WSL Chirps ownership drifted");
check(contract.supportContract === "wsl-support-v1", "WSL support contract drifted");
check(contract.compatibilityMode === false, "WSL compatibility mode is forbidden");
check(contract.workspacePrefix === "/home/", "WSL certification must use ext4 /home");
check(
  contract.transport === `alopex-chirps/${chirps.upstream.version}-udp-quic-mtls`,
  "WSL transport drifted from the pinned Chirps version",
);
check(
  JSON.stringify(contract.excludedDomains) ===
    JSON.stringify(["database", "replication", "sync", "quorum", "product-policy"]),
  "WSL Chirps certification absorbed a Nimino domain",
);
for (const proof of [
  "stable-node-identity",
  "private-key-mode-0600",
  "trusted-and-untrusted-mtls",
  "certificate-rotation-reload",
  "udp-quic-bind-change-rejoin",
  "shutdown-releases-all-sockets",
]) {
  check(contract.requiredProofs.includes(proof), `missing WSL Chirps proof: ${proof}`);
}
for (const signal of [
  "identity_is_stable_across_prepare_and_certificate_rotation",
  "insecure_rotated_private_key_is_a_typed_failure",
]) {
  check(identityTests.includes(signal), `missing identity proof: ${signal}`);
}
check(
  upstream.includes("mtls_accepts_trusted_rejects_untrusted_and_reloads_rotation"),
  "missing real mTLS trust/rotation proof",
);
for (const signal of [
  "certificate_rotation_and_bind_change_preserve_identity_and_rejoin",
  "rejoined after network change",
  "old anchor socket released",
  "rotated peer socket released",
]) {
  check(runtimeTests.includes(signal), `missing UDP/QUIC recovery proof: ${signal}`);
}
check(
  justfile.includes("wsl-chirps-contract:") &&
    justfile.includes("wsl-chirps-certify:") &&
    workflow.includes("run: just wsl-chirps-contract") &&
    hooks.includes("wsl-chirps-contract:"),
  "WSL Chirps gate is not wired into local and CI policy",
);

if (!process.argv.includes("--certify")) {
  console.log("Nimino WSL Chirps contract passed: mTLS rotation, rebind, rejoin, shutdown");
  process.exit(0);
}

const kernelRelease = os.release();
check(kernelRelease.toLowerCase().includes("microsoft"), "certification requires real WSL2");
const repositoryRoot = realpathSync(root);
check(
  repositoryRoot.startsWith(contract.workspacePrefix),
  `certification workspace must stay under ${contract.workspacePrefix}`,
);
const filesystemType = execFileSync("stat", ["-f", "-c", "%T", repositoryRoot], {
  encoding: "utf8",
}).trim();
check(
  filesystemType === "ext2/ext3" || filesystemType === "ext4",
  `certification requires the WSL ext4 filesystem, got ${filesystemType}`,
);

const temporaryRoot = join(root, "target/nim/wsl-chirps-tmp");
mkdirSync(temporaryRoot, { recursive: true });
try {
  execFileSync(
    "cargo",
    [
      "test",
      "--locked",
      "-p",
      "nimino-chirps",
      "--lib",
      "--test",
      "identity_lifecycle",
      "--test",
      "runtime_messaging",
    ],
    {
      cwd: root,
      env: { ...process.env, TMPDIR: temporaryRoot },
      stdio: "inherit",
    },
  );
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}

const evidencePath = resolve(
  root,
  process.env.NIMINO_WSL_CHIRPS_EVIDENCE ?? contract.evidencePath,
);
mkdirSync(dirname(evidencePath), { recursive: true });
writeFileSync(
  evidencePath,
  `${JSON.stringify(
    {
      schemaVersion: 1,
      contract: contract.contract,
      issue: contract.issue,
      supportContract: contract.supportContract,
      transport: contract.transport,
      compatibilityMode: false,
      kernelRelease,
      filesystemType,
      repositoryRoot,
      rustc: execFileSync("rustc", ["--version"], { encoding: "utf8" }).trim(),
      requiredProofs: contract.requiredProofs,
      result: "passed",
    },
    null,
    2,
  )}\n`,
);
console.log(`Nimino WSL Chirps certification passed: ${evidencePath}`);
