# Nimino cluster lifecycle contract

Version 1 is the pure policy boundary for Issue #48. It owns exact admission,
node lifecycle transitions, and per-state lane gates. Chirps supplies an
authenticated transport identity and opaque messaging only; reachability and
peer observations never grant admission or authority.

The lifecycle is a hard sequence:

```text
offline → joining → syncing → ready → draining → offline
```

`join` requires an authenticated, non-revoked, uniquely bound identity and the
exact Nimino product/control/data versions. Entering syncing, ready, draining,
or offline requires the relevant committed control decision. Ready additionally
requires an installed snapshot, matching checkpoint, and matching nonzero voter
epoch. Offline requires zero active work and a final matching checkpoint.

Before ready, client read/write and lease lanes are denied. Joining permits only
negotiation; syncing and draining permit negotiation, control, and data sync;
offline permits nothing. #51 persists and replicates these decisions, #52 owns
lease semantics, #56 composes the multi-node fault harness, and #12 performs the
single incompatible cutover.

Run `just nimino-cluster-contract` for schema/ownership/corpus checks and
`just nim-boundary-test` for the real Nim worker lifecycle scenarios.
