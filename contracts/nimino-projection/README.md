# Nimino projection rebuild contract

Version 1 rebuilds search, thread counters, and feed rows from a fixed canonical
checkpoint. Each build has an exact community, source digest, epoch, owner,
revision, and key cursor. A source checkpoint change, stale owner, stale epoch,
or cursor/revision conflict fails closed.

`nimino-store` keeps partial rows and metadata in separate rebuild-only tables.
Each batch is atomic and resumable. Once EOF is durable, the host recovers the
complete staged rows and publishes them through the existing idempotent
`cache.replace` transaction at the same source checkpoint. A crash before stage
cleanup repeats that same publish intent safely.

Nim derives search, feed, and thread rows. Rust stores opaque row bytes and never
computes counters or ranking. Dropping every projection and rebuilding from
zero produces the same rows as a partial multi-batch rebuild. #59 owns operator
repair and #12 removes the PostgreSQL/search mixed projection paths at cutover.

Run `just nimino-projection-contract`, `just nim-test`, and
`cargo test -p nimino-store --test projection_stage`.
