# Verify and repair a Nimino replica

## Verify before changing state

Run `verify` on each of the 1, 3, or 5 replicas. Supply every object from the
canonical manifest as `--object DIGEST:SIZE`; omit object flags only when the
manifest is empty.

```bash
nimino-data-ops verify \
  --store /var/lib/nimino/node.redb \
  --community COMMUNITY_ID \
  --object-root /var/lib/nimino/objects \
  --object SHA256:SIZE
```

Feed the JSON facts to `planRepairVerification`. Repair is forbidden unless a
strict majority reports the same canonical, projection, object, and effect
snapshot. `unknownEffects > 0` must be resolved through the #57 manual
reconciliation flow first.

## Repair an isolated target

Persist the Nim repair marker, then execute exactly the returned source/target
directive:

```bash
nimino-data-ops repair \
  --source-store /srv/node-a/node.redb \
  --target-store /srv/node-c/node.redb \
  --quarantine-store /srv/node-c/quarantine/node.redb \
  --community COMMUNITY_ID \
  --source-object-root /srv/node-a/objects \
  --target-object-root /srv/node-c/objects \
  --object-quarantine-root /srv/node-c/quarantine/objects \
  --object SHA256:SIZE
```

The command never overwrites quarantine paths. Objects copy resumably and are
verified before the redb candidate replaces the target. If capacity is
exhausted or the process is killed, the old target remains authoritative;
repeat the same command after correcting the fault. Serialize repair commands
for a target node.

Finally run `verify` again on all nodes and settle the Nim repair state only
when canonical, projection, and object digests match.
