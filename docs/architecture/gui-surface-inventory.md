# Nimino GUI surface inventory

Issue #15 freezes product-surface decisions for Desktop, Web, and Admin. The
machine-readable authority is
[`contracts/gui-surface-v1.json`](../../contracts/gui-surface-v1.json); CI
rejects unclassified feature roots and routes through
[`scripts/check-gui-surface-contract.mjs`](../../scripts/check-gui-surface-contract.mjs).
Mobile is intentionally outside this contract and was removed by #18/#33/#28.

## Decision summary

| Client | Surface | Decision | Destination / intent | Implementation |
|---|---|---|---|---|
| Desktop | Home | keep | Personalized feed, notifications, reminders | #34 |
| Desktop | Conversations | keep | Stream, Forum, and DM in one channel shell | #34 |
| Desktop | Search | keep | Global keyboard-first resolver | #34 |
| Desktop | Shell | shrink | One community/navigation/overlay hierarchy | #34 |
| Desktop | Onboarding | shrink | Identity and community setup only | #34 |
| Desktop | Agent workspace | shrink | Directory, activity, memory, terminal detail | #34 |
| Desktop | Projects | shrink | NIP-34 forge view | #34 |
| Desktop | Workflows | shrink | Definition, trace, approval views | #34 |
| Desktop | Huddles | shrink | Contextual channel affordance | #34 |
| Desktop | Identity/profile | shrink | Community-local profile/presence/status | #34 |
| Desktop | Settings | shrink | Presentation and platform-adapter controls | #34 |
| Desktop | Pulse | merge | Home, Conversations, and Search by capability | #34 |
| Desktop | Community governance | merge | Role-gated Community settings section | #34 |
| Desktop | Archive controls | merge | Advanced Settings section | #34 |
| Web | Repositories | shrink | Workspace root + repo/tree/blob | #39 |
| Web | Invite | keep | Host-scoped invite landing | #39 |
| Admin | Moderation inbox | keep | Canonical operator inbox | #39 |
| Admin | Feedback inbox | merge | Typed category in the operator inbox | #39 |

`keep` preserves independent reachability. `shrink` preserves the use case with
less navigation or presentation state. `merge` preserves the named data
contract under one destination. No surface is deleted in #15; redundant route
aliases below are removed by their implementation owner.

## Route dispositions

Most routes remain canonical. The removed and pending non-keep routes are the
complete exception list.

### Desktop — completed by #34

| Removed route | Decision | Canonical destination | Preserved data |
|---|---|---|---|
| `/pulse` | merge | Home Activity (`/?view=activity`), Conversations, Search | Timelines, publishing, actions, agent cards |
| `/reminders` | delete | Home, then select Reminders | Due state and badges; the old route never selected the filter |
| `/messages/new` | merge | New Message inside the Conversations shell | Participant-scoped DM creation |

The route manifest no longer contains these paths or compatibility aliases.
Home owns Inbox and Activity tabs, New Message is shell-local state, and channel
workflow actions open the canonical `/workflows` route instead of a duplicate
editor overlay. The GUI contract and Desktop keyboard E2E enforce this shape.

### Web and Admin — owned by #39

| Current route | Decision | Canonical destination | Preserved data |
|---|---|---|---|
| Web `/repos` | delete | `/` | Repository list |
| Admin `/` | delete | `/reports` | Moderation inbox |
| Admin `/feedback` | merge | Operator inbox | Typed feedback payload and acted-on state |
| Admin `/feedback/$feedbackId` | merge | Operator inbox detail | Typed feedback payload |

There are no compatibility aliases in the target topology. Pending routes are
removed only after their destination is reachable and their negative route test
exists.

## Pulse capability handoff

Pulse is not treated as an agent-only feed. Its complete current capability set
has an explicit destination:

| Capability | Decision | Destination | Data owner / contract |
|---|---|---|---|
| Everyone / people / liked / mine timelines | merge | Home | Nim event/query policy; kind-1 notes, contacts, likes, profiles |
| Agent timeline and grouped cards | merge | Home | Nim agent/event policy; Home activity renderer |
| Note publishing and replies | merge | Home | Nim event acceptance; mentions, media, reply links, invalidation |
| Upvote / reply / share | merge | Home note cards | Nim events plus the OS clipboard adapter |
| Start DM | merge | Conversations | Participant-scoped DM channel owner |
| Pulse-only search input | delete | Global Search | Add public-note results, then verify the dead tab/route |

Admin feedback keeps its API payload distinct after the UI merge. #39 also
migrates per-feedback acted-on booleans from `nimino-admin-feedback-status` to the
unified inbox owner, verifies ID parity, and deletes the stale key. It must not
silently reset operator work state.

## Ownership handoff

| Concern | Owner | Required proof |
|---|---|---|
| Community-scoped state/reset | #21 | community-switch integration and owner inventory |
| Motion/reduced motion | #23 | budget gate, reduced-motion E2E, stable screenshot |
| Desktop navigation/layout | #34 | route contract, keyboard E2E, visual regression |
| Web/Admin consolidation | #39 | route smoke, builds, dead-reference search |
| Native adapter boundary | #44 | command contract and dependency gate |

Feature/domain policy does not move into React, Web, Admin, or Tauri as part of
these UI changes. The canonical event, membership, workflow, project, and
moderation contracts stay with their domain owners; the surfaces only present
them.
