# Rust responsibility contract v1

Rust is a boundary language in Nimino: it owns typed process boundaries,
crypto/codecs, bounded I/O, and measured native hot paths. Nim owns every
domain and product decision. TypeScript owns presentation state.

The manifest classifies every Cargo package. Every `.rs` source inherits the
action of its nearest package; Tauri command files additionally use the finer
[`nimino-tauri`](../nimino-tauri/README.md) inventory. The checker rejects new
packages, unowned Rust sources, upward dependency edges, policy wire operations
outside `nimino-boundary`, and untracked dependencies on the legacy mesh.

`keep` packages are already bounded. `shrink` packages retain only their
adapter or entry-point responsibility at cutover. `buzz-relay-mesh` is the one
tracked replacement and is removed atomically by #66; there is no compatibility
mode.

Run `just rust-responsibility-contract`.
