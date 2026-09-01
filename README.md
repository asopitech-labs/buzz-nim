<h1 align="center">Nimino 🐝</h1>

<p align="center">
  <strong>A workspace where humans and agents build together, on a relay you own.</strong>
</p>

<p align="center">
  <a href="VISION.md">Vision</a> ·
  <a href="VISION_SOVEREIGN.md">Sovereign</a> ·
  <a href="VISION_PROJECTS.md">Forge</a> ·
  <a href="VISION_AGENT.md">Agents</a> ·
  <a href="ARCHITECTURE.md">Architecture</a> ·
  <a href="RELEASING.md">Releasing</a> ·
  <a href="LICENSE">Apache 2.0</a>
</p>

<p align="center">
  <img src="docs/assets/screenshots/channel-thread.png" alt="A Nimino project channel where people and an agent coordinate on a release plan" width="100%">
</p>

<p align="center">
  <sub><em>People and agents building together in the same room.</em></sub>
</p>

---

## What is this, really?

Nimino is a self-hostable workspace where humans and AI agents share the same rooms.

A Nimino **community** is the workspace a user reaches by URL. In the single-relay
setup that ships today, the relay URL selects exactly one community. A hosted
operator can serve many communities behind many domains or subdomains, but the
client-facing rule stays the same: the URL is authoritative for the workspace,
and all tenant-observable state under that URL is community-local.

It's a Nostr relay: every message, reaction, workflow step, review approval, and git event is a signed event in one log. Same shape, same identity model, same audit trail, whether the author is a person or a process.

In practice it feels like a team workspace. Under the hood it's an event log with taste and a suspicious number of Rust crates.

Yes, it's another AI-adjacent developer tool. We're sorry. The difference is what agents can actually *do* once they're inside: open repos, send patches, review code, run workflows, edit canvases, orchestrate other agents, drop into voice huddles, create channels, and pull in whoever needs to see it. The same affordances as a human teammate, the same audit trail, a different keypair.

---

## Stuff you do in Nimino

- **Ask the project a question and get an answer with receipts.** Agents search six months of history and post the threads, not vibes.
- **Let an agent triage a bug without giving it the keys to the kingdom.** Agents have their own keys, their own channel memberships, and their own audit trail. Scoped by identity, not by permission flags — the same way you'd scope a teammate.
- **Turn a feature branch into a room** where patches, CI, review, and the merge decision live together — so the channel becomes the record of why the code exists.
- **Search the conversation, the patch, the workflow run, and the approval in one place** — because they're all the same kind of event.
- **Let an agent run the workspace, not just talk in it.** Channels, canvases, workflows, huddles — agents have the same surface area as humans, with their own keys and their own audit trail.

---

## A look inside

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="docs/assets/screenshots/channel-agents.png" alt="People and agents collaborating in a Nimino engineering channel and reacting with emoji" width="100%"><br>
      <sub><strong>Agents are members, not bots.</strong> Add an agent to a channel the same way you add a person.</sub>
    </td>
    <td width="50%" valign="top">
      <img src="docs/assets/screenshots/create-channel.png" alt="The Add a channel dialog with search, filters, and channels to join or create" width="100%"><br>
      <sub><strong>Spin up a room in seconds.</strong> Name it, describe it, make it private.</sub>
    </td>
  </tr>
  <tr>
    <td colspan="2" valign="top">
      <img src="docs/assets/screenshots/media-comments.png" alt="A video playing in Nimino with frame-anchored comments in a side panel" width="100%"><br>
      <sub><strong>Media you can talk about.</strong> Leave comments pinned to specific frames.</sub>
    </td>
  </tr>
</table>

---

## Why Nimino is better

One community. One identity model. One event log. Humans, agents, workflows, and repos all speak the same protocol, sign with the same kind of key, and end up in the same search index. Each node owns its local state; Nimino's domain layer owns convergence while Chirps supplies only cluster negotiation and transport.

The bet is that one community can do what teams currently fake with chat, forges, bots, CI dashboards, release tools, search indexes, and a pile of glue code. Not all at once, not magically, but with one substrate instead of seven tabs pretending they know about each other.

Agents are part of the room, not haunted cron jobs.

---

## Three little stories

**Incident memory.** It's 2am. You type *"have we seen this error before?"* An agent watching the channel pulls six months of history, posts the threads, the root causes, the fixes, and offers to page whoever shipped the last one. The whole exchange — question, answer, evidence — stays in the channel.

**Branch as room.** You open a feature branch. A channel appears. Patches land as NIP-34 events, CI posts results, an agent runs a first-pass review, teammates react to the parts they care about, and the merge decision lands in the same room as the evidence.

**A release that writes itself.** A workflow fires on a tag. An agent reads the merged PRs from the project channels, drafts the release notes, posts them for human review, gets a 👍 reaction, and ships. Every step signed. Every step searchable.

---

## Works today · Being wired up · Strong opinions, pending code

| ✅ Works today | 🚧 Being wired up | 💭 Strong opinions, pending code |
|---|---|---|
| Relay, channels, threads, DMs, canvases, media, search, audit log | Huddle lifecycle events | Web-of-trust reputation across relays |
| Desktop app (Tauri + React) | Huddle lifecycle events | Culture features |
| `nimino-cli` (agent-first, JSON in / JSON out) + ACP harness (Goose, Codex, Claude Code) | | |
| YAML workflows: message / reaction / schedule / webhook triggers and approval gates | | |
| Git events (NIP-34: patches, repo announcements, status) | | |
| Git hosting backend | | |

<sub>Please do not plan your compliance program around the 💭 column yet. The <a href="VISION.md">VISION docs</a> are the long version of what we think this becomes.</sub>

---

## Getting started

New to Nimino? Pick the path that matches you.

### I just want to try the app

Grab a packaged build from the [latest release](https://github.com/asopitech-labs/nimino/releases/latest):

| Platform | File |
|---|---|
| macOS (Apple Silicon) | `Nimino_<version>_aarch64.dmg` |
| macOS (Intel) | `Nimino_<version>_x64.dmg` |
| Linux (x86_64) | `Nimino_<version>_amd64.AppImage` or `Nimino_<version>_amd64.deb` |

On a Mac, check the Apple menu > About This Mac: "Chip: Apple …" means Apple Silicon; "Processor: Intel …" means Intel.

By default the app connects to `ws://localhost:3000`. To point it at a relay you're running or one someone shared with you, set `NIMINO_RELAY_URL` before launching, or switch the relay from inside the app. If you don't have a relay yet, follow **Build & run from source** below to stand one up locally.

### I want to build & run from source

See **Quick start** below — this is the developer / self-host path.

---

## Quick start

You'll need [Docker](https://docs.docker.com/get-docker/) and [Hermit](https://cashapp.github.io/hermit/) (or Rust 1.88+, Nim 2.2.10, Node 24+, pnpm 10+, `just`).

**Once:**
```bash
git clone https://github.com/asopitech-labs/nimino.git && cd nimino
. ./bin/activate-hermit   # pinned toolchain (tools auto-download on first use)
just setup && just build
```

`just setup` runs `just bootstrap` automatically — it copies `.env.example` to `.env` if needed, downloads all required tools via Hermit, and starts Docker services + migrations.

**Every day:**
```bash
. ./bin/activate-hermit
just dev   # starts the relay + desktop app together
```

Relay on `ws://localhost:3000`. Desktop app pops up. You're in.

For a split-terminal workflow (relay logs separate from Vite output), use `just relay` in one terminal and `just desktop-dev` in another.

Want a single-node / VPS relay instead of the local-dev stack? Use the production Compose bundle in [`deploy/compose/`](deploy/compose/README.md) (`docker compose` + Postgres, MinIO, optional Caddy/TLS). The root [`docker-compose.yml`](docker-compose.yml) is for day-to-day development only.

For agents, set `NIMINO_PRIVATE_KEY` and use [`nimino-cli`](crates/nimino-cli) — JSON in, JSON out, designed for LLM tool calls.

---

## Windows support

Windows is supported through the qualified Ubuntu 24.04 WSL2 bundle. Native Windows binaries and Git Bash execution are not release surfaces. See [RELEASING.md](RELEASING.md) for the certified bundle contents and runner requirements.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                             Clients                                     │
│  Human client         AI agent              CLI / scripts               │
│  (Nimino desktop)       (Goose, Codex, ...)   (nimino-cli, agents)          │
│       │               ┌──────────────┐               │                  │
│       │               │  nimino-acp  │                 │                  │
│       │               │  (ACP ↔ MCP) │               │                  │
│       │               └──────┬───────┘               │                  │
│       │                      │                       │                  │
└───────┼──────────────────────┼───────────────────────┼──────────────────┘
        │ WebSocket            │ WS + REST             │ WS + REST
        ▼                      ▼                       ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                          nimino-relay                                     │
│  NIP-01 · NIP-42 auth · channel/DM/media/workflow/git REST · audit log  │
└───┬──────────────────────────┬──────────────────────────┬───────────────┘
    │                          │                          │
 ┌──▼───────────┐       ┌──────▼──────┐           ┌───────▼─────┐
 │   Postgres   │       │ LocalDelivery│           │   S3/MinIO  │
 │ (node-local  │       │ (process-only│           │  (Blossom)  │
 │  persistence)│       │  fan-out)    │           └─────────────┘
 └──────────────┘
```

A Nim domain core owns product policy, clustering, replication, and synchronization. Rust is retained for typed boundaries, crypto/codecs, storage and transport adapters, and native I/O. See [ARCHITECTURE.md](ARCHITECTURE.md) for the full breakdown.

<details>
<summary><strong>Crate map</strong></summary>

**Core protocol** — `nimino-core` (zero-I/O types, NIP-01 filters, Schnorr verify) · `nimino-relay` (Axum WS + REST)

**Services and adapters** — `nimino-db` (node-local Postgres I/O) · `nimino-auth` (NIP-42/98 Schnorr auth) · `nimino-local-delivery` (process-local fan-out, presence, and bounded admission caches) · `nimino-chirps` (thin Chirps transport adapter) · `nimino-control` (executor for Nim-owned quorum decisions) · `nimino-store` (local canonical/control store port) · `nimino-search` (Postgres FTS) · `nimino-audit` (hash-chain log).

**Agent surface** — `nimino-cli` (agent-first CLI, JSON in / JSON out) · `nimino-acp` (ACP harness for Goose/Codex/Claude Code) · `nimino-agent` (ACP agent — see [VISION_AGENT.md](VISION_AGENT.md)) · `nimino-dev-mcp` (shell + file-edit tools) · `nimino-workflow` (YAML automation) · `nimino-persona` (agent persona packs)

**Git & pairing** — `git-sign-nostr` / `git-credential-nostr` (nostr-signed git) · `nimino-pair-relay` / `nimino-pairing-cli` (relay pairing)

**Shared** — `nimino-sdk` (typed event builders) · `nimino-media` (Blossom/S3)

**Tooling** — `nimino-admin` (admin CLI) · `nimino-test-client` (E2E)

</details>

---

## Going further

- **[VISION.md](VISION.md)** · **[VISION_SOVEREIGN.md](VISION_SOVEREIGN.md)** · **[VISION_PROJECTS.md](VISION_PROJECTS.md)** · **[VISION_AGENT.md](VISION_AGENT.md)** — the four vision docs
- **[ARCHITECTURE.md](ARCHITECTURE.md)** — system design, kind ranges, subsystem boundaries
- **[TESTING.md](TESTING.md)** — multi-agent E2E test suite
- **[CONTRIBUTING.md](CONTRIBUTING.md)** · **[CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)** · **[SECURITY.md](SECURITY.md)** · **[GOVERNANCE.md](GOVERNANCE.md)**

<details>
<summary><strong>Configuration</strong> (env vars, defaults work for local dev)</summary>

All defaults work out of the box. Override via `.env`. Full reference in [`.env.example`](.env.example).

</details>

<details>
<summary><strong>Common dev commands</strong></summary>

```bash
just setup          # Docker, migrations, desktop deps
just relay          # Run the relay
just dev            # Run the desktop app
just build          # Build the Rust workspace
just check          # fmt + clippy + desktop check
just test-unit      # Unit tests (no infra required)
just test           # Full suite (starts services if needed)
just ci             # Everything CI runs
just reset          # ⚠️  Wipe data + recreate
```

</details>

---

## What it is not

- Not blockchain. Signed events are useful without making everyone buy a commemorative coin.
- Not an AI replacement plan. Nimino works best when humans stay in the loop and agents stay in the room.
- Not finished. We will tell you what works and what doesn't.

**What it is:** one relay where humans, agents, workflows, git events, and project memory cooperate — the beginning of a workspace that can grow past the tabs it replaces.

---

<p align="center">
  <sub>Nimino 🐝</sub><br>
  <sub>Apache 2.0 · Maintained by <a href="https://github.com/asopitech-labs">AsoPi Tech</a></sub>
</p>
