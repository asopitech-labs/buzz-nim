#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import os from "node:os";
import { dirname, join, posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-wsl-bundle/v1/contract.json"),
    "utf8",
  ),
);
const support = JSON.parse(
  readFileSync(join(root, "contracts/wsl-support-v1.json"), "utf8"),
);
const releaseSet = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-release-set/v1/contract.json"),
    "utf8",
  ),
);
const agentBundle = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-agent-bundle/v1/contract.json"),
    "utf8",
  ),
);
const service = readFileSync(
  join(root, "scripts/nimino-wsl-service.sh"),
  "utf8",
);
const composer = readFileSync(
  join(root, "scripts/nimino-wsl-bundle.mjs"),
  "utf8",
);
const workflow = readFileSync(join(root, ".github/workflows/ci.yml"), "utf8");
const hooks = readFileSync(join(root, "lefthook.yml"), "utf8");
const justfile = readFileSync(join(root, "Justfile"), "utf8");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function same(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

check(contract.schemaVersion === 1, "wrong WSL bundle schema version");
check(contract.contract === "nimino.wsl-bundle", "wrong WSL bundle contract");
check(contract.issue === 60, "WSL bundle ownership drifted");
check(
  contract.platformContract === support.contractVersion,
  "WSL support contract drifted",
);
check(
  contract.releaseSetAuthority === releaseSet.contract,
  "release-set authority drifted",
);
check(
  contract.compatibilityMode === false,
  "WSL compatibility mode is forbidden",
);
check(
  support.implementationOwnership.bundleCertification === 60,
  "wrong WSL bundle owner",
);
check(
  same(
    contract.components.map(({ installName }) => installName),
    [
      "nimino",
      "nimino-acp",
      "nimino-agent",
      "nimino-core-worker",
      "nimino-data-ops",
      "nimino-dev-mcp",
      "nimino-relay",
    ],
  ),
  "WSL bundle inventory drifted",
);
check(
  same(
    contract.components
      .filter(({ role }) => ["cli", "acp", "agent", "mcp"].includes(role))
      .map(({ role, artifactId, installName }) => ({
        role,
        artifactId,
        installName,
      })),
    agentBundle.components,
  ),
  "WSL agent surface drifted from its component contract",
);
for (const proof of [
  "manifest-pinned-clean-install",
  "relay-health",
  "data-verify",
  "cli-agent-mcp-execution",
  "atomic-update-rollback",
  "checksum-and-provenance",
  "failure-cleanup",
  "uninstall-orphan-scan",
]) {
  check(
    contract.requiredProofs.includes(proof),
    `missing WSL bundle proof: ${proof}`,
  );
}
check(
  service.includes("--bundle") && !service.includes("--relay"),
  "single-relay install path remains",
);
for (const signal of [
  "nimino-release-set.mjs",
  "--artifact-dir",
  "SHA256SUMS",
  "renameSync(staging, outputPath)",
  "if (staging) rmSync",
]) {
  check(composer.includes(signal), `missing WSL composer guarantee: ${signal}`);
}
check(
  /^wsl-bundle-contract:\n {4}node scripts\/test-nimino-wsl-bundle-contract\.mjs$/m.test(
    justfile,
  ) &&
    /^wsl-bundle-e2e: wsl-bundle-contract\n {4}node scripts\/test-nimino-wsl-bundle-contract\.mjs --e2e$/m.test(
      justfile,
    ) &&
    /^wsl-bundle-certify: wsl-bundle-contract\n {4}node scripts\/test-nimino-wsl-bundle-contract\.mjs --certify$/m.test(
      justfile,
    ) &&
    workflow.includes("run: just wsl-bundle-contract") &&
    hooks.includes("wsl-bundle-contract:"),
  "WSL bundle gates are not wired into local and CI policy",
);

if (!process.argv.includes("--e2e") && !process.argv.includes("--certify")) {
  console.log(
    "Nimino WSL bundle contract passed: pinned install, workflow, cleanup, uninstall",
  );
  process.exit(0);
}

function decode(buffer) {
  if (!buffer) return "";
  return (
    buffer.includes(0) ? buffer.toString("utf16le") : buffer.toString("utf8")
  )
    .replaceAll("\0", "")
    .trim();
}

function run(command, args = []) {
  const result = spawnSync(command, args, { cwd: root });
  return {
    ok: result.status === 0,
    stdout: decode(result.stdout),
    stderr: decode(result.stderr),
  };
}

function semverCompare(left, right) {
  const a = left.split(".").map(Number);
  const b = right.split(".").map(Number);
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    if ((a[index] ?? 0) !== (b[index] ?? 0))
      return (a[index] ?? 0) - (b[index] ?? 0);
  }
  return 0;
}

function valueAt(value, path) {
  return path.split(".").reduce((entry, key) => entry?.[key], value);
}

function matches(candidate, rule) {
  const actual = valueAt(candidate, rule.path);
  if (rule.operator === "equals") return same(actual, rule.value);
  if (rule.operator === "at-least")
    return typeof actual === "number" && actual >= rule.value;
  if (rule.operator === "semver-at-least") {
    return typeof actual === "string" && semverCompare(actual, rule.value) >= 0;
  }
  if (rule.operator === "path-prefix") {
    const normalized =
      typeof actual === "string" ? posix.normalize(actual) : "";
    return normalized.startsWith(rule.value) && !normalized.startsWith("/mnt/");
  }
  return false;
}

function detectSecretService() {
  const alias = run("gdbus", [
    "call",
    "--session",
    "--dest",
    "org.freedesktop.secrets",
    "--object-path",
    "/org/freedesktop/secrets",
    "--method",
    "org.freedesktop.Secret.Service.ReadAlias",
    "default",
  ]);
  if (!alias.ok) return { backend: "unavailable", serviceState: "missing" };
  const collection = alias.stdout.match(/objectpath '([^']+)'/)?.[1];
  if (!collection || collection === "/") {
    return { backend: "Linux Secret Service", serviceState: "missing" };
  }
  const locked = run("gdbus", [
    "call",
    "--session",
    "--dest",
    "org.freedesktop.secrets",
    "--object-path",
    collection,
    "--method",
    "org.freedesktop.DBus.Properties.Get",
    "org.freedesktop.Secret.Collection",
    "Locked",
  ]);
  return {
    backend: "Linux Secret Service",
    serviceState:
      locked.ok && /false/.test(locked.stdout)
        ? "available-unlocked"
        : "locked",
  };
}

function detectCandidate(repositoryRoot) {
  const osRelease = Object.fromEntries(
    readFileSync("/etc/os-release", "utf8")
      .split("\n")
      .filter((line) => line.includes("="))
      .map((line) => {
        const index = line.indexOf("=");
        return [
          line.slice(0, index),
          line.slice(index + 1).replace(/^"|"$/g, ""),
        ];
      }),
  );
  const wslVersionText = run("/mnt/c/Windows/System32/wsl.exe", [
    "--version",
  ]).stdout;
  const wslVersion =
    wslVersionText
      .match(/\d+\.\d+\.\d+\.\d+/)?.[0]
      ?.split(".")
      .slice(0, 3)
      .join(".") ?? "unknown";
  const windowsText = run("/mnt/c/Windows/System32/cmd.exe", [
    "/d",
    "/c",
    "ver",
  ]).stdout;
  const hostBuild = Number(windowsText.match(/10\.0\.(\d+)/)?.[1] ?? 0);
  const distroPath = run("wslpath", ["-w", "/"]).stdout;
  const distribution =
    distroPath.match(/^\\\\wsl(?:\.localhost)?\\([^\\]+)\\/)?.[1] ?? "unknown";
  const architecture = run("uname", ["-m"]).stdout;
  const pidOne = readFileSync("/proc/1/comm", "utf8").trim();
  const network = run("wslinfo", ["--networking-mode"]);
  return {
    host: {
      os: hostBuild >= 22000 ? "Windows 11" : "unknown",
      build: hostBuild,
      architecture,
    },
    wsl: {
      generation: os.release().toLowerCase().includes("wsl2") ? 2 : 1,
      distribution,
      distributionRelease: `${osRelease.VERSION_ID} LTS`,
      architecture,
      version: wslVersion,
      updateChannel: /^\d+\.\d+\.\d+$/.test(wslVersion) ? "stable" : "unknown",
      systemd: pidOne === "systemd",
    },
    gui: {
      method: existsSync("/mnt/wslg") ? "WSLg" : "unavailable",
      windowsNativeShell: false,
    },
    filesystem: { livePath: repositoryRoot },
    network: { mode: network.ok ? network.stdout : "unknown" },
    secrets: detectSecretService(),
  };
}

const kernelRelease = os.release();
check(
  kernelRelease.toLowerCase().includes("microsoft"),
  "WSL bundle E2E requires real WSL2",
);
const repositoryRoot = realpathSync(root);
check(
  repositoryRoot.startsWith("/home/"),
  "WSL bundle E2E workspace must stay under /home",
);
const filesystem = run("stat", ["-f", "-c", "%T", repositoryRoot]);
check(
  filesystem.ok && ["ext2/ext3", "ext4"].includes(filesystem.stdout),
  `WSL bundle E2E requires ext4, got ${filesystem.stdout || "unknown"}`,
);

const evidencePath = resolve(
  root,
  process.env.NIMINO_WSL_BUNDLE_EVIDENCE ?? contract.evidencePath,
);
mkdirSync(dirname(evidencePath), { recursive: true });
const candidate = detectCandidate(repositoryRoot);
const failedRules = support.supportedConfiguration
  .filter((rule) => !matches(candidate, rule))
  .map(({ path, operator, value }) => ({
    path,
    operator,
    expected: value,
    actual: valueAt(candidate, path),
  }));
if (process.argv.includes("--certify") && failedRules.length > 0) {
  writeFileSync(
    evidencePath,
    `${JSON.stringify(
      {
        schemaVersion: 1,
        contract: contract.contract,
        issue: contract.issue,
        kernelRelease,
        filesystemType: filesystem.stdout,
        candidate,
        failedRules,
        result: "unsupported",
      },
      null,
      2,
    )}\n`,
  );
  console.error(
    `error: WSL release candidate is unsupported: ${failedRules.map(({ path }) => path).join(", ")}`,
  );
  process.exit(1);
}

const temporaryRoot = join(root, "target/nim/wsl-bundle-tmp");
mkdirSync(temporaryRoot, { recursive: true });
try {
  const e2e = spawnSync(
    process.execPath,
    [join(root, "scripts/test-nimino-wsl-service.mjs")],
    {
      cwd: root,
      env: { ...process.env, TMPDIR: temporaryRoot },
      stdio: "inherit",
    },
  );
  check(e2e.status === 0, "WSL bundle lifecycle E2E failed");
} finally {
  rmSync(temporaryRoot, { recursive: true, force: true });
}
writeFileSync(
  evidencePath,
  `${JSON.stringify(
    {
      schemaVersion: 1,
      contract: contract.contract,
      issue: contract.issue,
      kernelRelease,
      filesystemType: filesystem.stdout,
      candidate,
      platformQualification: process.argv.includes("--certify")
        ? "supported"
        : "not-evaluated",
      requiredProofs: contract.requiredProofs,
      result: "passed",
    },
    null,
    2,
  )}\n`,
);
console.log(`Nimino WSL bundle E2E passed: ${evidencePath}`);
