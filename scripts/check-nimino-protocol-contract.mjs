#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contractPath = join(root, "contracts", "nimino-protocol-v1.json");
const fixturePath = join(root, "contracts", "fixtures", "old-buzz-client-v1.json");
const namingPath = join(root, "contracts", "nimino-naming-v1.json");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function same(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function sortedUnique(values) {
  return [...new Set(values)].sort((left, right) =>
    typeof left === "number" ? left - right : left.localeCompare(right),
  );
}

check(existsSync(contractPath), "missing contracts/nimino-protocol-v1.json");
check(existsSync(fixturePath), "missing contracts/fixtures/old-buzz-client-v1.json");

const contract = JSON.parse(readFileSync(contractPath, "utf8"));
const fixture = JSON.parse(readFileSync(fixturePath, "utf8"));
const naming = JSON.parse(readFileSync(namingPath, "utf8"));

check(contract.schemaVersion === 1, "protocol schemaVersion must be 1");
check(contract.contract === "nimino.protocol", "unexpected protocol contract id");
check(contract.protocolVersion === 1, "protocolVersion must be 1");
check(contract.capability === "nimino-v1", "unexpected protocol capability");
check(contract.compatibilityMode === false, "Buzz compatibility mode is forbidden");
check(contract.cutoverOwner === 12, "Issue #12 must own physical cutover");
check(contract.namingContract === naming.contract, "naming contract link drifted");
check(Array.isArray(contract.surfaces), "protocol surfaces must be an array");

const decisions = new Set(["keep", "redefine", "delete"]);
const types = new Set(["kind", "tag", "route", "schema", "query-field", "capability"]);
const ids = contract.surfaces.map(({ id }) => id);
check(new Set(ids).size === ids.length, "protocol surface ids must be unique");
const classifiedValues = new Set();
for (const surface of contract.surfaces) {
  check(types.has(surface.type), `${surface.id}: invalid surface type`);
  check(decisions.has(surface.decision), `${surface.id}: invalid decision`);
  check(typeof surface.reason === "string" && surface.reason, `${surface.id}: missing reason`);
  check(
    Array.isArray(surface.values) && surface.values.length > 0,
    `${surface.id}: values must be non-empty`,
  );
  check(
    same(surface.values, sortedUnique(surface.values)),
    `${surface.id}: values must be sorted and unique`,
  );
  if (surface.legacyValues !== undefined) {
    check(surface.decision === "redefine", `${surface.id}: only redefine has legacyValues`);
    check(
      Array.isArray(surface.legacyValues) &&
        surface.legacyValues.length > 0 &&
        same(surface.legacyValues, sortedUnique(surface.legacyValues)),
      `${surface.id}: legacyValues must be sorted and unique`,
    );
  }
  for (const value of surface.values) {
    const key = `${surface.type}\0${value}`;
    check(!classifiedValues.has(key), `${surface.id}: duplicate classification for ${value}`);
    classifiedValues.add(key);
  }
  for (const value of surface.legacyValues ?? []) {
    const key = `${surface.type}\0${value}`;
    check(!classifiedValues.has(key), `${surface.id}: duplicate legacy classification for ${value}`);
    classifiedValues.add(key);
  }
}
for (const type of types) {
  check(contract.surfaces.some((surface) => surface.type === type), `missing ${type} surface`);
}
for (const decision of decisions) {
  check(
    contract.surfaces.some((surface) => surface.decision === decision),
    `missing ${decision} decision`,
  );
}

const kindSource = readFileSync(join(root, "crates", "nimino-core", "src", "kind.rs"), "utf8");
const registeredKinds = sortedUnique(
  [...kindSource.matchAll(/pub const (?:KIND_[A-Z0-9_]+|RELAY_ADMIN_[A-Z0-9_]+): u32 = (\d+);/g)].map(
    (match) => Number(match[1]),
  ),
);
const activeKinds = sortedUnique(
  contract.surfaces
    .filter((surface) => surface.type === "kind" && surface.decision !== "delete")
    .flatMap((surface) => surface.values),
);
check(same(activeKinds, registeredKinds), "kind registry changed without protocol classification");
const deletedKinds = contract.surfaces
  .filter((surface) => surface.type === "kind" && surface.decision === "delete")
  .flatMap((surface) => surface.values);
check(
  deletedKinds.every((kind) => !registeredKinds.includes(kind)),
  "deleted kind remains registered",
);

const nip11 = readFileSync(join(root, "crates", "nimino-relay", "src", "nip11.rs"), "utf8");
const supportedNips = nip11
  .match(/SUPPORTED_NIPS: &\[u32\] = &\[([^\]]+)\]/)?.[1]
  .split(",")
  .map((value) => Number(value.trim()))
  .filter(Number.isInteger);
check(same(supportedNips, contract.standardNips), "NIP-11 standard NIP inventory drifted");
check(
  contract.standardNips.every((nip, index) => index === 0 || contract.standardNips[index - 1] < nip),
  "standardNips must be sorted and unique",
);
const conditionalNip = Number(nip11.match(/NIP_RELAY_MEMBERSHIP: u32 = (\d+);/)?.[1]);
check(same(contract.conditionalNips, [conditionalNip]), "conditional NIP inventory drifted");
const advertisedExtensions = [
  ...nip11.matchAll(/supported_extensions: Some\(vec!\["([^"]+)"\.to_string\(\)\]\)/g),
].map((match) => match[1]);
const keptCapabilities = contract.surfaces
  .filter((surface) => surface.type === "capability" && surface.decision === "keep")
  .flatMap((surface) => surface.values);
check(
  same(advertisedExtensions, keptCapabilities),
  "currently advertised capability inventory drifted",
);

const routeSources = [
  "crates/nimino-relay/src/router.rs",
  "crates/nimino-relay/src/api/admin/mod.rs",
  "crates/nimino-relay/src/api/git/mod.rs",
  "crates/nimino-relay/src/api/git/transport.rs",
];
const registeredRoutes = sortedUnique(
  routeSources.flatMap((path) => {
    const source = readFileSync(join(root, path), "utf8");
    return [...source.matchAll(/\.(?:route|nest)\(\s*"([^"]+)"/g)].map((match) => match[1]);
  }),
);
const classifiedRoutes = sortedUnique(
  contract.surfaces
    .filter((surface) => surface.type === "route" && surface.decision === "keep")
    .flatMap((surface) => surface.values),
);
check(same(classifiedRoutes, registeredRoutes), "relay route inventory changed without classification");
for (const route of contract.surfaces.find((surface) => surface.id === "route.legacy").values) {
  check(!registeredRoutes.includes(route), `legacy route remains registered: ${route}`);
}

const nipDocs = execFileSync("git", ["ls-files", "docs/nips/NIP-*.md"], { cwd: root })
  .toString("utf8")
  .trim()
  .split("\n")
  .filter(Boolean)
  .sort();
const classifiedDocs = sortedUnique(
  contract.surfaces
    .filter((surface) => surface.type === "schema")
    .flatMap((surface) => surface.values)
    .filter((value) => value.startsWith("docs/nips/")),
);
check(same(classifiedDocs, nipDocs), "custom NIP schema inventory changed without classification");

const bridge = readFileSync(join(root, "crates", "nimino-relay", "src", "api", "bridge.rs"), "utf8");
const bridgeExtractors = bridge.slice(
  bridge.indexOf("// The CLI injects extension fields"),
  bridge.indexOf("const BRIDGE_WINDOW_DEFAULT_LIMIT"),
);
const registeredQueryFields = sortedUnique([
  ...[...bridgeExtractors.matchAll(/\.get\("([^"]+)"\)/g)].map((match) => match[1]),
  ...[...bridge.matchAll(/extension_flag\(raw, "([^"]+)"\)/g)].map((match) => match[1]),
]);
const classifiedQueryFields = sortedUnique(
  contract.surfaces
    .filter((surface) => surface.type === "query-field")
    .flatMap((surface) => surface.values),
);
check(
  same(classifiedQueryFields, registeredQueryFields),
  "bridge query extension inventory changed without classification",
);

const namingSurfaces = new Map(naming.surfaces.map((surface) => [surface.id, surface]));
for (const surface of contract.surfaces.filter((candidate) => candidate.type === "tag")) {
  for (const id of surface.values) {
    const linked = namingSurfaces.get(id);
    check(linked, `${surface.id}: unknown naming surface ${id}`);
    check(linked.match === "prefix", `${surface.id}: tag naming surface must use prefix matching`);
    check(linked.canonical.startsWith("nimino"), `${surface.id}: non-Nimino canonical tag prefix`);
    check(linked.legacy.every((value) => value.startsWith("buzz")), `${surface.id}: legacy tag prefix drifted`);
  }
}

function findValue(type, value) {
  for (const surface of contract.surfaces.filter((candidate) => candidate.type === type)) {
    if (surface.values.includes(value)) return { surface, legacy: false };
    if (surface.legacyValues?.includes(value)) return { surface, legacy: true };
    if (type === "tag") {
      for (const namingId of surface.values) {
        const linked = namingSurfaces.get(namingId);
        if (value.startsWith(linked.canonical)) return { surface, legacy: false };
        if (linked.legacy.some((prefix) => value.startsWith(prefix))) return { surface, legacy: true };
      }
    }
  }
  return null;
}

const standardFilterFields = new Set([
  "#a",
  "#d",
  "#e",
  "#h",
  "#p",
  "#t",
  "authors",
  "ids",
  "kinds",
  "limit",
  "search",
  "since",
  "until",
]);

function classify(operation) {
  const matches = [];
  if (operation.type === "event") {
    matches.push(findValue("kind", operation.kind));
    for (const tag of operation.tags ?? []) {
      const tagMatch = findValue("tag", tag[0]);
      if (tagMatch) matches.push(tagMatch);
    }
  } else if (operation.type === "query") {
    for (const field of Object.keys(operation.filter)) {
      if (!standardFilterFields.has(field)) matches.push(findValue("query-field", field));
    }
  } else {
    matches.push(findValue(operation.type, operation.value));
  }
  check(matches.every(Boolean), `unclassified fixture operation: ${JSON.stringify(operation)}`);
  const rejected = matches.find(({ surface, legacy }) => surface.decision === "delete" || legacy);
  return rejected
    ? { outcome: "reject", surface: rejected.surface.id }
    : { outcome: "accept", surface: matches[0].surface.id };
}

check(fixture.schemaVersion === 1, "old-client fixture schemaVersion must be 1");
check(fixture.protocolVersion === contract.protocolVersion, "fixture protocolVersion drifted");
check(Array.isArray(fixture.cases) && fixture.cases.length > 0, "old-client fixture is empty");
check(
  new Set(fixture.cases.map(({ id }) => id)).size === fixture.cases.length,
  "old-client fixture ids must be unique",
);
for (const testCase of fixture.cases) {
  const actual = classify(testCase.operation);
  check(actual.outcome === testCase.expect, `${testCase.id}: expected ${testCase.expect}, got ${actual.outcome}`);
  check(actual.surface === testCase.surface, `${testCase.id}: rejection/acceptance surface drifted`);
}
check(fixture.cases.some(({ expect }) => expect === "reject"), "fixture needs a rejection case");
check(fixture.cases.some(({ expect }) => expect === "accept"), "fixture needs an interop control");

check(
  readFileSync(join(root, "Justfile"), "utf8").includes(
    "protocol-contract:\n    node scripts/check-nimino-protocol-contract.mjs",
  ),
  "protocol contract is not wired into Justfile",
);
check(
  readFileSync(join(root, ".github", "workflows", "ci.yml"), "utf8").includes(
    "run: just protocol-contract",
  ),
  "protocol contract is not wired into CI",
);

console.log(
  `Nimino protocol v${contract.protocolVersion} passed: ${registeredKinds.length} kinds, ${registeredRoutes.length} routes, ${nipDocs.length} schemas, ${fixture.cases.length} fixtures`,
);
