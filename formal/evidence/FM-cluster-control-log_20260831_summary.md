# Evidence: FM-cluster-control-log

## Date

2026-08-31 (Asia/Tokyo)

## Working tree

Base commit `1f6d3b2e`; the Nimino cutover changes were uncommitted during the run.

## Command

```bash
tlc -workers auto \
  -config formal/scenarios/NiminoControlLog_3Node.cfg \
  formal/tla/cluster/NiminoControlLog.tla
```

TLC2 version: `2026.08.21.155922` (revision `9787e65`, TLA+ Tools v1.8.0).

## Result

`Model checking completed. No error has been found.`

- 1,361,681 states generated
- 364,278 distinct states found
- 0 states left on queue
- complete graph depth 41

## Invariants

`TypeOK`, `AuthorityHasQuorum`, `QuorumsIntersect`,
`CommittedWithQuorum`, `SequentialVoterTransition`, `EpochsMonotonic`, and
`SnapshotCoversCommittedOnly` all passed.

Model SHA-256:
`4f3739c597ea27e9994279dc5332a7469c1dcbe3bf3d22b011490fd27ec64ea6`.

Scenario SHA-256:
`6a326c1e945217fb09e01d77b9c532f30dba8712ca604e4a70c18754bbd6a810`.

## Counterexample

None.

## Follow-up

#48, #49, and #51 must implement this contract. #52 adds lease/fencing without
making time a quorum or authority source.
