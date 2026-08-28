#!/usr/bin/env node

import { existsSync, readFileSync } from "node:fs";
import { dirname, join, posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contractPath = join(root, "contracts", "wsl-support-v1.json");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function same(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function valueAt(object, path) {
  return path.split(".").reduce((value, key) => value?.[key], object);
}

function setValue(object, path, value) {
  const keys = path.split(".");
  const leaf = keys.pop();
  const parent = keys.reduce((entry, key) => entry[key], object);
  parent[leaf] = value;
}

function compareSemver(left, right) {
  const parse = (version) => version.split(".").map(Number);
  const a = parse(left);
  const b = parse(right);
  for (let index = 0; index < Math.max(a.length, b.length); index += 1) {
    if ((a[index] ?? 0) !== (b[index] ?? 0)) {
      return (a[index] ?? 0) - (b[index] ?? 0);
    }
  }
  return 0;
}

function matchesRule(candidate, { path, operator, value }) {
  const actual = valueAt(candidate, path);
  if (operator === "equals") return JSON.stringify(actual) === JSON.stringify(value);
  if (operator === "at-least") return typeof actual === "number" && actual >= value;
  if (operator === "semver-at-least") {
    return typeof actual === "string" && compareSemver(actual, value) >= 0;
  }
  if (operator === "path-prefix") {
    const normalized = typeof actual === "string" ? posix.normalize(actual) : "";
    return normalized.startsWith(value) && !normalized.startsWith("/mnt/");
  }
  throw new Error(`unknown operator: ${operator}`);
}

function isSupported(candidate, expected) {
  return expected.every((rule) => matchesRule(candidate, rule));
}

check(existsSync(contractPath), "missing contracts/wsl-support-v1.json");
const contract = JSON.parse(readFileSync(contractPath, "utf8"));

check(contract.schemaVersion === 1, "schemaVersion must be 1");
check(
  contract.contractVersion === "wsl-support-v1",
  "contractVersion must be wsl-support-v1",
);
check(contract.issue === 16, "contract must belong to issue #16");
check(Array.isArray(contract.supportedConfiguration), "supportedConfiguration must be an array");
check(
  contract.supportedConfiguration.length === 16,
  "supportedConfiguration must contain the exact v1 rules",
);
check(Array.isArray(contract.unsupportedMatrix), "unsupportedMatrix must be an array");
check(
  contract.unsupportedMatrix.length === 19,
  "unsupportedMatrix must contain the exact v1 negative cases",
);
check(Array.isArray(contract.evidence) && contract.evidence.length >= 3, "local evidence is incomplete");
check(
  Array.isArray(contract.sources) && contract.sources.length === 8,
  "official sources must contain the exact v1 references",
);
check(contract.policies.gui.method === "WSLg", "WSLg must be the sole GUI method");
check(
  contract.policies.filesystem.liveStorage === "distribution-ext4",
  "live storage must use distribution ext4",
);
check(contract.policies.network.mode === "mirrored", "network mode must be mirrored");
check(
  contract.policies.secrets.backend === "Linux Secret Service",
  "secrets must use Linux Secret Service",
);
check(contract.implementationOwnership.installer === 19, "installer lifecycle must remain in issue #19");
check(contract.implementationOwnership.launcher === 25, "launcher boundary must remain in issue #25");
check(contract.implementationOwnership.chirpsRuntime === 45, "WSL Chirps certification must remain in issue #45");
check(
  contract.policies.secrets.legacyPlaintextFallback.ownerIssue === 25,
  "legacy identity.key fallback removal must remain in issue #25",
);
check(
  contract.policies.secrets.legacyPlaintextFallback.action === "delete",
  "legacy identity.key fallback must be deleted",
);

const requiredRules = {
  "host.os": ["equals", "Windows 11"],
  "host.build": ["at-least", 26200],
  "host.architecture": ["equals", "x86_64"],
  "wsl.generation": ["equals", 2],
  "wsl.distribution": ["equals", "Ubuntu-24.04"],
  "wsl.distributionRelease": ["equals", "24.04 LTS"],
  "wsl.architecture": ["equals", "x86_64"],
  "wsl.version": ["semver-at-least", "2.7.3"],
  "wsl.updateChannel": ["equals", "stable"],
  "wsl.systemd": ["equals", true],
  "gui.method": ["equals", "WSLg"],
  "gui.windowsNativeShell": ["equals", false],
  "filesystem.livePath": ["path-prefix", "/home/"],
  "network.mode": ["equals", "mirrored"],
  "secrets.backend": ["equals", "Linux Secret Service"],
  "secrets.serviceState": ["equals", "available-unlocked"],
};
const actualRulePaths = contract.supportedConfiguration.map(({ path }) => path).sort();
check(
  same(actualRulePaths, Object.keys(requiredRules).sort()),
  "support rule paths must exactly match v1",
);
for (const rule of contract.supportedConfiguration) {
  check(
    same(Object.keys(rule).sort(), ["operator", "path", "value"]),
    `invalid support rule shape: ${rule.path}`,
  );
  check(
    same([rule.operator, rule.value], requiredRules[rule.path]),
    `unexpected support rule: ${rule.path}`,
  );
}

for (const evidence of contract.evidence) {
  const sourcePath = join(root, evidence.path);
  check(existsSync(sourcePath), `missing evidence path: ${evidence.path}`);
  check(
    readFileSync(sourcePath, "utf8").includes(evidence.symbol),
    `missing evidence symbol: ${evidence.path} :: ${evidence.symbol}`,
  );
}

const requiredSources = [
  "https://documentation.ubuntu.com/wsl/latest/reference/release_notes/",
  "https://github.com/microsoft/WSL/releases/tag/2.7.3",
  "https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information",
  "https://learn.microsoft.com/en-us/windows/wsl/filesystems",
  "https://learn.microsoft.com/en-us/windows/wsl/networking",
  "https://learn.microsoft.com/en-us/windows/wsl/systemd",
  "https://learn.microsoft.com/en-us/windows/wsl/tutorials/gui-apps",
  "https://specifications.freedesktop.org/secret-service/latest/",
].sort();
check(
  same(contract.sources.map(({ url }) => url).sort(), requiredSources),
  "official source URLs must exactly match v1",
);
for (const source of contract.sources) {
  const url = new URL(source.url);
  check(url.protocol === "https:", `non-primary source: ${source.url}`);
}

check(
  isSupported(contract.canonicalCandidate, contract.supportedConfiguration),
  "canonical candidate must be supported",
);
const ids = new Set();
const requiredNegativeCases = {
  "wrong-host-os": ["host.os", "Windows 10"],
  "old-windows-build": ["host.build", 26100],
  "windows-arm64": ["host.architecture", "arm64"],
  wsl1: ["wsl.generation", 1],
  "unversioned-ubuntu": ["wsl.distribution", "Ubuntu"],
  "ubuntu-26-04": ["wsl.distributionRelease", "26.04 LTS"],
  "wsl-arm64": ["wsl.architecture", "arm64"],
  "old-wsl-package": ["wsl.version", "2.7.2"],
  "preview-wsl-channel": ["wsl.updateChannel", "pre-release"],
  "systemd-disabled": ["wsl.systemd", false],
  "windows-native-shell": ["gui.method", "Windows-native-shell"],
  "windows-shell-enabled": ["gui.windowsNativeShell", true],
  "third-party-x-server": ["gui.method", "third-party-X-server"],
  "windows-mounted-live-data": [
    "filesystem.livePath",
    "/mnt/c/Users/example/Nimino",
  ],
  "escaped-windows-mounted-live-data": [
    "filesystem.livePath",
    "/home/nimino/../../mnt/c/Nimino",
  ],
  "nat-network": ["network.mode", "NAT"],
  "windows-credential-manager": [
    "secrets.backend",
    "Windows Credential Manager",
  ],
  "secret-service-missing": ["secrets.serviceState", "missing"],
  "secret-service-locked": ["secrets.serviceState", "locked"],
};
check(
  same(
    contract.unsupportedMatrix.map(({ id }) => id).sort(),
    Object.keys(requiredNegativeCases).sort(),
  ),
  "unsupported case IDs must exactly match v1",
);
for (const negative of contract.unsupportedMatrix) {
  check(!ids.has(negative.id), `duplicate unsupported case: ${negative.id}`);
  ids.add(negative.id);
  check(
    same(Object.keys(negative).sort(), ["expect", "id", "set"]),
    `invalid unsupported case shape: ${negative.id}`,
  );
  check(
    same(Object.keys(negative.set).sort(), ["path", "value"]),
    `invalid unsupported mutation shape: ${negative.id}`,
  );
  check(negative.expect === "reject", `${negative.id} must reject`);
  check(
    same(
      [negative.set.path, negative.set.value],
      requiredNegativeCases[negative.id],
    ),
    `unexpected unsupported mutation: ${negative.id}`,
  );
  const candidate = structuredClone(contract.canonicalCandidate);
  setValue(candidate, negative.set.path, negative.set.value);
  check(
    !isSupported(candidate, contract.supportedConfiguration),
    `${negative.id} was unexpectedly supported`,
  );
}

console.log(
  `WSL support contract passed: 1 supported configuration, ${contract.unsupportedMatrix.length} rejected configurations`,
);
