# Legacy mesh and Redis replacement manifest

This manifest freezes the #58 decision boundary before #66 removes code. Every
Rust file in `buzz-relay-mesh` and `buzz-pubsub`, every direct relay consumer,
and the Cargo wiring has one action, one replacement, and one target owner.

There is no compatibility mode. `buzz-relay-mesh` is fully replaced by the
`nimino-chirps` transport adapter plus Nim-owned cluster policy. Shared Redis is
absent from the target topology. Presence, typing, and admission windows may
survive only as explicitly non-authoritative, single-node caches; canonical
state, replication, moderation, replay protection, and effect execution never
depend on them.

Run `just legacy-control-manifest-contract`. #66 must remove every listed old
path in one atomic cleanup and may not leave a forwarding wrapper.
