# Nimino convergence operations

## Merge order

Run `observeIdentity` before a replicated record enters canonical merge. A
different digest for the same community/record ID means quarantine: remove any
active canonical candidate for that ID and append both variants to evidence.
Never choose the locally observed variant.

Pass valid records to `mergeReplicaRecord`. Persist only its winner. Versioned
tombstones block stale/equal live records; permanent tombstones block every live
record. Restrictions merge by revision, with ban and timeout winning only an
equal-revision race. Newer release transitions must already have passed the
Nimino moderation policy.

Merge retention watermarks with `mergeRetention`. Refuse any state where
`prunedThrough` exceeds `tombstoneProtectedThrough`; physical deletion cannot
run ahead of durable anti-resurrection evidence.

## Fault handling

- Scope, digest, or version errors: reject the peer frame and retain current
  canonical state.
- Identity collision or fact conflict: remove the candidate from serving paths,
  persist quarantine evidence, and alert operators.
- Partition/rejoin: replay all verified candidates in any order; valid inputs
  converge to the same winner.

## Verification

```bash
just nimino-convergence-contract
cd nim/nimino_core && nim c -r --hints:off tests/test_convergence_policy.nim
```
