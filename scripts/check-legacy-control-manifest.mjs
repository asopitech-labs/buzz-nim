#!/usr/bin/env node

import { existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";

const manifest = JSON.parse(
  readFileSync("contracts/nimino-legacy-control/v1/manifest.json", "utf8"),
);

function check(condition, message) {
  if (!condition) throw new Error(message);
}

check(manifest.schemaVersion === 1 && manifest.version === 1, "wrong manifest version");
check(manifest.contract === "nimino.legacy-control-replacement", "wrong contract");
check(manifest.compatibilityMode === false, "compatibility mode must stay disabled");
check(manifest.publicationState === "removed", "legacy control must remain removed");
check(manifest.physicalRemovalOwner === 66, "#66 must own physical removal");

for (const path of manifest.retiredPaths) {
  check(!existsSync(path), `retired path returned: ${path}`);
}
for (const path of manifest.targetPaths) {
  check(existsSync(path), `target path is missing: ${path}`);
}
for (const cache of manifest.optionalLocalCaches) {
  check(cache.authoritative === false, `${cache.surface} became authoritative`);
  check(cache.scope === "single-node", `${cache.surface} escaped node-local scope`);
}

const files = execFileSync("git", ["ls-files", "-co", "--exclude-standard"], {
  encoding: "utf8",
})
  .trim()
  .split("\n")
  .filter(Boolean)
  .filter((path) =>
    /^(Cargo\.toml|crates\/.*\.(rs|toml)|deploy\/.*\.(ya?ml|tpl|sh)|\.github\/workflows\/.*\.yml)$/.test(
      path,
    ),
  )
  .filter(existsSync);

for (const symbol of manifest.forbiddenRuntimeSymbols) {
  const leaks = files.filter((path) => readFileSync(path, "utf8").includes(symbol));
  check(leaks.length === 0, `retired symbol ${symbol} returned: ${leaks.join(", ")}`);
}

check(
  manifest.ownerReview.status === "implemented-by-issue-66",
  "owner review state must describe the completed cut",
);

console.log(
  `Legacy control removal verified: ${manifest.retiredPaths.length} paths absent, ${manifest.targetPaths.length} targets present`,
);
