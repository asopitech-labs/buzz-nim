#!/usr/bin/env node

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = process.cwd();
const cli = join(root, "scripts/nimino-promote-release.mjs");
const releaseSetCli = join(root, "scripts/nimino-release-set.mjs");
const supplyCli = join(root, "scripts/nimino-release-supply-chain.mjs");
const platform = JSON.parse(
  readFileSync("contracts/nimino-platform-release/v1/contract.json", "utf8"),
);
const bundle = JSON.parse(
  readFileSync("contracts/nimino-wsl-bundle/v1/contract.json", "utf8"),
);
const promotion = JSON.parse(
  readFileSync("contracts/nimino-promotion/v1/contract.json", "utf8"),
);
const releaseSetContract = JSON.parse(
  readFileSync("contracts/nimino-release-set/v1/contract.json", "utf8"),
);
const promotionWorkflow = readFileSync(
  ".github/workflows/nimino-promote.yml",
  "utf8",
);
const candidateWorkflow = readFileSync(
  ".github/workflows/nimino-platform-release.yml",
  "utf8",
);
const work = mkdtempSync(join(tmpdir(), "nimino-promotion-test-"));
const mockBin = join(work, "bin");
const state = join(work, "state");
mkdirSync(mockBin);
mkdirSync(state);
const commit = spawnSync("git", ["rev-parse", "HEAD"], {
  cwd: root,
  encoding: "utf8",
}).stdout.trim();

assert.equal(promotion.schemaVersion, 1);
assert.equal(promotion.contract, "nimino.promotion");
assert.equal(promotion.issue, 63);
assert.equal(promotion.releaseSetAuthority, releaseSetContract.contract);
assert.equal(promotion.compatibilityMode, false);
assert.equal(promotion.promotionInput, releaseSetContract.promotionInput);
assert.equal(promotion.downgradePolicy, "reject");
assert.equal(promotion.partialFailurePolicy, "restore-last-known-good");
assert.equal(promotion.physicalCutoverOwner, 68);
assert.equal(promotion.legacyPipelineDeletionOwner, 65);
for (const signal of [
  "release_set_id:",
  "environment: nimino-production",
  "github.repository == 'asopitech-labs/nimino'",
  "attestations: read",
  "nimino-promote-release.mjs resolve",
  "nimino-promote-release.mjs promote",
])
  assert.ok(
    promotionWorkflow.includes(signal),
    `missing promotion workflow signal: ${signal}`,
  );
assert.ok(!promotionWorkflow.includes("inputs.version"));
assert.ok(!promotionWorkflow.includes("block/buzz"));
for (const signal of [
  "nimino-release-supply-chain.mjs sbom",
  "nimino-release-supply-chain.mjs checksums",
  "actions/attest@1e69f48acb82d1966a394da916b4c1698aa569d6",
  "subject-checksums: candidate/SHA256SUMS",
  "sbom-path: candidate/nimino-release.spdx.json",
  "needs: finalize",
])
  assert.ok(
    candidateWorkflow.includes(signal),
    `missing supply-chain pipeline signal: ${signal}`,
  );

function digest(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function node(program, parameters, env = {}) {
  return spawnSync(process.execPath, [program, ...parameters], {
    cwd: root,
    encoding: "utf8",
    env: {
      ...process.env,
      PATH: `${mockBin}:${process.env.PATH}`,
      GITHUB_REPOSITORY: "asopitech-labs/nimino",
      GH_TOKEN: "test-token",
      MOCK_STATE_DIR: state,
      MOCK_SOURCE_COMMIT: commit,
      ...env,
    },
  });
}

function fixture(version, label) {
  const directory = join(work, `candidate-${version}`);
  mkdirSync(directory);
  const specifications = [];
  for (const component of bundle.components) {
    const path = join(directory, component.installName);
    writeFileSync(path, `${label}:${component.artifactId}\n`);
    specifications.push({ id: component.artifactId, path });
  }
  for (const selected of platform.platforms)
    for (const artifact of selected.artifacts) {
      const fileName = artifact.fileName.replace("{version}", version);
      const path = join(directory, fileName);
      writeFileSync(path, `${label}:${selected.id}:${artifact.role}\n`);
      specifications.push({
        id: `desktop-${selected.id}-${artifact.role}`,
        path,
      });
      if (artifact.signature) {
        writeFileSync(`${path}.sig`, `${label}-${selected.id}-signature\n`);
        specifications.push({
          id: `desktop-${selected.id}-${artifact.role}-signature`,
          path: `${path}.sig`,
        });
      }
    }
  const releaseSet = join(directory, "release-set.json");
  const created = node(releaseSetCli, [
    "create",
    "--version",
    version,
    "--tag",
    `nimino-v${version}`,
    "--commit",
    commit,
    "--output",
    releaseSet,
    ...specifications.flatMap(({ id, path }) => [
      "--artifact",
      `${id}:${version}:${path}`,
    ]),
  ]);
  assert.equal(created.status, 0, created.stderr);
  const releaseSetValue = JSON.parse(readFileSync(releaseSet, "utf8"));
  const base = `https://github.com/asopitech-labs/nimino/releases/download/nimino-v${version}`;
  const platforms = Object.fromEntries(
    platform.platforms.map((selected) => {
      const updater = selected.artifacts.find(({ signature }) => signature);
      const fileName = updater.fileName.replace("{version}", version);
      return [
        selected.updaterKey,
        {
          signature: readFileSync(
            join(directory, `${fileName}.sig`),
            "utf8",
          ).trim(),
          url: `${base}/${fileName}`,
        },
      ];
    }),
  );
  const wslName = platform.wsl.archive.replace("{version}", version);
  const wslPath = join(directory, wslName);
  writeFileSync(wslPath, `${label}:wsl archive\n`);
  writeFileSync(
    join(directory, "latest.json"),
    `${JSON.stringify(
      {
        version,
        notes: `Nimino ${version}`,
        pub_date: "1970-01-01T00:00:00.000Z",
        releaseSetId: releaseSetValue.releaseSetId,
        platforms,
        wsl: {
          platform: platform.wsl.id,
          url: `${base}/${wslName}`,
          sha256: digest(wslPath),
        },
      },
      null,
      2,
    )}\n`,
  );
  for (const name of [
    "release-set.sigstore.json",
    "nimino-release.spdx.sigstore.json",
    "latest.sigstore.json",
    "nimino-wsl.sigstore.json",
  ])
    writeFileSync(join(directory, name), `${label}:${name}\n`);
  assert.equal(
    node(supplyCli, [
      "sbom",
      "--release-set",
      releaseSet,
      "--output",
      join(directory, "nimino-release.spdx.json"),
    ]).status,
    0,
  );
  writeFileSync(
    join(directory, "nimino-wsl-bundle-e2e.json"),
    `${JSON.stringify({ result: "passed", releaseSetId: releaseSetValue.releaseSetId })}\n`,
  );
  assert.equal(
    node(supplyCli, [
      "checksums",
      "--directory",
      directory,
      "--output",
      join(directory, "SHA256SUMS"),
    ]).status,
    0,
  );
  writeFileSync(
    join(directory, "SHA256SUMS.sigstore.json"),
    "checksum signature\n",
  );
  return {
    directory,
    releaseSet: releaseSetValue,
    assets: readdirSync(directory).sort(),
  };
}

writeFileSync(
  join(mockBin, "cosign"),
  `#!/usr/bin/env bash
set -euo pipefail
[[ "\${1:-}" == verify-blob ]]
`,
);
chmodSync(join(mockBin, "cosign"), 0o755);
writeFileSync(
  join(mockBin, "gh"),
  `#!/usr/bin/env bash
set -euo pipefail
command="$1 $2"
case "$command" in
  "release list")
    draft=true
    marker="$MOCK_STATE_DIR/published-$(basename "$MOCK_CANDIDATE_DIR")"
    [[ ! -f "$marker" ]] || draft=false
    printf '[{"tagName":"%s","isDraft":%s,"isPrerelease":false}]\n' "$MOCK_TAG" "$draft"
    ;;
  "release view")
    tag="$3"
    if [[ "$tag" == "$MOCK_TAG" ]]; then
      draft=true
      [[ ! -f "$MOCK_STATE_DIR/published-$(basename "$MOCK_CANDIDATE_DIR")" ]] || draft=false
      printf '{"isDraft":%s,"isPrerelease":false,"targetCommitish":"%s","assets":%s}\n' "$draft" "$MOCK_SOURCE_COMMIT" "$MOCK_ASSETS_JSON"
    elif [[ "$tag" == nimino-desktop-latest && -f "$MOCK_STATE_DIR/stable-exists" ]]; then
      assets='[]'
      if [[ -f "$MOCK_STATE_DIR/latest.json" && -f "$MOCK_STATE_DIR/promotion.json" ]]; then
        assets='[{"name":"latest.json"},{"name":"promotion.json"}]'
      fi
      printf '{"assets":%s}\n' "$assets"
    else
      exit 1
    fi
    ;;
  "release download")
    tag="$3"; shift 3; pattern= directory=
    while [[ $# -gt 0 ]]; do
      case "$1" in
        --pattern) pattern="$2"; shift 2 ;;
        --dir) directory="$2"; shift 2 ;;
        *) shift ;;
      esac
    done
    mkdir -p "$directory"
    source="$MOCK_CANDIDATE_DIR"
    [[ "$tag" != nimino-desktop-latest ]] || source="$MOCK_STATE_DIR"
    if [[ -n "$pattern" ]]; then cp "$source/$pattern" "$directory/$pattern"
    else cp "$source"/* "$directory/"
    fi
    ;;
  "release edit")
    touch "$MOCK_STATE_DIR/published-$(basename "$MOCK_CANDIDATE_DIR")"
    ;;
  "release create")
    touch "$MOCK_STATE_DIR/stable-exists"
    ;;
  "release upload")
    shift 3
    while [[ $# -gt 0 && "$1" != --repo ]]; do
      name="$(basename "$1")"
      failure_marker="$MOCK_STATE_DIR/upload-failed-$(basename "$MOCK_CANDIDATE_DIR")"
      if [[ "$name" == promotion.json && "\${MOCK_FAIL_PROMOTION_UPLOAD:-}" == once && ! -f "$failure_marker" ]]; then
        touch "$failure_marker"
        exit 9
      fi
      cp "$1" "$MOCK_STATE_DIR/$name"
      shift
    done
    ;;
  "release delete")
    rm -f "$MOCK_STATE_DIR/stable-exists" "$MOCK_STATE_DIR/latest.json" "$MOCK_STATE_DIR/promotion.json"
    ;;
  "attestation verify")
    [[ "\${MOCK_FAIL_ATTESTATION:-}" != true ]]
    ;;
  "api repos/asopitech-labs/nimino/commits/"*)
    printf '%s\n' "$MOCK_SOURCE_COMMIT"
    ;;
  *)
    echo "unexpected gh command: $*" >&2
    exit 70
    ;;
esac
`,
);
chmodSync(join(mockBin, "gh"), 0o755);

function environment(candidate, extra = {}) {
  return {
    MOCK_CANDIDATE_DIR: candidate.directory,
    MOCK_TAG: candidate.releaseSet.tag,
    MOCK_ASSETS_JSON: JSON.stringify(
      candidate.assets.map((name) => ({ name })),
    ),
    ...extra,
  };
}

function promote(candidate, extra = {}) {
  return node(
    cli,
    [
      "promote",
      "--release-set-id",
      candidate.releaseSet.releaseSetId,
      "--tag",
      candidate.releaseSet.tag,
    ],
    environment(candidate, extra),
  );
}

const first = fixture("1.2.3", "first");
const resolved = join(work, "resolved.json");
assert.notEqual(
  node(
    cli,
    [
      "resolve",
      "--release-set-id",
      first.releaseSet.releaseSetId,
      "--output",
      resolved,
    ],
    environment(first, { GH_TOKEN: "" }),
  ).status,
  0,
);
assert.equal(
  node(
    cli,
    [
      "resolve",
      "--release-set-id",
      first.releaseSet.releaseSetId,
      "--output",
      resolved,
    ],
    environment(first),
  ).status,
  0,
);
assert.equal(JSON.parse(readFileSync(resolved, "utf8")).tag, "nimino-v1.2.3");
assert.notEqual(promote(first, { MOCK_FAIL_ATTESTATION: "true" }).status, 0);
assert.notEqual(
  promote(first, { MOCK_FAIL_PROMOTION_UPLOAD: "once" }).status,
  0,
);
assert.equal(existsSync(join(state, "stable-exists")), false);
assert.equal(promote(first).status, 0);
const firstRecord = readFileSync(join(state, "promotion.json"), "utf8");
const firstLatest = readFileSync(join(state, "latest.json"), "utf8");
assert.equal(
  JSON.parse(firstRecord).current.releaseSetId,
  first.releaseSet.releaseSetId,
);
assert.equal(promote(first).status, 0);
assert.equal(readFileSync(join(state, "promotion.json"), "utf8"), firstRecord);

const second = fixture("1.2.4", "second");
const failed = promote(second, { MOCK_FAIL_PROMOTION_UPLOAD: "once" });
assert.notEqual(failed.status, 0);
assert.equal(readFileSync(join(state, "promotion.json"), "utf8"), firstRecord);
assert.equal(readFileSync(join(state, "latest.json"), "utf8"), firstLatest);

const older = fixture("1.2.2", "older");
const downgraded = promote(older);
assert.notEqual(downgraded.status, 0);
assert.match(downgraded.stderr, /downgrade rejected/);

console.log(
  "Nimino promotion tests passed: missing auth, provenance, retry, downgrade, rollback",
);
