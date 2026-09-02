# Nimino convergence contract

Version 1 owns deterministic merge policy above #50's authenticated, bounded
sync frames. Identity observations are keyed by community and stable record ID.
The same ID with a different content digest removes the candidate from canonical
truth and produces a deterministic quarantine marker; arrival order cannot pick
one malicious variant as truth.

Replaceable records use the existing event-policy order: higher logical time,
then lower record ID. At the same logical time a tombstone wins over live state.
Versioned tombstones reject stale live records but allow a separately authorized
newer version; permanent tombstones always win. Restriction revisions are
monotonic, and equal-revision concurrency prefers ban, then timeout, then
release. A later authorized release remains possible through the moderation
policy.

Retention merges take component-wise maxima and forbid pruning beyond the
durable tombstone-protection watermark. These rules are commutative, associative,
and idempotent for valid non-colliding inputs. #55 rebuilds projections from the
winner set, while #12 removes central-DB ordering at the incompatible cutover.

Run `just nimino-convergence-contract` and `just nim-test` for randomized order,
partition/rejoin, collision, tombstone, ban, retention, and malicious-peer
coverage.
