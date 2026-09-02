# Nimino core boundary contract

This directory is the machine-readable source of truth for communication
between the Rust host adapters and the Nimino core worker. Version `v1` is a
deliberate break from Nimino internals; there is no legacy or dual-runtime mode.

The transport is a long-lived child process. Each message is UTF-8 JSON framed
by a four-byte unsigned big-endian payload length. Standard output is reserved
for frames and standard error is reserved for diagnostics. A frame may not
exceed 1 MiB.

The host must complete `system.hello` before sending another operation. The
protocol version, schema hash, worker role, frame limit, and max-inflight value
must match exactly. There is no downgrade or Rust domain fallback. Only one
request is active in a worker at a time. The host owns bounded queuing and
process lifecycle; Nim owns operation semantics and all future domain policy.
The call deadline starts before bounded enqueue. Timeout, cancellation, caller
abandonment, and shutdown kill and reap the active worker. A reap failure is
terminal and forbids replacement.

Versioned artifacts:

- `request.schema.json` and `response.schema.json`: wire envelopes.
- `errors.json`: stable remote and host failure taxonomy.
- `lifecycle.json`: states, transitions, and recovery actions.
- `schema.sha256`: digest of the four schema/lifecycle files in the order
  documented in that file.
- `fixtures/`: language-neutral contract examples consumed by tests.

Changing an existing field or semantic requires a new version directory. Each
operation is a tagged variant with its own payload shape. New domain operations
may be added within v1 only by extending that union and adding typed adapters
on both sides; arbitrary string/value calls are not a public host API.
