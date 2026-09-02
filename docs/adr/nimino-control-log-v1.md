# ADR: Nimino control-log protocol v1

Status: Accepted for the Nimino v1.0 cutover. Implements Issue #46.

## Decision

Nimino owns cluster authority above the thin Chirps transport. Chirps supplies
authenticated node identities, reachability, and opaque messages; its peer view
cannot admit a node, elect a leader, commit a control entry, or grant ownership.

The versioned source of truth is
`contracts/nimino-control/v1/model.json`. Runtime admission is #48, durable
control-log I/O is #49, the replicated state machine is #51, and lease/fencing
is #52. This issue defines their shared safety contract without implementing
those runtimes or adding an old Buzz compatibility mode.

## Quorum and authority

For a non-empty voter set `V`, quorum is `floor(|V| / 2) + 1`. Stable phases
require one majority of the active set. The joint phase requires a majority of
the old set and a majority of the new set for both election and commit.

Voters grant at most one candidate vote in one persisted term. Majority
certificates intersect, so two disjoint minorities cannot create authority.
Transport reachability and elapsed time are never quorum votes. A leader may
append only while it retains authority for its term; an entry becomes committed
only with the active voter-phase certificate. Entries apply strictly through
the committed contiguous prefix.

## Epoch and voter-set transition

Election `term` is monotonic and persisted with the vote. `voterEpoch` starts at
one and increments on every committed configuration transition. The only v1
path is:

```text
stable-old
  -- commit begin-joint with old quorum --> joint
  -- commit finalize with old+new quorums --> stable-new
```

Each configuration commit invalidates current authority and requires election
under the newly active phase. A direct old-to-new jump, a finalize under only
one side, or reuse of the prior authority certificate is invalid. Removing the
last voter is invalid because every stable set must remain non-empty.

## Clock assumptions

Election deadlines use a monotonic local clock. Wall-clock equality, synchronized
clocks, and bounded clock skew are not safety assumptions. A timeout may start a
new term; it cannot elect, commit, authorize a node, or fence an owner by itself.
Lease timing and fencing tokens are deliberately deferred to #52.

## Snapshot and recovery

A snapshot is made only from an applied index at or below `commitIndex`. It
stores the last included index and term plus the exact voter epoch and phase at
that index. Recovery installs that snapshot, then replays only the committed
suffix in index order. Uncommitted state, peer observations, and wall-clock
timestamps cannot enter a snapshot as authority.

The durable adapter in #49 must atomically persist term/vote metadata, append
and fsync the control log, and install snapshots. Torn-write behavior belongs
to that adapter; the logical recovery ceiling defined here is `commitIndex`.

## Durable adapter (#49)

`crates/nimino-store` implements a separate `ControlLogStorePort` on three
control-only `redb` tables. Metadata uses revision compare-and-set; log writes
replace only an uncommitted suffix and commit with immediate durability;
snapshot installation updates recovery watermarks and compacts the covered
prefix in the same transaction. Recovery validates the snapshot boundary,
metadata indices, and contiguous suffix before returning opaque commands to the
Nim state machine. These tables never enter the canonical event change feed.

Issue #51 implements that state machine in pure Nim. Election, append, commit,
apply, snapshot, and recovery are deterministic plans. A plan becomes
authoritative only after its `ControlLogStorePort` action succeeds; failure
settles to the pre-transition state. Configuration entries do not alter voter
authority when appended—only quorum commit advances phase and voter epoch.

## Formal model

`formal/tla/cluster/NiminoControlLog.tla` models election timeouts, quorum-backed
authority, append/ack/commit/apply, both voter transitions, snapshot, crash, and
replay. The initial bounded scenario deliberately permits one uncommitted entry
at a time; #51 may pipeline entries only after retaining the same prefix, epoch,
and quorum invariants.

The three-node scenario changes `{n1,n2}` to `{n2,n3}`. TLC exhaustively checks
that:

- authority and every commit carry the required phase quorum;
- any two active quorum certificates intersect;
- voter phases and voter epochs follow only the committed two-step path;
- terms and recorded epochs never regress; and
- apply, snapshot, crash recovery, and replay never pass the committed prefix.

Run:

```bash
just control-model-check
```

The accepted run and exact model hashes are recorded in
`formal/evidence/FM-cluster-control-log_20260828_summary.md`. CI runs the
lighter `control-model-contract` gate to reject hash, invariant, ownership, or
evidence drift; a changed model must be rechecked with TLC and recorded again.

## Rejected alternatives

- Chirps membership as authority: rejected because Chirps is negotiation and
  transport only.
- One-step voter replacement: rejected because it admits disjoint old/new
  majorities.
- Wall-clock or last-writer-wins leadership: rejected because partitions and
  clock skew can advance two authorities.
- Shared Postgres/Redis control authority: rejected; #49 provides a per-node
  durable log and #58 removes the old control-plane surfaces.
- Dual runtime or staged public migration: rejected; #12 owns one incompatible
  cutover.
