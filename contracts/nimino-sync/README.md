# Nimino bounded anti-entropy contract

Version 1 is a pull-based, per-community sync session. A source advertises its
durable canonical checkpoint and SHA-256 prefix digest; a peer requests bounded
ordered ranges after its own checkpoint. Each frame binds protocol version,
session, community, sender, and receiver.

Only one batch may be applying at a time. The receiver verifies sequence shape,
encoded size, record and rolling digests, then commits through the existing
`NodeStorePort` exact-checkpoint transaction. The protocol advances only after
that commit succeeds. A restart reads the durable canonical checkpoint and
recomputes its prefix digest, so an unacknowledged batch resumes without a
second checkpoint database.

Chirps carries opaque frames through its bounded queues and supplies no sync or
conflict authority. Timeout and cancel close a session; a late store settlement
cannot reopen it. #53 owns divergent histories, #56 composes real multi-process
cluster scenarios, and #12 removes Redis/shared-DB replication at cutover.

Run `just nimino-sync-contract` and `just nim-test` for the contract and the
3-node bootstrap, restart, slow-peer, duplicate, isolation, and backpressure
scenarios.
