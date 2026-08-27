# Nimino lease and fencing operations

## Safety state

The committed #51 control entry is the only source of a lease grant. Each node
applies that entry with its own monotonic clock epoch and routes only while the
same granting term, voter epoch, and live quorum remain observable. Loss of any
fact closes routing and singleton effects locally.

Every effect request carries `resourceId`, `ownerId`, and `fenceToken` through
`authorizeSingletonEffect`. Consumers must treat every non-`lfeNone` result as
a hard stop; stale, future, and owner-mismatched attempts are never retried as
an unfenced operation.

## Partition and restart

On quorum loss, keep the lease state for diagnostics but stop serving it. After
quorum recovery, commit a fresh grant; its fence token is the previous watermark
plus one. Eligible nodes come only from #48's ready lease lane and are recorded
in normalized order in the command.

Recovery applies durable lease commands with `lamRecovery`. This restores the
last fence and control index but leaves `activeLease` empty. Never persist or
reconstruct a remaining wall-clock duration. A new process clock epoch requires
a fresh committed grant before routes or singleton effects resume.

## Verification

Run:

```bash
just nimino-lease-contract
cd nim/nimino_core && nim c -r --hints:off tests/test_lease_fencing.nim
```

The unit scenarios cover partition closure, expiry, deterministic failover,
idempotent replay, restart recovery, routing, and typed consumer rejection.
