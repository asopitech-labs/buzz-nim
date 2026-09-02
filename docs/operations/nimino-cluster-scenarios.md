# Nimino cluster scenario operations

## Purpose

The #56 gate runs one fixed contract against 1, 3, and 5 real Alopex Chirps
UDP/QUIC mTLS nodes plus the production Nim worker. It proves transport
isolation, drain policy, OS process kill/reap, rejoin, stable identity,
incremented incarnation, and cleanup. It does not claim data convergence; #59
owns that proof.

## Run and reproduce

```bash
just nimino-cluster-scenarios
```

The contract fixes failure seed `202608280056`. The same seed and node count
always select the same non-primary failure node. For the portable partition
step that node starts without a discovery seed, so real QUIC delivery fails
without requiring firewall or network-namespace privileges. Its dedicated test
process is then killed and reaped before restart against the primary seed.

## Evidence and cleanup

Success writes `target/nim/nimino-cluster-scenarios.json`. Each topology records
the selected failure index, partition result, identity continuity, incarnation
before/after, drained node count, and rebound UDP socket count. CI uploads that
file as `nimino-cluster-scenarios` even when the job fails.

Every scenario drains all live nodes through `domain.cluster.lifecycle`, awaits
explicit mesh shutdown, rebinds every original UDP address, and shuts down the
supervised Nim worker. A missing artifact means the suite did not reach full
cleanup and is not release evidence.
