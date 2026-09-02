# Nimino MCP execution contract

Version 1 keeps process, filesystem, and image I/O in the Rust adapter while
requiring an operation capability before any external side effect. The
comma-separated `NIMINO_MCP_CAPABILITIES` grant contains `process.exec`,
`filesystem.read`, `filesystem.write`, and/or `network.read`; an empty value
denies every external operation. Unset retains the full local developer
profile until #42 supplies the explicit production grants.

Filesystem grants are deliberately host-wide. Granting `process.exec` already
allows the selected shell to address the host, so a second path sandbox in
`read_file` would imply an isolation boundary that does not exist. A read-only
composition omits `process.exec` and `filesystem.write`.

Every capability decision writes `nimino.mcp-capability-audit/v1` structured
data to stderr before the operation. Shell timeout and cancellation terminate
and reap the process tree. Shell capture, text results, image input, decoded
allocation, and image output are bounded.

Run `just nimino-mcp-execution-contract` for source/contract drift and
`just nimino-mcp-framing` for a real stdio initialize, tool discovery, and
capability-denial exchange.
