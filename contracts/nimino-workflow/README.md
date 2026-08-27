# Nimino workflow policy contract

`domain.workflow.policy` owns normalized definition validation, condition
evaluation, current-step planning, effect selection, and versioned run-state
transitions. Rust adapters decode YAML/JSON, load verified facts, execute the
returned effect through a port, and persist transitions with revision and
transition-ID compare-and-swap.

Planning binds `send_message` to the workflow's durable channel, or to the
trigger channel for an unbound workflow. Cross-channel overrides and threaded
replies without a trigger message fail closed, so the executor port receives a
fully selected destination rather than making another policy decision.

The contract is cutover-ready for issue #12. It has no Buzz compatibility or
Rust policy fallback. Durable effect-ledger storage remains issue #57.

The v1 condition language is deliberately bounded: typed scalar variables,
parentheses, `!`, `&&`, `||`, comparisons, and `str_contains`,
`str_starts_with`, `str_ends_with`, and `str_len`. Templates support trigger
and prior-step fields plus Unicode-safe `truncate(N)` and full `npub` encoding.
This is the Nimino v1 language; arbitrary `evalexpr` extensions are not a
compatibility requirement.
