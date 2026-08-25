#!/usr/bin/env node

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifestPath = join(root, "contracts", "gui-surface-v1.json");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function same(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function exactKeys(value, keys) {
  return same(Object.keys(value).sort(), [...keys].sort());
}

function sorted(values) {
  return [...values].sort();
}

function virtualRoutes(path) {
  const source = readFileSync(join(root, path), "utf8");
  const routes = [...source.matchAll(/\broute\(\s*"([^"]+)"/g)].map((match) => match[1]);
  check(/\bindex\(/.test(source), `${path} must declare an index route`);
  return sorted(["/", ...routes]);
}

function validateEvidence(evidence, context) {
  check(exactKeys(evidence, ["path", "symbol", "signal"]), `invalid evidence: ${context}`);
  const path = join(root, evidence.path);
  check(existsSync(path), `missing evidence path for ${context}: ${evidence.path}`);
  check(
    typeof evidence.symbol === "string" &&
      evidence.symbol.length > 0 &&
      readFileSync(path, "utf8").includes(evidence.symbol),
    `evidence symbol drifted for ${context}: ${evidence.symbol}`,
  );
  check(
    typeof evidence.signal === "string" && evidence.signal.length > 0,
    `missing evidence signal: ${context}`,
  );
}

check(existsSync(manifestPath), "missing contracts/gui-surface-v1.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

check(
  exactKeys(manifest, [
    "schemaVersion",
    "contract",
    "scope",
    "actions",
    "crossCuttingOwners",
    "clients",
  ]),
  "unexpected GUI contract fields",
);
check(manifest.schemaVersion === 1, "GUI contract schemaVersion must be 1");
check(manifest.contract === "nimino.gui-surfaces", "unexpected GUI contract id");
check(
  same(manifest.scope, ["desktop", "web", "admin-web"]),
  "GUI scope must exclude Mobile and cover only active clients",
);
check(
  same(Object.keys(manifest.actions).sort(), ["delete", "keep", "merge", "shrink"]),
  "GUI actions must be keep/shrink/merge/delete",
);
check(
  Object.values(manifest.actions).every((description) =>
    Boolean(typeof description === "string" && description.length > 0),
  ),
  "GUI actions need descriptions",
);

const ownerIssues = manifest.crossCuttingOwners.map((owner) => owner.issue);
check(
  same(sorted(ownerIssues), [21, 23, 34, 39, 44]),
  "GUI downstream owner inventory drifted",
);
for (const owner of manifest.crossCuttingOwners) {
  check(
    exactKeys(owner, ["concern", "issue", "responsibility"]) &&
      typeof owner.concern === "string" &&
      typeof owner.responsibility === "string" &&
      owner.responsibility.length > 0,
    `invalid cross-cutting owner for #${owner.issue}`,
  );
}

check(Array.isArray(manifest.clients), "GUI clients must be an array");
check(
  same(
    manifest.clients.map((client) => client.id),
    ["desktop", "web", "admin-web"],
  ),
  "GUI client inventory drifted",
);

const allSurfaces = new Map();
for (const client of manifest.clients) {
  check(
    exactKeys(client, ["id", "root", "routeSource", "routes", "surfaces"]),
    `unexpected client fields: ${client.id}`,
  );
  check(existsSync(join(root, client.root)), `missing client root: ${client.root}`);
  check(existsSync(join(root, client.routeSource)), `missing route source: ${client.routeSource}`);
  check(Array.isArray(client.surfaces) && client.surfaces.length > 0, `${client.id} has no surfaces`);

  for (const surface of client.surfaces) {
    const surfaceKeys = [
      "id",
      "action",
      "owner",
      "implementationIssue",
      "featureRoots",
      "dataContract",
      "replacement",
      "usageEvidence",
    ];
    if (surface.id === "desktop.pulse") surfaceKeys.push("capabilityDispositions");
    check(
      exactKeys(surface, surfaceKeys),
      `unexpected surface fields: ${surface.id}`,
    );
    check(surface.id.startsWith(`${client.id}.`), `surface belongs to wrong client: ${surface.id}`);
    check(!allSurfaces.has(surface.id), `duplicate GUI surface: ${surface.id}`);
    check(surface.action in manifest.actions, `invalid action for ${surface.id}`);
    check([34, 39].includes(surface.implementationIssue), `invalid implementation owner for ${surface.id}`);
    check(typeof surface.owner === "string" && surface.owner.length > 0, `missing owner for ${surface.id}`);
    check(
      typeof surface.dataContract === "string" && surface.dataContract.length > 0,
      `missing data contract for ${surface.id}`,
    );
    check(Array.isArray(surface.featureRoots), `missing feature roots for ${surface.id}`);
    check(
      Array.isArray(surface.usageEvidence) && surface.usageEvidence.length > 0,
      `missing usage evidence for ${surface.id}`,
    );
    for (const evidence of surface.usageEvidence) {
      validateEvidence(evidence, surface.id);
    }
    if (["merge", "delete"].includes(surface.action)) {
      check(
        exactKeys(surface.replacement, ["targetSurface", "dataContract"]),
        `missing replacement for ${surface.id}`,
      );
    } else {
      check(surface.replacement === null, `unexpected replacement for ${surface.id}`);
    }
    allSurfaces.set(surface.id, surface);
  }
}

for (const surface of allSurfaces.values()) {
  if (surface.replacement) {
    check(
      allSurfaces.has(surface.replacement.targetSurface),
      `unknown replacement target for ${surface.id}: ${surface.replacement.targetSurface}`,
    );
    check(
      surface.replacement.targetSurface.startsWith(`${surface.id.split(".")[0]}.`),
      `replacement crosses client boundary for ${surface.id}`,
    );
    check(
      typeof surface.replacement.dataContract === "string" &&
        surface.replacement.dataContract.length > 0,
      `missing replacement data contract for ${surface.id}`,
    );
  }
}

const pulse = allSurfaces.get("desktop.pulse");
check(pulse, "missing desktop.pulse surface");
const requiredPulseCapabilities = [
  "agent-activity",
  "human-timelines",
  "note-actions",
  "note-publishing",
  "search",
];
check(
  same(
    sorted(pulse.capabilityDispositions.map((capability) => capability.id)),
    requiredPulseCapabilities,
  ),
  "Pulse capability inventory drifted",
);
for (const capability of pulse.capabilityDispositions) {
  check(
    exactKeys(capability, [
      "id",
      "action",
      "targetSurface",
      "dataOwner",
      "dataContract",
      "evidence",
    ]),
    `unexpected Pulse capability fields: ${capability.id}`,
  );
  check(["merge", "delete"].includes(capability.action), `invalid Pulse action: ${capability.id}`);
  check(allSurfaces.has(capability.targetSurface), `unknown Pulse target: ${capability.id}`);
  check(
    typeof capability.dataOwner === "string" &&
      capability.dataOwner.length > 0 &&
      typeof capability.dataContract === "string" &&
      capability.dataContract.length > 0,
    `missing Pulse data owner/contract: ${capability.id}`,
  );
  validateEvidence(capability.evidence, `desktop.pulse.${capability.id}`);
}

const requiredRoutes = {
  desktop: virtualRoutes("desktop/src/app/routes.ts"),
  web: virtualRoutes("web/src/app/routes.ts"),
  // ponytail: Admin has no router; keep its four hand-written paths here until #39 installs one.
  "admin-web": ["/", "/feedback", "/feedback/$feedbackId", "/reports", "/reports/$reportId"],
};
for (const client of manifest.clients) {
  check(
    same(sorted(client.routes.map((route) => route.path)), requiredRoutes[client.id]),
    `${client.id} route inventory drifted`,
  );
  const seenRoutes = new Set();
  for (const route of client.routes) {
    check(
      exactKeys(route, ["path", "action", "surface", "replacement", "evidence"]),
      `unexpected route fields: ${client.id}${route.path}`,
    );
    check(!seenRoutes.has(route.path), `duplicate route: ${client.id}${route.path}`);
    check(route.action in manifest.actions, `invalid route action: ${client.id}${route.path}`);
    check(allSurfaces.has(route.surface), `unknown route surface: ${route.surface}`);
    check(route.surface.startsWith(`${client.id}.`), `route crosses client boundary: ${route.path}`);
    validateEvidence(route.evidence, `${client.id}${route.path}`);
    if (["merge", "delete"].includes(route.action)) {
      check(
        exactKeys(route.replacement, ["targetSurface", "dataContract"]) &&
          allSurfaces.has(route.replacement.targetSurface) &&
          route.replacement.targetSurface.startsWith(`${client.id}.`) &&
          typeof route.replacement.dataContract === "string" &&
          route.replacement.dataContract.length > 0,
        `missing route replacement: ${client.id}${route.path}`,
      );
    } else {
      check(route.replacement === null, `unexpected route replacement: ${client.id}${route.path}`);
    }
    seenRoutes.add(route.path);
  }
}

for (const client of manifest.clients.filter((candidate) => candidate.id !== "admin-web")) {
  const featureDirectory = join(root, client.root, "features");
  const actual = sorted(
    readdirSync(featureDirectory, { withFileTypes: true })
      .filter((entry) => entry.isDirectory())
      .map((entry) => entry.name),
  );
  const classified = client.surfaces.flatMap((surface) => surface.featureRoots);
  check(
    new Set(classified).size === classified.length,
    `${client.id} feature root has multiple surface owners`,
  );
  check(same(sorted(classified), actual), `${client.id} feature-root coverage drifted`);
}

const counts = Object.fromEntries(
  Object.keys(manifest.actions).map((action) => [
    action,
    [...allSurfaces.values()].filter((surface) => surface.action === action).length,
  ]),
);
console.log(
  `GUI surface contract passed: ${allSurfaces.size} surfaces, ${Object.values(requiredRoutes).flat().length} routes (${Object.entries(counts)
    .map(([action, count]) => `${action}=${count}`)
    .join(", ")})`,
);
