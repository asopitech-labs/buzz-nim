#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const manifest = JSON.parse(
  readFileSync(join(root, "contracts", "nimino-naming-v1.json"), "utf8"),
);
const surface = (id) => manifest.surfaces.find((entry) => entry.id === id);
const check = (condition, message) => {
  if (!condition) throw new Error(message);
};

check(manifest.compatibilityAliases === false, "runtime aliases must be disabled");

const cliCargo = readFileSync(join(root, "crates", "buzz-cli", "Cargo.toml"), "utf8");
const relayCargo = readFileSync(join(root, "crates", "buzz-relay", "Cargo.toml"), "utf8");
check(
  cliCargo.includes(`[[bin]]\nname = "${surface("binary.cli").canonical}"`),
  "CLI binary is not Nimino",
);
check(
  relayCargo.includes(`[[bin]]\nname = "${surface("binary.relay").canonical}"`),
  "relay binary is not Nimino",
);

const tauri = JSON.parse(
  readFileSync(join(root, "desktop", "src-tauri", "tauri.conf.json"), "utf8"),
);
check(
  tauri.identifier === surface("desktop.bundle-identifier").canonical,
  "Desktop bundle identifier is not Nimino",
);
check(
  JSON.stringify(tauri.plugins?.["deep-link"]?.desktop?.schemes) ===
    JSON.stringify([surface("url.deep-link-scheme").canonical]),
  "Desktop deep-link scheme is not Nimino-only",
);

const excluded = new Set([
  "CHANGELOG.md",
  "contracts/fixtures/old-buzz-client-v1.json",
  "contracts/nimino-naming-v1.json",
  "contracts/nimino-protocol-v1.json",
  "crates/buzz-relay/CHANGELOG.md",
  "docs/adr/nim-rust-boundary-v1.md",
  "scripts/check-nimino-naming-contract.mjs",
  "scripts/check-nimino-protocol-contract.mjs",
  "scripts/check-nimino-runtime-namespace.mjs",
  "scripts/check-mobile-removal-contract.mjs",
]);
const files = execFileSync("git", ["ls-files", "-z"], { cwd: root })
  .toString()
  .split("\0")
  .filter((path) => path && !excluded.has(path));

const legacyRuntimePatterns = [
  [/(^|[^A-Z0-9_])BUZZ_[A-Z0-9_]+/m, "legacy environment prefix"],
  [/buzz:\/\//, "legacy deep-link scheme"],
  [/buzz:\\\/\\\//, "legacy escaped deep-link scheme"],
  [/(?:url\.protocol|[A-Z_]*LINK_SCHEME)[^\n]*"buzz:"/, "legacy deep-link protocol"],
  [/buzz-media:\/\//, "legacy media scheme"],
  [/["`]buzz-desktop(?:[: ])/, "legacy Desktop log namespace"],
  [/["`]buzz-tray["`]/, "legacy Desktop tray identifier"],
  [/metrics::(?:counter|gauge|histogram)!\(\s*"buzz_/m, "legacy metric prefix"],
  [/postgres:\/\/buzz(?::|@)/, "legacy database identity"],
  [
    /(?:POSTGRES_USER|PGUSER|PGSCHEMA_PLAN_USER|NIMINO_DB_USER)[^\n]*\bbuzz\b/,
    "legacy database user",
  ],
  [
    /(?:POSTGRES_DB|PGDATABASE|PGSCHEMA_PLAN_DB|NIMINO_DB_NAME)[^\n]*\bbuzz\b/,
    "legacy database name",
  ],
  [/\bbuzz_dev(?:_secret|_key)?\b/, "legacy development credential"],
  [/NIMINO_S3_BUCKET[^\n]*buzz-media/, "legacy object bucket"],
  [
    /["`]buzz:(?:text-scale|last-runtime(?:-provider)?|observer-archive-default-seeded|onboarding-welcome-(?:channel|initial-unread-suppression)\.v1)(?=["`:$])/, 
    "legacy colon preference",
  ],
  [
    /["`]buzz\.(?:appearance|channels|projects|desktop|quick-reaction-emojis|mesh-compute|machine-onboarding|channel-read-state|nip-rs)\./,
    "legacy dotted preference",
  ],
  [
    /["`]buzz-(?:theme|communities|active-community-id|channel-mutes\.v1|home-feed-(?:done|seen)\.v1|onboarding-complete\.v1|community-onboarding-(?:complete|transaction)\.v1|community-discovery-after-leave|machine-onboarding-complete\.v2|e2e-project-branches|terminal-dock-height|thread-(?:participation|authored|muted|activity)\.v1|channel-messages\.v1|channels\.v1|observed-unread\.v1|(?:sidebar|timeline)-skeleton-shape\.v1|user-labels\.v1|local-storage-quota-recovery\.v1|feature-overrides-v\d+|community-join-seen\.v1|sync-watermark\.v1|channel-sections\.v1|self-profile\.v1|workspaces|active-workspace-id|drafts\.v[12]|community-destinations|community-icons|glass-(?:background|opacity)|prominent-active-tab|accent-color|sidebar-width|welcome-channel-ensured\.v2|forced-unread\.v1|channel-(?:stars|sort)\.v1|notification-settings\.v2)(?=["`:$])/, 
    "legacy hyphen preference",
  ],
];

for (const path of files) {
  const contents = readFileSync(join(root, path), "utf8");
  for (const [pattern, label] of legacyRuntimePatterns) {
    check(!pattern.test(contents), `${label} remains in ${path}`);
  }
}

const keyring = readFileSync(
  join(root, "desktop", "src-tauri", "src", "app_state_keyring.rs"),
  "utf8",
);
check(!keyring.includes('"buzz-desktop'), "legacy Desktop keyring service remains");

const desktopMain = readFileSync(join(root, "desktop", "src", "main.tsx"), "utf8");
check(
  !desktopMain.includes("migrateLegacyCommunityStorageBeforeRender"),
  "legacy Desktop storage is still read during startup",
);

const desktopRustMain = readFileSync(
  join(root, "desktop", "src-tauri", "src", "lib.rs"),
  "utf8",
);
check(
  !desktopRustMain.includes("migration::migrate_legacy_nest()"),
  "legacy Desktop nest is still read during startup",
);

const desktopMigrations = readFileSync(
  join(root, "desktop", "src-tauri", "src", "migration.rs"),
  "utf8",
);
check(
  !desktopMigrations.includes("\n    migrate_legacy_app_data_dir(app);"),
  "legacy Desktop app data is still read during startup",
);

const devSetup = readFileSync(join(root, "scripts", "dev-setup.sh"), "utf8");
check(
  !devSetup.includes("postgres://sprout:"),
  "legacy database defaults are still translated during setup",
);

const secretStore = readFileSync(
  join(root, "desktop", "src-tauri", "src", "secret_store.rs"),
  "utf8",
);
check(
  secretStore.includes(surface("storage.lock-prefix").canonical),
  "Desktop lock prefix is not Nimino",
);
check(
  !secretStore.includes(surface("storage.lock-prefix").legacy[0]),
  "legacy Desktop lock prefix remains",
);

const relayConfig = readFileSync(
  join(root, "crates", "buzz-relay", "src", "config.rs"),
  "utf8",
);
check(
  relayConfig.includes(`/nimino`),
  "relay database default is not Nimino",
);
check(!relayConfig.includes("postgres://buzz"), "legacy relay database default remains");

const chartValues = readFileSync(
  join(root, "deploy", "charts", "buzz", "values.yaml"),
  "utf8",
);
check(
  chartValues.includes("database: nimino") && chartValues.includes("username: nimino"),
  "Helm quickstart database identity is not Nimino",
);

const mediaConfig = readFileSync(
  join(root, "crates", "buzz-media", "src", "config.rs"),
  "utf8",
);
check(
  mediaConfig.includes(`"${surface("storage.object-bucket").canonical}"`),
  "media bucket default is not Nimino",
);

for (const path of [
  "crates/buzz-relay/src/main.rs",
  "crates/buzz-relay/src/router.rs",
  "crates/buzz-relay/src/telemetry.rs",
]) {
  check(
    !readFileSync(join(root, path), "utf8").includes('"buzz-relay"'),
    `legacy relay service name remains in ${path}`,
  );
}

console.log(`Nimino runtime namespace passed: ${files.length} tracked files`);
