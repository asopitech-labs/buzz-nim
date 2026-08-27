# Nimino data contract

Version 1 is the pure contract between Nim domain owners and the storage,
replication, and projection work in Epic #10. It is a hard cut from the Buzz DB
API: no SQL shape, Rust type, Postgres transaction, or Chirps message is part of
the contract.

## Use cases and activities

1. A Nim domain service emits one community-scoped canonical commit. The data
   adapter compares the exact checkpoint, commits every mutation atomically,
   and returns the new checkpoint.
2. A caller submits a typed `by_key`, `page`, or canonical `changes` query. The
   adapter applies the mandatory community scope and bounded page contract.
3. A projection builder receives ordered canonical records through an exact
   checkpoint and digest. It emits one deterministic search, thread, or feed
   cache replacement carrying the same source identity.

## Responsibility map

| Decision or state | Owner | Artifact |
|---|---|---|
| Product acceptance and mutation content | Nim domain issues #26, #29, #31 | canonical write intent |
| Atomicity, checkpoint compare, persistence | data adapter issue #35 | transaction result |
| Replication order and convergence | sync issues #50 and #53 | canonical change stream |
| Search/thread/feed derivation | projection issue #55 | replaceable cache rows |
| Audit and operational evidence | append-only sink | log rows |
| Cluster negotiation | Chirps adapter only | no data-domain artifact |

Canonical records are managed truth. Cache records are discardable and
rebuildable. Log records are evidence and cannot authorize or reconstruct the
current domain state. Contract entities cross boundaries but own no I/O.

The only selected pattern is an Adapter boundary: downstream engines translate
these intents to their native storage operations. A repository hierarchy,
generic SQL expression tree, and runtime compatibility facade are deliberately
absent.

## Invariants and verification

- One write intent contains one data class, one community, and unique typed keys.
- Canonical and cache writes require exact checkpoints; logs are append-only.
- Queries carry a known record type, mandatory community, and bounded limit.
- Projection input and output identities match exactly and output keys are
  unique.
- `fixtures/postgres-schema-0033.json` classifies the 36 active logical tables
  in the final additive schema. It is cutover evidence only, not an importer or
  dual-runtime map.

Pure validators live in `nim/nimino_core/tests/test_data_contract.nim`.
`scripts/test-nimino-data-contract.mjs` keeps the JSON contract, Nim record
types, and all 36 active logical tables in `schema/schema.sql` aligned. Storage
integration and rebuild scenario tests belong to their owning downstream
issues.
