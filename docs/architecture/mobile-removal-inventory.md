# Mobile removal inventory

Status: Mobile product lane removed by Issue #33; retained follow-up surfaces
remain owned by Issues #20 and #28.

## Decision

Nimino has no smartphone application. The complete Flutter, iOS, Android,
APNs push, Mobile build, and Mobile release surfaces are deletion targets.
Issue #33 owns their atomic physical removal; no archive, placeholder, or
disabled lane remains.

NIP-AB remains for one use case only: an authorized Nimino Desktop transfers
its identity to a fresh Nimino Desktop during recovery, with explicit SAS
confirmation. Issue #28 owns both Desktop roles and removes every Mobile name,
instruction, and dead route. NIP-AB does not become cluster sync, background
device coordination, or a Mobile compatibility mode.

The machine-readable source of truth is
[`contracts/mobile-removal-v1.json`](../../contracts/mobile-removal-v1.json).

## Lifecycle summary

| Surface | Action | Owner | Completion proof |
|---|---|---|---|
| `mobile/**` source, tests, assets, toolchains | removed | #33 | `test ! -d mobile` and no tracked Mobile path |
| Mobile release workflows/scripts/hooks/docs | removed/shrunk | #33 (primary; #65 dependency) | legacy-reference and workflow inventory gates |
| APNs push gateway, live queue/schema, image, chart | removed | #33 | additive teardown leaves no live tables, credentials, runtime, artifact, or deployment |
| Applied push migration ledger | keep until reset | #33 / #12 | immutable checksums plus migration 0033 teardown |
| Desktop Mobile pairing wording/routes | pending shrink | #28 | Desktop sender/receiver E2E with no Mobile wording or dead route |
| NIP-AB protocol, CLI, bounded ephemeral relay | keep | protocol adapters; product owner #28 | vectors, state machine, interop, relay-limit tests |
| iOS/Android media-origin fixtures | keep | `nimino-media` | sanitizer tests; these are input compatibility, not product clients |
| Tauri Android/iOS packaging icons | removed | #33 | desktop bundles pass without the unused directories |

## NIP-AB ownership

The source is an already-authorized Desktop. The target is a fresh Desktop in
identity-recovery onboarding. Both display and confirm the SAS before the one-
time identity transfer. The pairing relay remains an opaque, short-lived NIP-01
transport. It never owns identity policy or persistent state.

The following are unsupported and must have no reachable surface:

- smartphone onboarding or a Nimino Mobile client;
- Mobile push notifications or Mobile release artifacts;
- repeated/automatic key transfer or multi-device coordination;
- using NIP-AB for Chirps negotiation, DB replication, or automatic sync.

## Cutover boundaries

- #28 changes the Desktop wording/routes and proves both non-Mobile Desktop
  roles. It does not redesign NIP-AB cryptography.
- #33 deletes the Mobile tree, Mobile push runtime, toolchain, scripts,
  workflows, docs, and unused platform assets, then drops its database state
  through an additive migration. Applied migration history remains immutable.
- #65 removes the obsolete upstream release authority from shared runbooks.
- #12 performs the incompatible public cutover; it does not preserve a Mobile
  compatibility mode.

## Verification

```bash
. ./bin/activate-hermit
just removed-client-contract
just ci-lanes-contract
just naming-contract
```

The contract gate obtains the tracked file list, classifies every current file
under `mobile/` exactly once, scans tracked paths and text for Mobile, pairing,
Flutter, APNs, and push-gateway references, and requires exactly one lifecycle
owner for every match. Retired push tokens are also pinned to exact per-file
occurrence counts in migration history, teardown/schema evidence, negative
tests, and changelogs. New, removed, or moved occurrences fail CI.

The manifest is now in `removed` phase. The gate requires zero `mobile/**`
files, rejects every `delete` selector, and verifies completed shrink evidence
is gone. Pending shrink surfaces owned by #20 and #28 must retain their current
and post-removal evidence until those issues complete. The gate therefore remains
a permanent zero-state policy without claiming unfinished follow-up work.
