# Chirps runtime operations

`nimino-chirps` is a bounded transport adapter. It starts Chirps on one
dedicated Tokio runtime thread and exposes opaque direct messages, broadcasts,
subscriptions, and observed peer hints. It does not own cluster admission,
protocol envelopes, database state, replication, or product decisions.

## Lifecycle

1. Build `NodeConfig` with explicit certificate, private-key, trust-anchor,
   identity, bind, and seed paths.
2. Call `MeshRuntime::start`; startup returns only after Chirps is listening.
3. Clone `MeshClient` for send, broadcast, peer observation, and subscriptions.
4. Call `MeshRuntime::stop` during orderly shutdown. It signals the worker,
   aborts retained upstream tasks by closing the dedicated Tokio runtime, joins
   the thread, and releases the UDP socket.

Dropping `MeshRuntime` only signals best-effort shutdown. Operators that need a
confirmed clean stop must await `stop`.

## Capacity and failure signals

- Command queues default to 64 entries; inbound subscription queues default to
  256. Both accept `1..=4096`.
- Messages are capped at 1 MiB before they enter Chirps.
- `Backpressure` means the command queue was full; retry with bounded jitter or
  reject work upstream.
- `SubscriberLagged` means a consumer lost overwritten messages; resynchronize
  through the domain protocol once #10 owns that flow.
- `Stopped` means no new work is accepted.
- `Transport` is an individual Chirps operation or startup failure.

`peers()` returns observed reachability hints. Never use it to authorize a node,
elect a leader, or decide canonical ownership; those responsibilities belong to
the admission and control-plane protocols.

## Verification

Run `cargo test -p nimino-chirps`. The runtime integration tests exercise three
mTLS nodes, direct send, broadcast, lag reporting, explicit shutdown, same-port
restart, and stable identity. Run `just chirps-contract` to reject scope or API
drift.
