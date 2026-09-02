# Nimino control-log store operations

`ControlLogStorePort` persists local control-plane bytes only. Chirps transports
messages, the Nim state machine decides elections/quorum/application, and the
canonical change feed handles product data. None of those authorities are
inferred by this adapter.

## Recovery order

1. Open the node store; `redb` repairs an interrupted transaction if needed.
2. Call `recover_control_state`.
3. Install the returned snapshot in Nim when present.
4. Replay returned entries in index order only through `commit_index`.
5. Keep later entries as uncommitted replication state; never apply them.

Recovery fails closed if metadata points beyond durable state, the applied
index exceeds commit, or the post-snapshot suffix is not contiguous.

## Write lifecycle

- Persist term/vote/commit/apply metadata with revision CAS. On
  `ControlMetadataConflict`, reload and let the Nim state machine recompute.
- Append through `replace_control_suffix`. It may truncate only after the
  committed index and never before an installed snapshot.
- Install a locally produced or validated remote snapshot with
  `install_control_snapshot`. Installation and covered-prefix compaction share
  one fsynced transaction; a newer remote snapshot advances commit/apply
  recovery watermarks to its included index.

`cargo test -p nimino-store` exercises append/reopen, metadata CAS, suffix
replacement, local and remote snapshot installation, table isolation, and a
process-exit torn-write drill.
