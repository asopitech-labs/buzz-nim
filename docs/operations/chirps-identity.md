# Chirps node identity and certificate lifecycle

Nimino requires explicit mutual TLS for every production Chirps node. The
`nimino-chirps` adapter accepts only a DER node certificate, PKCS#8 DER private
key, one or more DER trust anchors, and a persistent identity path. There is no
production self-signed fallback.

## Startup contract

1. Mount the certificate, private key, and complete trust-anchor set.
2. Restrict the private key to owner-only access (`0600` or stricter on Unix).
3. Keep the identity path on durable node-local storage; never share it between
   nodes.
4. Call `NodeConfig::prepare` before starting the Chirps runtime. It validates
   readable, non-empty material and loads or creates the stable NodeId.
5. Pass the same config to the runtime adapter implemented by #43. Admission
   and capability decisions remain in the Nimino control plane under #48.

Chirps upgrades the initial 16-byte identity record to its 24-byte
identity/incarnation record on first runtime start. The adapter accepts both
formats and always derives the same NodeId.

## Certificate rotation

Stage a complete new certificate/key/trust generation, then replace all
configured paths before restarting the node. Run `NodeConfig::prepare` against
the new generation before stopping the old process. A failed preflight leaves
the durable identity unchanged; restore the prior generation and retry.

Chirps fingerprints the configured files and reloads changed material on the
next runtime start. The integration test proves that a warmed certificate cache
accepts the rotated generation and rejects a peer without a shared trust
anchor.

## Failure guide

| Failure | Meaning | Operator action |
|---|---|---|
| `TrustAnchorsRequired` | no peer trust was configured | mount the cluster CA or explicit peer anchors |
| `CertificateMissing` / `PrivateKeyMissing` / `TrustAnchorMissing` | a generation is incomplete | restore the complete generation; do not start |
| `InvalidMaterial` | a path is empty or not a regular file | replace it with the expected DER material |
| `InsecurePrivateKeyPermissions` | group/other access is present | restrict the key to `0600` or stricter |
| `InvalidIdentity` | identity storage is truncated or foreign | restore that node's backup; do not generate a replacement silently |
| `InsecureIdentityPermissions` | persisted identity is overexposed | restrict the file to `0600` or stricter |
| `Io` | storage or permission operation failed | repair the reported path and retry |

Do not delete or regenerate a valid identity during certificate rotation. A new
identity represents a different transport node and must pass #48 admission.
