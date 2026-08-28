# Nimino Tauri adapter contract v1

The target desktop backend is an adapter boundary, not a second product-policy
runtime. Rust may retain platform access, bounded network I/O, and measured
performance paths. Nim owns authorization, lifecycle, routing, and product
state decisions; TypeScript renders state and requests typed adapter effects.

The v1 manifest classifies every Tauri command source file. Its checker rejects
unregistered commands, unclassified files, duplicate classifications, forbidden
server-policy dependencies, and missing hot-path evidence. `move` and `delete`
entries remain only until the incompatible #12/#66 cutover; there is no
compatibility mode.

Run `just nimino-tauri-contract` for the inventory/dependency gate and
`just nimino-tauri-native-test` for the WebSocket, unread projection, and
terminal frame integration evidence.
