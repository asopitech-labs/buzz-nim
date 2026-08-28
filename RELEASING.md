# Releasing Nimino

Nimino has one release authority: an immutable `nimino.release-set`. Relay,
Desktop, WSL, Chirps, Nim core, and data evidence do not version or promote
independently.

| Authority | Value |
|---|---|
| Source repository | `asopitech-labs/nimino` |
| Candidate tag | `nimino-vX.Y.Z` |
| Promotion input | 64-hex `releaseSetId` |
| Rolling updater release | `nimino-desktop-latest` |
| Human approval | protected `nimino-production` environment |

This repository is a hard fork. There is no `desktop-v`, `relay-v`, `chart-v`,
Block/Square signing path, Buzz compatibility release, or native Windows
installer in the Nimino release contract.

The predecessor workflow/script/credential/external-authority disposition is
frozen in `contracts/nimino-legacy-release/v1/manifest.json`. Its operator
checklist is classification evidence for the atomic #66 deletion, not another
release procedure. This file remains the sole release runbook.

## Supported artifacts

| Surface | Platform | Artifact/signing |
|---|---|---|
| Relay | Linux amd64 + arm64 | OCI image, Helm chart, Compose; keyless Sigstore |
| Desktop | macOS arm64 | Apple signed/notarized DMG + signed Tauri updater |
| Desktop | macOS x86_64 | Apple signed/notarized DMG + signed Tauri updater |
| Desktop | Linux x86_64 | AppImage + signed Tauri updater |
| Windows host | WSL2 x86_64 only | qualified Ubuntu 24.04 bundle; keyless Sigstore |

The WSL bundle includes the relay and its matching `nimino-core-worker` plus
CLI, ACP, agent, MCP, and data operations tools. Native Windows shells are not
a supported execution or release surface.

## Candidate lifecycle

Pushing an immutable `nimino-vX.Y.Z` tag starts
`.github/workflows/nimino-platform-release.yml`. Issue #68 owns that physical
cutover action; ordinary development must not create the tag.

The workflow:

1. verifies the tag and canonical repository;
2. calls the reusable relay image/chart/Compose build;
3. fails before Desktop builds when any required updater or Apple signing
   credential is missing;
4. builds the fixed Desktop and WSL matrix;
5. combines every component under one `release-set.json`;
6. qualifies the real WSL environment and complete bundle;
7. generates a deterministic SPDX 2.3 SBOM and complete `SHA256SUMS`;
8. adds keyless blob signatures, GitHub provenance, and SBOM attestations;
9. uploads the result as a draft GitHub Release.

The release-set intentionally contains the WSL component files rather than its
derived archive: the archive embeds the same release-set ID and therefore
cannot contain its own digest. `latest.json` and the WSL bundle both carry that
ID and are separately signed and checksummed.

## Promotion

Promotion is manual and accepts only a release-set ID:

```sh
gh workflow run nimino-promote.yml \
  --repo asopitech-labs/nimino \
  --ref main \
  -f release_set_id=<64-hex-id>
```

The protected `nimino-production` environment must require owner approval.
Before any write, the workflow resolves the ID to exactly one candidate and
checks:

- tag, release target, source commit, and release-set identity;
- every artifact digest and the complete, sorted checksum inventory;
- SPDX content against the release-set;
- keyless signatures for release-set, SBOM, updater, WSL, and checksums;
- GitHub provenance and SPDX SBOM attestations;
- the exact Desktop/WSL platform matrix and canonical asset URLs;
- WSL qualification evidence;
- same-version identity and monotonic version rules.

The immutable candidate is published before the rolling updater changes. The
rolling release contains only `latest.json` and `promotion.json`. Updating them
is guarded by the prior promotion record. If the second write or read-back
verification fails, both files are restored to the last known-good bytes. A
same-release-set retry is a no-op only when bytes match; same-version changes
and downgrades are rejected.

## Rollback policy

There are two different rollback cases:

- A partial promotion write is rolled back automatically to the last
  `promotion.json` authority, and the workflow fails.
- A bad release already consumed by clients is not downgraded in place. Stop
  deployment rollout, retain the prior immutable release-set for server
  rollback, then build and promote a higher patch release for clients.

Never move a `nimino-v*` tag, replace immutable release assets, or edit a
release-set. Rebuild under a new version when any artifact byte must change.

## Required repository configuration

Configure these secrets before the first candidate:

| Secret | Purpose |
|---|---|
| `NIMINO_UPDATER_PUBLIC_KEY` | Public Tauri updater key embedded in Desktop |
| `TAURI_SIGNING_PRIVATE_KEY` | Detached updater signatures |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Private-key password when encrypted |
| `NIMINO_APPLE_CERTIFICATE` | Base64 Apple Developer ID certificate |
| `NIMINO_APPLE_CERTIFICATE_PASSWORD` | Certificate password |
| `NIMINO_APPLE_SIGNING_IDENTITY` | Exact Developer ID identity |
| `NIMINO_APPLE_API_KEY_ID` | App Store Connect API key ID |
| `NIMINO_APPLE_API_ISSUER` | App Store Connect issuer ID |
| `NIMINO_APPLE_API_KEY_P8` | Notarization API private key |

Also provide a self-hosted runner with labels
`self-hosted, linux, x64, nimino-wsl-ubuntu-24.04` matching
`contracts/wsl-support-v1.json`, and protect the `nimino-production`
environment with required reviewers.

## Local release checks

```sh
just release-set-contract
just relay-release-contract
just platform-release-contract
just promotion-contract
just wsl-bundle-e2e
```

`just wsl-bundle-certify` is stricter than the portable E2E and succeeds only
inside the exact supported WSL candidate. Do not waive failed host, WSL,
systemd, mirrored-network, ext4, WSLg, or Secret Service checks.

## First physical cutover

Issues #64–#67 must freeze backup/rollback evidence, delete predecessor
pipelines and product paths, and pass clean-clone plus compatibility-negative
certification. Issue #68 alone creates and promotes the first tag, then records
post-cutover smoke evidence. Until those gates pass, the repository is
cutover-ready but not released.
