#!/usr/bin/env node

import { createHash } from "node:crypto";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifestPath = join(root, "contracts", "nimino-naming-v1.json");

function check(condition, message) {
  if (!condition) throw new Error(message);
}

function same(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

function hasExactKeys(value, keys) {
  return same(Object.keys(value).sort(), [...keys].sort());
}

check(existsSync(manifestPath), "missing contracts/nimino-naming-v1.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));

const requiredSurfaceIds = [
  "binary.cli",
  "binary.relay",
  "desktop.bundle-identifier",
  "environment.prefix",
  "metrics.prefix",
  "package.cargo",
  "package.nim",
  "package.npm.desktop",
  "package.npm.family",
  "package.rust-crate",
  "product.display",
  "protocol.colon-tag-prefix",
  "protocol.namespace",
  "protocol.tag-prefix",
  "release.artifact-prefix",
  "release.container-repository.relay",
  "release.container-repository.sprig",
  "release.desktop-artifact-prefix",
  "release.helm-chart-repository",
  "release.helm-chart.relay",
  "release.rolling-tag",
  "repository.slug",
  "storage.config-directory",
  "storage.database",
  "storage.keyring-service",
  "storage.lock-prefix",
  "storage.object-bucket",
  "storage.preference-colon-prefix",
  "storage.preference-dot-prefix",
  "storage.preference-hyphen-prefix",
  "url.deep-link-scheme",
  "url.media-scheme",
];
const matchSemantics = {
  exact: {
    caseSensitive: true,
    boundary: "whole-value",
    replacement: "whole-value",
  },
  prefix: {
    caseSensitive: true,
    boundary: "start-of-value",
    replacement: "replace-prefix-preserve-suffix",
  },
  "oci-repository": {
    caseSensitive: true,
    boundary: "whole-repository-before-tag-or-sha256-digest",
    replacement: "replace-repository-preserve-tag-or-digest",
  },
};

check(manifest.schemaVersion === 1, "naming schemaVersion must be 1");
check(manifest.contract === "nimino.naming", "unexpected naming contract id");
check(manifest.owner === "asopitech-labs", "naming authority must be asopitech-labs");
check(manifest.compatibilityAliases === false, "Buzz compatibility aliases are forbidden");
check(same(manifest.matchSemantics, matchSemantics), "naming match semantics drifted");
check(Array.isArray(manifest.surfaces), "naming surfaces must be an array");

const surfaceIds = manifest.surfaces.map((surface) => surface.id);
check(new Set(surfaceIds).size === surfaceIds.length, "naming surface ids must be unique");
check(
  same([...surfaceIds].sort(), requiredSurfaceIds),
  `naming surface coverage drifted: ${surfaceIds.sort().join(",")}`,
);

function transform(surface, input) {
  for (const legacy of surface.legacy) {
    if (surface.match === "exact" && input === legacy) return surface.canonical;
    if (surface.match === "prefix" && input.startsWith(legacy)) {
      return `${surface.canonical}${input.slice(legacy.length)}`;
    }
    if (surface.match === "oci-repository" && input.startsWith(legacy)) {
      const suffix = input.slice(legacy.length);
      if (
        suffix === "" ||
        /^(?::[A-Za-z0-9_][A-Za-z0-9_.-]{0,127})?(?:@sha256:[a-f0-9]{64})?$/.test(suffix)
      ) {
        return `${surface.canonical}${suffix}`;
      }
    }
  }
  return null;
}

for (const surface of manifest.surfaces) {
  check(
    hasExactKeys(surface, ["id", "match", "canonical", "legacy", "fixtures"]),
    `unexpected fields for ${surface.id}`,
  );
  check(surface.match in matchSemantics, `invalid match for ${surface.id}`);
  check(
    typeof surface.canonical === "string" && surface.canonical.length > 0,
    `missing single canonical name for ${surface.id}`,
  );
  check(!/buzz/i.test(surface.canonical), `legacy Buzz name is canonical for ${surface.id}`);
  check(
    Array.isArray(surface.legacy) &&
      surface.legacy.length > 0 &&
      new Set(surface.legacy).size === surface.legacy.length &&
      surface.legacy.every(
        (value) => typeof value === "string" && value.length > 0 && /buzz/i.test(value),
      ),
    `invalid legacy names for ${surface.id}`,
  );
  check(
    Array.isArray(surface.fixtures) &&
      surface.fixtures.length > 0 &&
      new Set(surface.fixtures.map((fixture) => fixture.input)).size ===
        surface.fixtures.length,
    `missing unique fixtures for ${surface.id}`,
  );
  for (const fixture of surface.fixtures) {
    check(hasExactKeys(fixture, ["input", "output"]), `invalid fixture for ${surface.id}`);
    check(!/buzz/i.test(fixture.output), `legacy Buzz name remains in ${surface.id} output`);
    check(
      transform(surface, fixture.input) === fixture.output,
      `fixture does not follow ${surface.match} semantics for ${surface.id}: ${fixture.input}`,
    );
  }
  for (const legacy of surface.legacy) {
    check(
      surface.fixtures.some((fixture) =>
        surface.match === "exact"
          ? fixture.input === legacy
          : fixture.input.startsWith(legacy),
      ),
      `legacy value lacks a fixture for ${surface.id}: ${legacy}`,
    );
    check(transform(surface, `x${legacy}`) === null, `match boundary is loose for ${surface.id}`);
    if (surface.match === "oci-repository") {
      check(
        transform(surface, `${legacy}-other:main`) === null,
        `OCI repository boundary is loose for ${surface.id}`,
      );
    }
  }
}

const expectedFamilies = {
  "desktop.bundle-identifier": [
    { input: "xyz.block.buzz.app", output: "com.asopitech.nimino" },
    { input: "xyz.block.buzz.app.dev", output: "com.asopitech.nimino.dev" },
    {
      input: "xyz.block.buzz.app.dev.example",
      output: "com.asopitech.nimino.dev.example",
    },
  ],
  "storage.keyring-service": [
    { input: "buzz-desktop", output: "nimino-desktop" },
    { input: "buzz-desktop-dev", output: "nimino-desktop-dev" },
    { input: "buzz-desktop-dev.example", output: "nimino-desktop-dev.example" },
  ],
  "storage.preference-hyphen-prefix": [
    { input: "buzz-theme", output: "nimino-theme" },
    { input: "buzz-communities", output: "nimino-communities" },
    { input: "buzz-channel-mutes.v1", output: "nimino-channel-mutes.v1" },
    { input: "buzz-home-feed-done.v1", output: "nimino-home-feed-done.v1" },
  ],
  "protocol.colon-tag-prefix": [
    { input: "buzz:mention-snapshot", output: "nimino:mention-snapshot" },
    { input: "buzz:sent-from-thread", output: "nimino:sent-from-thread" },
    { input: "buzz:custom-emoji", output: "nimino:custom-emoji" },
  ],
  "release.container-repository.relay": [
    { input: "ghcr.io/block/buzz", output: "ghcr.io/asopitech-labs/nimino" },
    { input: "ghcr.io/block/buzz:main", output: "ghcr.io/asopitech-labs/nimino:main" },
    {
      input:
        "ghcr.io/block/buzz@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      output:
        "ghcr.io/asopitech-labs/nimino@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    },
  ],
  "release.container-repository.sprig": [
    {
      input: "ghcr.io/block/buzz-sprig",
      output: "ghcr.io/asopitech-labs/nimino-sprig",
    },
    {
      input:
        "ghcr.io/block/buzz-sprig:sha-deadbee@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      output:
        "ghcr.io/asopitech-labs/nimino-sprig:sha-deadbee@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    },
  ],
  "release.desktop-artifact-prefix": [
    { input: "Buzz.app", output: "Nimino.app" },
    { input: "Buzz.app.tar.gz", output: "Nimino.app.tar.gz" },
    {
      input: "Buzz_${VERSION}_aarch64.app.tar.gz",
      output: "Nimino_${VERSION}_aarch64.app.tar.gz",
    },
    {
      input: "Buzz_${VERSION}_aarch64-signed.dmg",
      output: "Nimino_${VERSION}_aarch64-signed.dmg",
    },
  ],
  "release.helm-chart.relay": [{ input: "buzz", output: "nimino" }],
  "release.helm-chart-repository": [
    {
      input: "oci://ghcr.io/block/buzz/charts",
      output: "oci://ghcr.io/asopitech-labs/nimino/charts",
    },
  ],
};
for (const [id, fixtures] of Object.entries(expectedFamilies)) {
  const surface = manifest.surfaces.find((candidate) => candidate.id === id);
  check(same(surface?.fixtures, fixtures), `${id} prod/dev/scoped family drifted`);
}

const npmInventory = [
  {
    path: "desktop/package.json",
    surfaceId: "package.npm.desktop",
    legacy: "buzz",
    canonical: "nimino-desktop",
  },
  {
    path: "package.json",
    surfaceId: "package.npm.family",
    legacy: "buzz-workspace",
    canonical: "nimino-workspace",
  },
  {
    path: "web/package.json",
    surfaceId: "package.npm.family",
    legacy: "buzz-web",
    canonical: "nimino-web",
  },
  {
    path: "admin-web/package.json",
    surfaceId: "package.npm.family",
    legacy: "buzz-admin-web",
    canonical: "nimino-admin-web",
  },
];
for (const item of npmInventory) {
  const current = JSON.parse(readFileSync(join(root, item.path), "utf8")).name;
  check(
    current === item.legacy || current === item.canonical,
    `unclassified npm package name in ${item.path}: ${current}`,
  );
  const surface = manifest.surfaces.find((candidate) => candidate.id === item.surfaceId);
  check(
    transform(surface, item.legacy) === item.canonical,
    `npm target mapping drifted for ${item.path}`,
  );
}

const expectedChirpsOccurrences = [
  { path: "Cargo.toml", values: ["alopex-chirps"] },
  { path: "Cargo.lock", values: ["alopex-chirps"] },
  {
    path: "crates/nimino-chirps/**",
    values: ["Alopex Chirps", "alopex-chirps", "alopex_chirps"],
  },
  { path: "contracts/chirps-v*.json", values: ["alopex-chirps"] },
  {
    path: "scripts/check-chirps-api-contract.mjs",
    values: ["alopex-chirps", "alopex_chirps"],
  },
  { path: "docs/development/nim-core.md", values: ["Alopex Chirps"] },
  { path: "docs/adr/nim-rust-boundary-v1.md", values: ["Alopex Chirps"] },
];
check(
  Array.isArray(manifest.allowedExternalNames) && manifest.allowedExternalNames.length === 1,
  "external-name allowlist must be exact",
);
const chirps = manifest.allowedExternalNames[0];
check(
  hasExactKeys(chirps, ["id", "owner", "names", "occurrences", "reason"]) &&
    chirps.id === "alopex-chirps" &&
    chirps.owner === "alopex-db" &&
    same(chirps.names, ["Alopex Chirps", "alopex-chirps", "alopex_chirps"]) &&
    same(chirps.occurrences, expectedChirpsOccurrences) &&
    typeof chirps.reason === "string" &&
    chirps.reason.length > 0,
  "Alopex Chirps allowlist must stay exact, scoped, and justified",
);

check(
  Array.isArray(manifest.allowedLegacyReferences) &&
    manifest.allowedLegacyReferences.length === 3,
  "legacy-reference allowlist must be exact",
);
const references = Object.fromEntries(
  manifest.allowedLegacyReferences.map((entry) => [entry.id, entry]),
);
check(
  Object.keys(references).length === 3 &&
    same(Object.keys(references).sort(), [
      "immutable-project-history",
      "naming-contract-self-reference",
      "upstream-legal-attribution",
    ]),
  "legacy-reference ids must be unique and exact",
);
for (const entry of manifest.allowedLegacyReferences) {
  check(
    hasExactKeys(entry, ["id", "category", "occurrences", "reason"]) &&
      typeof entry.reason === "string" &&
      entry.reason.length > 0,
    `invalid legacy-reference entry: ${entry.id}`,
  );
}
check(
  references["naming-contract-self-reference"].category === "contract" &&
    same(references["naming-contract-self-reference"].occurrences, [
      {
        path: "contracts/nimino-naming-v1.json",
        values: ["$generated-denylist"],
      },
      {
        path: "scripts/check-nimino-naming-contract.mjs",
        values: ["$generated-denylist"],
      },
    ]),
  "contract self-reference allowlist drifted",
);
check(
  references["upstream-legal-attribution"].category === "legal" &&
    same(references["upstream-legal-attribution"].occurrences, []),
  "legal allowlist drifted",
);
const history = references["immutable-project-history"];
check(
  history.category === "history" &&
    same(
      history.occurrences.map((occurrence) => occurrence.path),
      ["CHANGELOG.md", "docs/adr/nim-rust-boundary-v1.md"],
    ),
  "history allowlist paths must be exact",
);
for (const occurrence of history.occurrences) {
  check(
    same(occurrence.values, ["$generated-denylist"]) &&
      hasExactKeys(occurrence, ["path", "values", "boundary"]),
    `invalid history occurrence: ${occurrence.path}`,
  );
  const boundary = occurrence.boundary;
  check(
    hasExactKeys(boundary, ["kind", "bytes", "sha256"]) &&
      ["immutable-prefix", "immutable-suffix"].includes(boundary.kind) &&
      Number.isInteger(boundary.bytes) &&
      boundary.bytes > 0 &&
      /^[0-9a-f]{64}$/.test(boundary.sha256),
    "history boundary must be a byte-counted SHA-256 region",
  );
  const historyBytes = readFileSync(join(root, occurrence.path));
  check(boundary.bytes <= historyBytes.length, "history region exceeds the recorded file");
  const start = boundary.kind === "immutable-prefix" ? 0 : historyBytes.length - boundary.bytes;
  check(
    createHash("sha256")
      .update(historyBytes.subarray(start, start + boundary.bytes))
      .digest("hex") ===
      boundary.sha256,
    `immutable history region changed: ${occurrence.path}`,
  );
}

const denylist = new Map();
for (const surface of manifest.surfaces) {
  for (const value of surface.legacy) {
    const key = `${surface.match}\0${value}`;
    const entry = denylist.get(key) ?? {
      match: surface.match,
      value,
      surfaceIds: [],
    };
    entry.surfaceIds.push(surface.id);
    denylist.set(key, entry);
  }
}
const generated = {
  schemaVersion: 1,
  contract: "nimino.naming.denylist",
  matchSemantics,
  entries: [...denylist.values()]
    .map((entry) => ({ ...entry, surfaceIds: entry.surfaceIds.sort() }))
    .sort((left, right) =>
      `${left.match}:${left.value}`.localeCompare(`${right.match}:${right.value}`),
    ),
  allowedReferences: manifest.allowedLegacyReferences.map((entry) => ({
    ...entry,
    occurrences: entry.occurrences.map((occurrence) => ({
      ...occurrence,
      values: occurrence.values.flatMap((value) =>
        value === "$generated-denylist"
          ? [...new Set([...denylist.values()].map((item) => item.value))].sort()
          : value,
      ),
    })),
  })),
};
check(generated.entries.length > 0, "generated Buzz denylist must not be empty");

const args = process.argv.slice(2);
check(args.length <= 1 && [undefined, "--denylist"].includes(args[0]), "unknown argument");
if (args[0] === "--denylist") console.log(JSON.stringify(generated, null, 2));
else {
  console.log(
    `Nimino naming contract passed: ${manifest.surfaces.length} surfaces, ${generated.entries.length} deny rules`,
  );
}
