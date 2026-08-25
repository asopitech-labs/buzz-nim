# Mobile removal inventory

Status: accepted for the Nimino v1.0 cutover. Implements Issue #18.

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
| `mobile/**` source, tests, assets, toolchains | delete | #33 | `test ! -d mobile` and no tracked Mobile path |
| Mobile release workflows/scripts/hooks/docs | delete/shrink | #33 (primary; #65 dependency) | legacy-reference and workflow inventory gates |
| APNs push gateway, queue/schema, image, chart | delete | #33 | no APNs credentials, runtime, migration, artifact, or deployment remains |
| Desktop Mobile pairing wording/routes | shrink | #28 | Desktop sender/receiver E2E with no Mobile wording or dead route |
| NIP-AB protocol, CLI, bounded ephemeral relay | keep | protocol adapters; product owner #28 | vectors, state machine, interop, relay-limit tests |
| iOS/Android media-origin fixtures | keep | `buzz-media` | sanitizer tests; these are input compatibility, not product clients |
| Tauri Android/iOS packaging icons | delete | #33 | desktop bundles pass without the unused directories |

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
- #33 deletes the Mobile tree, Mobile push stack, toolchain, scripts,
  workflows, docs, and unused platform assets in one cutover-ready patch.
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
owner for every match. New or overlapping references fail CI.

Issue #33 changes the manifest phase from `inventory` to `removed`, updates the
reviewed counts, and keeps this gate in CI. In that phase the gate requires zero
`mobile/**` files and rejects every path selected by a `delete` surface; deleted
evidence is no longer required. This turns the inventory into a permanent
zero-state deny gate instead of breaking or being retired at cutover.
