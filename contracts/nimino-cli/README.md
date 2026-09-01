# Nimino CLI v1 contract

The Nim core owns the 115 canonical leaf command paths, their local/read/write
I/O class, whether relay identity is required, the downstream Nim domain
policy, JSON output contract, error category, retryability, and process exit
code. Rust remains the argument/JSON/YAML codec, Nostr signing, HTTP/WebSocket,
filesystem, and process adapter.

Every normal command writes JSON only. Human text is limited to `--help` and
`--version`; failures use JSON stderr with `error`, `message`, and `retryable`.
The sole executable name is `nimino`; `nimino` is neither a command alias nor a
build artifact.

Workflow reads and mutations route through the v1 command plan. Definition,
trigger, approval, and run decisions remain owned by `domain.workflow.policy`;
deletion uses `domain.event.policy`. #66 removes the current Rust command and
exit-policy branches atomically, and #12 owns the public cutover.

Run `just nimino-cli-contract`, `just nim-ci`, and `just nim-boundary-test`.
