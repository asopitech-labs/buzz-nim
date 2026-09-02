# Nimino workflow effect ledger contract

Version 1 stores each external workflow effect as replicated canonical
`workflow_effect` state. Identity includes community, workflow, run, step,
idempotency key, and the resolved effect digest. Chirps carries no ledger or
execution policy.

The host first persists a fenced claim, then persists `executing`. Only a
successful settlement of that second write returns `execute_external`. A crash
before the marker can safely release a claim after #52 denies its old lease; a
crash at or after the marker recovers as `unknown`. Unknown effects never retry
automatically and require an authorized operator to record a receipt or
explicitly request another attempt with the same idempotency key.

Run `just nimino-effect-ledger-contract` and `just nim-test`. #12 owns the hard
cutover from the old fire-and-forget executor; #29 remains the workflow decision
owner.
