# Nimino promotion contract

Issue #63 promotes only a fully qualified `nimino.release-set`. The release-set
ID is the sole operator input; version or tag strings are resolved from that
content identity and cannot select different bytes.

Every candidate carries a deterministic SPDX 2.3 SBOM, complete SHA-256 index,
keyless signatures, GitHub provenance, and SBOM attestations. Promotion rejects
missing evidence, downgrades, and same-version byte changes. A retry is a no-op
only for identical bytes. If updating the rolling updater state partially
fails, the last known-good `latest.json` and `promotion.json` are restored.

The protected `nimino-production` GitHub environment is the human approval
boundary. This workflow prepares the independent Nimino cutover; issue #68 owns
the first physical promotion and post-cutover smoke test.
