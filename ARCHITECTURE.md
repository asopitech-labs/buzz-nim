# Nimino architecture

This document describes the repository as it exists after the independent-fork
cutover. It is intentionally not an upstream migration guide.

## Status at a glance

Nimino has a working Nim domain worker, a typed Nim↔Rust boundary, a narrow
Chirps adapter, a Rust relay, Desktop/Web clients, CLI tooling, and independent
release contracts. The production relay starts its Nim worker, Chirps mesh,
Nim-owned quorum control executor, per-node canonical store, projection follower,
and bounded anti-entropy loop as one runtime. Multi-node release qualification
still requires the released artifact to pass the complete 1/3/5-node matrix.

Mobile and all inherited release/control paths are absent. There is no
compatibility mode, old product alias, Redis fan-out, or legacy relay mesh.

## Ownership rules

| Concern | Owner | Rule |
|---|---|---|
| Product and domain policy | Nim | Membership, moderation, workflow, cluster lifecycle, replication, convergence, repair |
| Cluster negotiation | Chirps via `nimino-chirps` | Peer negotiation, reachability facts, secure opaque messages only |
| Typed language boundary | `nimino-boundary` | Versioned request/response framing, lifecycle, cancellation, errors |
| Crypto, codecs, storage and bounded I/O | Rust adapters | No product policy |
| Presentation state | TypeScript/React | No domain authority |
| Durable relay data | PostgreSQL/object storage | Accessed through Rust effect adapters today |
| Process-local ephemeral state | `nimino-local-delivery` | Presence, admission windows and replay cache; never cluster authority |

The executable ownership manifest is
`contracts/rust-responsibility/v1/manifest.json`. New policy in Rust is a
contract violation even when it is convenient.

## Repository map

```text
nim/
  nimino_core/             pure domain modules and boundary worker

crates/
  nimino-boundary/         typed worker protocol and lifecycle
  nimino-chirps/           only crate allowed to depend on alopex-chirps
  nimino-control/          schedules and persists Nim control decisions
  nimino-store/            storage effect boundary
  nimino-object-store/     object-storage adapter
  nimino-data-ops/         verify/repair operator commands
  nimino-relay/            HTTP/WebSocket composition and socket delivery
  nimino-local-delivery/   process-local ephemeral state
  nimino-core/             wire types, Nostr verification and codecs
  nimino-db/               PostgreSQL adapter
  ...                      bounded client, tooling and effect adapters

desktop/                   Tauri + React desktop application
web/                       browser client
contracts/                 executable ownership and release contracts
deploy/                    Nimino-owned container/Helm assets
```

There is no `mobile/` tree.

## Current relay flow

```text
client
  │ HTTP / WebSocket
  ▼
nimino-relay
  ├─ resolve host → community
  ├─ authenticate and verify signed input
  ├─ enforce the current Rust ingest/access seam
  ├─ persist durable events in PostgreSQL/object storage
  ├─ fan out through one send-time access gate to local sockets
  └─ run Nim + Chirps + quorum control + bounded sync over a per-node redb store
```

All local live delivery routes through
`handlers::event::filter_fanout_by_access`. Subscription registration alone is
never authority to deliver. Community identity is attached to both the event
and receiving connection and is checked again at the send seam.

`nimino-local-delivery` does not forward between processes. Its replay and
rate-limit state is process-local, so a multi-node release must replace those
compositions with Nim-owned cluster/domain operations before claiming
cluster-wide security guarantees.

## Nim domain boundary

The worker protocol is defined by `contracts/nim-rust-boundary/v1` and
implemented by `nim/nimino_core` plus `nimino-boundary`.

```text
Rust adapter                    Nim worker
     │ framed request                │
     ├──────────────────────────────►│ validate + apply domain policy
     │ typed result / stable error   │
     ◄───────────────────────────────┤
     │ perform approved effect       │
```

The boundary owns framing, process lifecycle, timeouts and cancellation. Nim
owns the decision. Rust may reject malformed or cryptographically invalid input
before the call, but it must not recreate the policy after the call.

## Chirps and cluster flow

`nimino-chirps` is the sole `alopex-chirps` dependency. It exposes node identity,
validated configuration, reachability hints and bounded opaque messaging.
Chirps does not own database semantics, membership authority, replication,
conflict resolution or repair.

The intended composition is:

```text
Chirps secure message / peer fact
              │
              ▼
       Rust typed adapter
              │
              ▼
      Nim cluster lifecycle
      sync / convergence / repair
              │ approved effect
              ▼
      store and socket adapters
```

The relay composes `nimino-chirps`, `nimino-boundary`, `nimino-control`, the Nim
worker, and `nimino-sync`. Real three-node scenarios prove election, quorum
commit, minority rejection, restart catch-up, divergent data convergence,
community isolation, and clean shutdown. Canonical writes precede PostgreSQL
projection, and a durable projection follower resumes from its Redb checkpoint.

## Persistence and repair

- PostgreSQL stores durable relay events and relational projections.
- Object storage holds media and Git objects.
- Ephemeral Nostr events and presence are process-local today.
- `nimino-data-ops` provides explicit verification and repair commands over the
  store adapters.
- Nim owns convergence and repair policy; adapters expose facts and execute
  approved effects.

No cache or transport is a source of truth.

## Client surfaces

- Desktop: Tauri 2 + React.
- WSL: the supported Windows distribution path and release target.
- Web: browser client served independently or by the relay HTTP surface.
- CLI/agent tools: Nostr-first operations through `nimino-cli` and the typed
  agent adapters.

Native Windows and mobile are not supported release surfaces.

## Release architecture

The repository owns its release pipeline:

- `nimino-platform-release.yml` builds Desktop/WSL artifacts.
- `nimino-relay-release.yml` builds the relay image and chart.
- `nimino-promote.yml` promotes only a previously qualified release set.
- `nimino-benchmark.yml` records performance evidence.

`RELEASING.md` is the operational source of truth. Promotion requires the exact
artifact digest certified by the clean-clone and platform/cluster/security
matrix. Inherited repositories, registries, credentials and signing authorities
are denylisted by contract.

## Release blockers that must remain visible

The repository must not claim production cluster readiness until all are true:

1. PostgreSQL event ingestion and projections are connected to the Nim-owned
   canonical sync path, including divergent-writer convergence and repair.
2. NIP-98 replay protection, admission limits, presence and live invalidations
   have explicit multi-node ownership instead of process-local state.
3. The released image/chart passes real 1/3/5-node negotiation, convergence,
   partition/rejoin and repair scenarios.
4. Clean-clone Desktop, WSL and relay artifacts are signed and verified by the
   independent Nimino pipeline.
