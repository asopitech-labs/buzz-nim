# Nimino agent policy v1

Nim owns persona precedence, ordered event-trigger selection, and the
cancel/restart lifecycle. Rust decodes persona files and verified Nostr facts,
frames ACP JSON-RPC over bounded NDJSON, supervises subprocesses, and executes
the returned lifecycle action.

Trigger expressions reuse the bounded Nim workflow expression language. Rules
are ordered and first-match wins; an invalid expression fails the whole match
closed. Persona trigger objects replace pack defaults shallowly, while absent
fields inherit their pack or built-in default.

ACP is an adapter contract, not a second policy runtime. Cancellation timeout
returns `reap_and_wait`; a replacement cannot spawn before the retry deadline,
and stale attempt or turn identifiers are rejected. #66 removes the registered
Rust policy sites and #12 performs the incompatible public cutover.

Run `just nimino-agent-contract`, `just nim-ci`, `just nim-boundary-test`, and
`cargo test -p nimino-acp -p nimino-agent -p nimino-persona`.
