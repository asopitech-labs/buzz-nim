# Nimino per-node store operations

`nimino-store` is the local persistence adapter for Nimino canonical truth,
rebuildable cache rows, and append-only evidence. It does not own product
validation, replication, projections, or Chirps cluster decisions.

## Lifecycle

| Operation | Gate | Durable result |
|---|---|---|
| Bootstrap | target is absent or a valid schema-v1 store | all six v1 tables and `schema_version=1` |
| Commit | exact community checkpoint and unique intent | records, changes, checkpoint, and receipt in one transaction |
| Cache replace | exact source checkpoint and explicit record type | only that cache set is atomically replaced or cleared |
| Log append | unique typed keys and unique intent | all evidence rows append or none append |
| Crash reopen | file is recoverable by `redb` | last complete transaction; uncommitted writes are absent |
| Backup | destination does not exist | fsynced, schema-verified file installed atomically without overwrite |
| Restore | node is stopped and destination does not exist | verified backup copied to a fresh node path |

## Backup and restore

The host calls `NodeStorePort::backup_to` while the node is running. The adapter
serializes writes during the copy, syncs the temporary file, verifies schema 1,
then atomically links it into place. It never overwrites a destination.

Restore is a stopped-node operation: retain or rename the old database for
rollback, call `RedbNodeStore::restore_backup` into a fresh path, then start the
node. A schema mismatch or corrupt backup leaves the destination absent and the
old database untouched.

## Failure handling

- `CheckpointConflict`: obtain fresh canonical changes and recompute the Nim
  domain intent; do not force the checkpoint.
- `IntentConflict`: reject reuse of an idempotency key for different content.
- `UnsupportedSchema`: run the explicit migration supplied by the release that
  introduces the schema. There is no compatibility mode.
- `Engine` on reopen: stop the node and restore the last verified backup. Do not
  copy over the damaged file.

Run `cargo test -p nimino-store` for transaction, abrupt-exit recovery, schema,
backup, and restore drills.
