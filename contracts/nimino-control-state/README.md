# Nimino control state machine

Version 1 is the pure Nim decision contract above the local control-log port and
the thin Chirps transport. It verifies quorum certificates, advances voter
configuration only after commit, and exposes persistence-first transition
plans. Rust never interprets a command payload or chooses authority.

Each plan retains the pre-transition state. The host executes every declared
store action, then calls `settleControlPlan`; a failed append, metadata CAS, or
snapshot install returns the prior state with `store_failure`. This makes store
failure fail closed without embedding storage I/O in Nim.

The v1 state machine permits one pending entry and takes snapshots only when the
committed prefix is fully applied. These restrictions are safe subsets of the
formal model. Lease, fencing, and routing remain #52; multi-node partition and
rejoin orchestration remains #56.
