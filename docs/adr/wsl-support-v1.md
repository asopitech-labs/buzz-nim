# ADR: WSL support v1

Status: Accepted for the Nimino v1.0 cutover. Implements Issue #16.

## Decision summary

Nimino supports one Windows profile: Windows 11 build 26200 or newer on x86_64,
stable WSL 2.7.3 or newer, and the exact `Ubuntu-24.04` WSL2 distribution.
The Linux desktop runs through WSLg. A Windows-native desktop shell is not a
second supported mode.

The machine-readable source of truth is
[`contracts/wsl-support-v1.json`](../../contracts/wsl-support-v1.json). It also
contains executable negative cases for unsupported host, distro, GUI, storage,
network, and secret configurations.

## Supported matrix

| Layer | Supported v1 value |
|---|---|
| Windows host | Windows 11 build 26200+ (25H2 or newer), x86_64 |
| WSL | WSL2, stable channel, version 2.7.3+ |
| Distribution | Store identifier `Ubuntu-24.04`, Ubuntu 24.04 LTS, x86_64 |
| Service manager | systemd enabled |
| GUI | WSLg only; Wayland or WSLg-provided X11 compatibility |
| Live filesystem | Distribution ext4 under `/home/<user>` and XDG directories |
| Network | Mirrored mode; loopback for app-local traffic; explicit firewall rule for cluster ingress |
| Secret store | Linux Secret Service, present and unlocked, for the selected unprivileged distro user |

Versions below the stated minimums and alternatives not listed in this table
are unsupported. In particular, WSL1, Windows on ARM64, another distro,
unversioned `Ubuntu`, NAT mode, a third-party X server, and a Windows-native
shell are not compatibility targets.

Ubuntu 24.04 is deliberate: its WSL image enables systemd by default and the
existing Linux desktop artifacts already target Ubuntu 22.04 or newer. x86_64
is the only WSL release target until an ARM64 desktop artifact is independently
qualified.

## Use cases and completion conditions

| Use case | Activity | Completion condition |
|---|---|---|
| Launch the desktop | Select exact distro/user → start Linux process → WSLg presents its window | No Windows desktop binary or external X server is involved |
| Run local services | systemd user/service lifecycle → bind app-local endpoints to loopback | Restart and shutdown have one Linux owner |
| Join a Chirps cluster | Select explicit Linux interface/port → mirrored network → explicit Hyper-V firewall rule | Negotiation packets can enter without widening unrelated app ports |
| Persist data | Read/write in distro ext4 XDG paths | No live DB, repository, cache, or state is under `/mnt/*` |
| Resolve secrets | Read Linux Secret Service as the selected user | Unavailable keyring fails closed; no plaintext fallback crosses into Windows |
| Export or back up | Stop or snapshot the live owner → write an explicit export to a Windows mount | A Windows mount is an output, never the live data owner |

## Responsibility boundary

| Responsibility | Owner |
|---|---|
| Support values and rejection matrix | This v1 contract / Issue #16 |
| Idempotent install, systemd lifecycle, update, rollback, uninstall | Issue #19 |
| Exact distro/user/PID selection, non-persistent secret handoff, and deletion of the current `identity.key` fallback | Issue #25 |
| Chirps node negotiation and secure cluster messaging | `nimino-chirps` Rust adapter |
| Discovery policy, replication, conflict resolution, DB, and automatic sync | Nimino domain/core issues |
| Incompatible public cutover and obsolete path deletion | Issue #12 |

WSLg, WSL, systemd, Secret Service, and the filesystem are platform adapters.
They do not own product rules. Chirps supplies cluster facts and messaging; it
does not acquire database, replication, sync, quorum, or conflict authority.

## Policy details

### GUI

The release launches the Linux desktop in WSLg. WSLg may choose its provided
Wayland or X11 compatibility path. Nimino does not install a third-party X
server, remote-desktop shell, or parallel Windows-native desktop.

### Filesystem

Source trees, package caches, application state, logs, and databases live in
the selected distribution's ext4 filesystem. `/mnt/*` is allowed only for an
explicit import, export, or completed backup. It is never a live database or
build root. This follows Microsoft's cross-filesystem performance guidance.

### Network

The host uses WSL mirrored networking. Desktop-to-local-service traffic uses
`127.0.0.1`; Windows callers use `localhost`. A Chirps listener selects an
explicit Linux interface and port. LAN ingress requires a narrowly scoped
Windows Hyper-V firewall rule; installation never changes the global inbound
default. Mirrored multicast is transport capability only—Nimino still owns
peer-selection and synchronization semantics.

### Secrets

Production secrets live in Linux Secret Service for the selected unprivileged
user. Missing or locked Secret Service fails closed. Command-line arguments,
Windows environment variables, plaintext `.env` files, `/mnt/*` files, and
Windows Credential Manager are not release secret carriers. Issue #25 owns the
process-scoped, non-persistent launcher handoff. The current desktop can write
`identity.key` with mode `0o600` when Secret Service is unavailable; that is a
legacy path, not an accepted WSL backend. #25 must remove every read/write
fallback after locked/unavailable, import, first-run, and restart tests pass.

## Lifecycle

| Artifact | Action | Complete or delete when | Proof |
|---|---|---|---|
| `contracts/wsl-support-v1.json` | keep as v1 source of truth | explicit new-version cutover replaces it | contract checker and negative matrix |
| `scripts/check-wsl-support-contract.mjs` | keep in repository policy gate | contract is explicitly retired | local and hosted `Detect Changed Paths` gate |
| Windows-native shell path | do not extend; later delete | #25 provides the exact WSL launcher and #12 cuts over | launcher negative tests and dead-path gate |
| Desktop `identity.key` fallback | delete in #25 | Secret Service migration is verified and unavailable/locked startup fails closed | no-read/no-write negative tests and dead-path gate |
| Installer/service implementation | add only in #19 | its install/update/uninstall criteria pass | #19 lifecycle tests |

## Verification

```bash
. ./bin/activate-hermit
just wsl-support-contract
just naming-contract
just ci-lanes-contract
```

The first command accepts the canonical profile and rejects every mutated
unsupported case. It also verifies that each local evidence symbol still
exists. Hosted CI runs the same gate in `Detect Changed Paths`.

## Primary references

- [Windows 11 release information](https://learn.microsoft.com/en-us/windows/release-health/windows11-release-information)
- [WSL 2.7.3 release](https://github.com/microsoft/WSL/releases/tag/2.7.3)
- [Run Linux GUI apps with WSL](https://learn.microsoft.com/en-us/windows/wsl/tutorials/gui-apps)
- [WSL networking](https://learn.microsoft.com/en-us/windows/wsl/networking)
- [Working across WSL filesystems](https://learn.microsoft.com/en-us/windows/wsl/filesystems)
- [Use systemd with WSL](https://learn.microsoft.com/en-us/windows/wsl/systemd)
- [Ubuntu on WSL release notes](https://documentation.ubuntu.com/wsl/latest/reference/release_notes/)
- [Secret Service API](https://specifications.freedesktop.org/secret-service/latest/)
