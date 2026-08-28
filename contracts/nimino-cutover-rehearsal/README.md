# Nimino cutover recovery rehearsal

Issue #64 is the fail-closed recovery gate before the one-time Nimino cutover.
It creates no Buzz compatibility mode, dual write, down migration, or staged
public promotion. The snapshot is restored to new paths and serving ownership
is switched only after exact integrity comparison.

## Operator runbook

1. Freeze all store and object writes and record the maintenance-window
   boundary.
2. Run `nimino-data-ops verify` for every community. Stop if a node is
   unreadable, its selected authority does not match, or `unknownEffects` is
   nonzero.
3. Run `nimino-data-ops backup --store <store> --community <id>
   --object-root <objects> --object <sha256:size>... --backup-dir <new-dir>`.
   The command refuses overwrite, copies the redb store and declared immutable
   objects, verifies the backup inventory, and writes `manifest.json` last.
4. Run `nimino-data-ops restore --backup-dir <dir> --store <new-store>
   --community <id> --object-root <new-objects>`. Restore always targets new
   paths outside the bundle; create their parent directories first. Compare
   checkpoint, canonical/projection/object digests, and unknown effects with
   the backup output. The store copy also contains append-only logs. Its
   SHA-256 validates the immutable backup file itself; redb may rewrite physical
   housekeeping bytes while opening a restored copy, so restore integrity uses
   the semantic inventory plus redb's full-file integrity check.
5. Run `just nimino-cutover-rehearsal`. It also exercises failed release
   promotion rollback and writes the combined evidence to
   `target/nim/nimino-cutover-rehearsal.json`.

Any backup/restore mismatch, nonzero unknown effect, candidate promotion
failure, or post-cutover health failure is a stop-and-rollback decision. Stop
serving, restore the last verified bundle to a new target, atomically switch
serving ownership, and resume writes only after the exact digest comparison
passes. Never restore in place or boot a Buzz compatibility artifact.
