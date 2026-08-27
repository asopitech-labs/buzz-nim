# Nimino lease and fencing contract

Version 1 is the single Nim policy gate for singleton ownership, routing, and
side effects. A lease grant is only activated from a committed #51 control-log
entry. Chirps peer observations and uncommitted leader claims never authorize a
grant.

Every live route and singleton side effect also requires a current quorum fact,
the granting term and voter epoch, the same process-local monotonic clock epoch,
and an unexpired lease. Consumers submit both owner and fence token through
`authorizeSingletonEffect`; stale, future, and owner-mismatched attempts receive
distinct typed rejections.

Each committed grant increments the fence token, including renewal and
failover. Eligible ready nodes are normalized and the lexicographically first
node is selected, so every replica derives the same v1 owner. Recovery replays
the fence watermark but deliberately does not reactivate a timed lease; a fresh
committed grant is required after restart.

Run `just nimino-lease-contract` for the ownership and boundary contract and
`just nim-test` for partition, expiry, replay, and routing scenarios. #56 owns
the multi-node process harness and #12 removes the old Redis/mesh paths at the
single incompatible cutover.
