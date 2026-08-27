# Nimino object sync operations

## Fetch and install

Validate the content-addressed manifest, #48 lifecycle lane, and local object
facts with `planObjectSync`. Fetch only returned actions from their exact source.
Use `LocalObjectStore::begin_partial` to resume, append at the returned offset in
chunks no larger than 1 MiB, then call `finish_partial`. Never expose a partial
path to media or Git readers.

A digest, size, or offset failure leaves the installed object namespace
unchanged. Abort a cancelled partial explicitly. A missing origin closes that
plan; retry discovery rather than weakening digest or tenant checks.

## Pins and GC

Persist `object_pin` transitions with exact revision CAS. Pinned objects are
requested after rejoin even in lazy mode. Build GC reference and pin snapshots
across every community sharing the CAS, then call `planObjectGc`. Delete only
the returned bounded digest list; referenced, pinned, partial, unverified, and
inside-grace objects remain.

## Verification

```bash
just nimino-object-sync-contract
cd nim/nimino_core && nim c -r --hints:off tests/test_object_sync_policy.nim
cargo test -p nimino-object-store
```
