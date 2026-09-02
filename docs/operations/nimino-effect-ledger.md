# Nimino workflow effect ledger operations

## Normal execution

1. Initialize the ledger identity from the community, workflow, run, step,
   resolved effect digest, stable idempotency key, and #52 lease resource.
2. Call `planEffectClaim` with the live lease owner/fence and persist its
   `workflow_effect` next state through exact-checkpoint canonical commit.
3. Call `planEffectExecution`, persist `executing`, then settle the plan.
   Invoke external I/O only when settlement returns `efeExecuteExternal`, and
   pass the ledger's unchanged idempotency key to the provider.
4. Persist the exact success or failure receipt with `planEffectReceipt`.

Never execute from a plan before its canonical write succeeds. A failed claim,
execution-marker, or receipt write leaves the prior state authoritative.

## Crash recovery

- `claimed`: call `planEffectRecovery`. A still-live lease keeps the claim. An
  expired/replaced/recovered lease releases it to `pending`; loss of quorum does
  not release it.
- `executing` without a receipt: persist `unknown`. This includes both a crash
  immediately before external I/O and a crash after the provider accepted it;
  the system deliberately does not guess.
- `unknown`: automatic claim, execution, and receipt paths reject it. An
  authorized operator must inspect the provider by idempotency key, give a
  reason, and either attach a receipt or explicitly choose retry. Retry keeps
  the same key and creates a new fenced attempt.

## Verification

```bash
just nimino-effect-ledger-contract
cd nim/nimino_core && nim c -r --hints:off tests/test_effect_ledger.nim
```
