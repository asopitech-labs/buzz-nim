# Nimino projection rebuild operations

## Start and resume

Capture one canonical checkpoint and prefix digest, choose a fresh epoch and
#52-fenced owner, then call `startProjection` and
`begin_projection_stage`. Scan current canonical `event` records from zero in
strict key order. Before every batch, confirm the canonical checkpoint is still
unchanged.

Pass the page and any required current thread rows to `planProjectionBatch`.
Persist its mutations with exact stage revision/cursor CAS, then settle the plan.
After restart, recover metadata and rows from `recover_projection_stage` and
continue strictly after its cursor.

## Publish and cleanup

EOF moves Nim state to ready. Recover all staged rows and call the existing
`NodeStorePort.replace_cache` using the publish plan's record type, source
checkpoint, and intent ID. Only after success should the exact stage epoch be
discarded. A crash before discard repeats the same idempotent cache intent.

Drop and rebuild one projection at a time; canonical data never changes. Source
checkpoint drift cancels the epoch and starts a new zero-state build.

## Verification

```bash
just nimino-projection-contract
cd nim/nimino_core && nim c -r --hints:off tests/test_projection_lifecycle.nim
cargo test -p nimino-store --test projection_stage
```
