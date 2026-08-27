# Nimino event policy v1

Nim owns five deterministic decisions: event classification, replacement
ordering, NIP-10 thread plans, NIP-09 deletion plans, and NIP-25 reaction
plans. Rust verifies signatures, gathers facts, transports typed frames, and
applies the returned plan; it does not choose the result.

`v1/contract.json` is the cutover/deletion inventory. The listed Rust policy
paths remain only until Issue #12 switches the relay atomically. They are not a
fallback or a compatibility mode. `v1/golden.json` is the language-neutral
corpus executed through the real Rust-to-Nim worker boundary.

Deletion counter deltas apply only with the returned live-to-deleted mutation.
An already deleted target returns `noop` with zero deltas, so replay cannot
decrement thread counters twice.
