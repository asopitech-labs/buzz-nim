# Nimino anti-entropy operations

## Session flow

1. Read `NodeStorePort.canonical_checkpoint` and recompute the lowercase SHA-256
   prefix digest from its ordered `changes` feed.
2. Accept a remote digest only after #48 allows the data-sync lane.
3. Send the range request returned by `nextRangeRequest`; never widen its record
   or byte limits.
4. Verify each record and the rolling canonical range with
   `nimino_store::verify_range_digest`, then pass `digestVerified=true` to
   `planRangeBatch`.
5. Commit the returned records with `expectedCheckpoint`, install
   `inflightState`, and call `settleRangeBatch` with the actual committed
   checkpoint.

The protocol permits one applying batch. A full Chirps queue remains
backpressure, not a reason to buffer elsewhere. A timeout or cancel closes the
session; late settlement cannot reopen it.

## Resume and isolation

After restart, initialize from the durable canonical checkpoint and recomputed
digest. An already committed final batch is acknowledged idempotently if its
result matches that state. Never accept a checkpoint from memory or a peer as
local durability evidence.

Every digest, range request, range batch, and cancel frame must exactly match
protocol version, session, community, sender, and receiver. On a mismatch,
discard the frame and keep the prior checkpoint. Digest divergence is handed to
#53; do not force a range onto a conflicting history.

## Verification

```bash
just nimino-sync-contract
cd nim/nimino_core && nim c -r --hints:off tests/test_sync_protocol.nim
```
