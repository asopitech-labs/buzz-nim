#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contractPath = join(root, "contracts", "mobile-removal-v1.json");
const policyArtifacts = new Set([
  "contracts/mobile-removal-v1.json",
  "docs/architecture/mobile-removal-inventory.md",
  "scripts/check-mobile-removal-contract.mjs",
]);
const mandatoryReferences = new Set([
  "bin/.flutter-3.41.7.pkg",
  "bin/dart",
  "bin/flutter",
  "crates/buzz-media/src/validation.rs",
]);
const mobileReference = /mobile\/|mobile-v|mobilepairing|mobile-pairing|mobile app|mobile device|flutter|nip-ab|kind_pairing|pairing_relay|pairingrelay|start_pairing|start_identity_recovery_pairing|identityrecoverypairing|nostrpair|buzz-push-gateway|buzz_push_|\bapns\b|nip-pl|push_gateway/iu;
const requiredSurfaceIds = new Set([
  "mobile.root-metadata",
  "mobile.dart-product",
  "mobile.dart-tests",
  "mobile.assets",
  "mobile.android",
  "mobile.ios",
  "mobile.local-tooling",
  "external.mobile-dedicated-release",
  "external.mobile-toolchain-shims",
  "external.shared-doc-build-references",
  "external.mobile-push-dedicated",
  "external.mobile-push-shared-integration",
  "external.desktop-identity-pairing",
  "external.nip-ab-interop",
  "external.pairing-relay-runtime",
  "external.platform-media-interop",
  "external.protocol-mobile-examples",
  "external.non-product-lexemes",
  "external.tauri-mobile-icons",
]);
const inventorySurfaceCounts = {
  "mobile.root-metadata": 9,
  "mobile.dart-product": 321,
  "mobile.dart-tests": 133,
  "mobile.assets": 9,
  "mobile.android": 49,
  "mobile.ios": 57,
  "mobile.local-tooling": 2,
  "external.mobile-dedicated-release": 8,
  "external.mobile-toolchain-shims": 3,
  "external.shared-doc-build-references": 14,
  "external.mobile-push-dedicated": 52,
  "external.mobile-push-shared-integration": 18,
  "external.desktop-identity-pairing": 18,
  "external.nip-ab-interop": 16,
  "external.pairing-relay-runtime": 9,
  "external.platform-media-interop": 16,
  "external.protocol-mobile-examples": 3,
  "external.non-product-lexemes": 5,
  "external.tauri-mobile-icons": 35,
};

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function matches(path, selector) {
  return selector.type === "exact" ? path === selector.value : path.startsWith(selector.value);
}

function resolvesInsideRoot(path) {
  return resolve(root, path).startsWith(`${root}${sep}`);
}

function trackedFiles() {
  return execFileSync("git", ["ls-files", "-z"], { cwd: root })
    .toString("utf8")
    .split("\0")
    .filter(Boolean);
}

function isText(path) {
  const bytes = readFileSync(join(root, path));
  return !bytes.subarray(0, 8192).includes(0);
}

function evidencePath(evidence, context) {
  check(evidence && ["path", "symbol", "signal"].every((key) => typeof evidence[key] === "string" && evidence[key]), `${context}: invalid evidence`);
  check(resolvesInsideRoot(evidence.path), `${context}: evidence escapes repository root`);
  return resolve(root, evidence.path);
}

function validateEvidence(evidence, context) {
  const path = evidencePath(evidence, context);
  check(existsSync(path), `${context}: missing evidence path ${evidence.path}`);
  check(readFileSync(path, "utf8").includes(evidence.symbol), `${context}: missing symbol ${evidence.symbol}`);
}

function validateEvidenceAbsent(evidence, context) {
  const path = evidencePath(evidence, context);
  check(!existsSync(path) || !readFileSync(path, "utf8").includes(evidence.symbol), `${context}: legacy symbol remains ${evidence.symbol}`);
}

check(existsSync(contractPath), "missing contracts/mobile-removal-v1.json");
const contract = JSON.parse(readFileSync(contractPath, "utf8"));
const files = trackedFiles();

check(contract.schemaVersion === 1, "schemaVersion must be 1");
check(contract.contractVersion === "mobile-removal-v1", "wrong contractVersion");
check(contract.issue === 18, "contract must belong to issue #18");
check(new Set(["inventory", "removed"]).has(contract.phase), "phase must be inventory or removed");
check(contract.phaseTransitionOwner === 33, "issue #33 must own the permanent zero-state transition");
check(
  Number.isInteger(contract.expectedCounts?.mobileFiles) && Number.isInteger(contract.expectedCounts?.externalReferences),
  "expectedCounts must contain integer file counts",
);
check(contract.pairingDecision.productUseCase === "desktop-identity-transfer-and-recovery", "NIP-AB must have one non-Mobile product use case");
check(contract.pairingDecision.source === "an authorized Nimino Desktop sends its identity after SAS confirmation", "NIP-AB source must be an authorized Desktop with SAS");
check(contract.pairingDecision.target === "a fresh Nimino Desktop receives the identity during onboarding", "NIP-AB target must be a fresh Desktop");
check(contract.pairingDecision.protocol === "NIP-AB v1 over an ephemeral pairing relay", "pairing protocol must remain NIP-AB v1 over an ephemeral relay");
check(contract.pairingDecision.owner === "Desktop identity lifecycle backed by the existing Rust NIP-AB protocol adapter", "Rust must remain the NIP-AB protocol adapter");
check(contract.pairingDecision.mobileClient === "delete", "Mobile pairing client must be deleted");
check(contract.pairingDecision.mobileWording === "delete", "Mobile pairing wording must be deleted");
check(contract.pairingDecision.ownerIssue === 28, "Desktop pairing surface must remain owned by issue #28");
check(contract.physicalDeletionOwner === 33, "Mobile physical deletion must remain owned by issue #33");
for (const unsupported of ["automated or repeated secret transfer", "Chirps cluster negotiation", "database replication", "automatic sync"]) {
  check(contract.pairingDecision.unsupported.includes(unsupported), `NIP-AB must not own ${unsupported}`);
}

const actions = new Set(["keep", "shrink", "delete"]);
const selectorTypes = new Set(["exact", "prefix"]);
const surfaces = [...contract.mobileTree, ...contract.externalSurfaces];
const surfaceIds = surfaces.map(({ id }) => id);
check(new Set(surfaceIds).size === requiredSurfaceIds.size && surfaceIds.every((id) => requiredSurfaceIds.has(id)), "surface ID set changed without contract review");
for (const surface of surfaces) {
  check(actions.has(surface.action), `${surface.id}: invalid action`);
  check([surface.owner, surface.deleteWhen, surface.proof].every((value) => typeof value === "string" && value), `${surface.id}: incomplete lifecycle`);
  check(surface.dependencies === undefined || (Array.isArray(surface.dependencies) && surface.dependencies.every((value) => typeof value === "string" && value)), `${surface.id}: invalid dependencies`);
  check(Array.isArray(surface.selectors) && surface.selectors.length > 0, `${surface.id}: missing selectors`);
  for (const selector of surface.selectors) {
    check(selectorTypes.has(selector.type), `${surface.id}: invalid selector type`);
    check(typeof selector.value === "string" && selector.value.length > 0, `${surface.id}: empty selector`);
    check(resolvesInsideRoot(selector.value), `${surface.id}: selector escapes repository root`);
  }
  if (contract.phase === "inventory") validateEvidence(surface.evidence, surface.id);
  else if (surface.action !== "delete") validateEvidence(surface.postRemovalEvidence ?? surface.evidence, surface.id);
  if (surface.action === "shrink") validateEvidence(surface.postRemovalEvidence, `${surface.id} post-removal`);
  if (contract.phase === "removed" && surface.action === "shrink") validateEvidenceAbsent(surface.evidence, surface.id);
}

if (contract.phase === "inventory") {
  for (const surface of surfaces) {
    const selected = files.filter((path) => surface.selectors.some((selector) => matches(path, selector))).length;
    check(selected === inventorySurfaceCounts[surface.id], `${surface.id}: selected path count changed: expected ${inventorySurfaceCounts[surface.id]}, found ${selected}`);
  }
}

const mobileFiles = files.filter((path) => path.startsWith("mobile/"));
check(mobileFiles.length === contract.expectedCounts.mobileFiles, "Mobile file count changed without inventory review");
check(contract.phase !== "inventory" || mobileFiles.length > 0, "mobile tree unexpectedly empty before issue #33");
check(contract.phase !== "removed" || mobileFiles.length === 0, "removed phase forbids tracked mobile paths");
const classificationErrors = [];
for (const path of mobileFiles) {
  const owners = contract.mobileTree.filter((surface) => surface.selectors.some((selector) => matches(path, selector)));
  if (owners.length !== 1) classificationErrors.push(`${path}: Mobile-tree owners=${owners.map(({ id }) => id).join(",") || "none"}`);
  else if (owners[0].action !== "delete") classificationErrors.push(`${path}: Mobile-tree owner must delete`);
}

const externalReferences = files.filter((path) => {
  if (path.startsWith("mobile/") || policyArtifacts.has(path)) return false;
  return mandatoryReferences.has(path) || mobileReference.test(path) || (isText(path) && mobileReference.test(readFileSync(join(root, path), "utf8")));
});
check(
  externalReferences.length === contract.expectedCounts.externalReferences,
  `external reference count changed without inventory review: expected ${contract.expectedCounts.externalReferences}, found ${externalReferences.length}`,
);
for (const path of externalReferences) {
  const owners = contract.externalSurfaces.filter((surface) => surface.selectors.some((selector) => matches(path, selector)));
  if (owners.length !== 1) classificationErrors.push(`${path}: external owners=${owners.map(({ id }) => id).join(",") || "none"}`);
}
for (const path of files) {
  const owners = surfaces.filter((surface) => surface.selectors.some((selector) => matches(path, selector)));
  if (owners.length > 1) classificationErrors.push(`${path}: overlapping owners=${owners.map(({ id }) => id).join(",")}`);
}
check(classificationErrors.length === 0, `classification errors:\n${classificationErrors.join("\n")}`);

for (const surface of surfaces) {
  for (const selector of surface.selectors) {
    const selected = files.some((path) => matches(path, selector));
    if (contract.phase === "removed" && surface.action === "delete") {
      check(!selected, `${surface.id}: delete selector remains after cutover ${selector.value}`);
    } else if (contract.phase === "inventory" || surface.action === "keep") {
      check(selected, `${surface.id}: stale selector ${selector.value}`);
    }
  }
}

console.log(`Mobile removal contract passed: ${mobileFiles.length} Mobile files and ${externalReferences.length} external reference files classified`);
