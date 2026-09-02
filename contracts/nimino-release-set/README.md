# Nimino immutable release set v1

One JSON document is the authority for a release. It pins the release version,
tag, resolved source commit, four source components, and every artifact by
version, SHA-256, size, and filename. Promotion accepts the document's
`releaseSetId`; it does not accept separate mutable version or artifact inputs.

The four source components are derived from the checked-out tree:

- Nim core: Nim package version plus a deterministic source-tree digest.
- Rust workspace: workspace version plus `Cargo.toml`/`Cargo.lock` digest.
- Alopex Chirps: exact crate version and crates.io checksum.
- Boundary schema: protocol version and schema bundle digest.

Create and verify a candidate:

```bash
node scripts/nimino-release-set.mjs create \
  --version 1.0.0 --tag nimino-v1.0.0 --commit <40-hex-commit> \
  --artifact cli:0.1.0:dist/nimino \
  --output dist/nimino-release-set.json

node scripts/nimino-release-set.mjs verify \
  --manifest dist/nimino-release-set.json \
  --resolved-tag-commit <40-hex-commit> \
  --artifact-dir dist
```

Verification rejects moved tags, changed inputs, missing or mismatched
artifacts, version downgrades, and same-version content changes. An identical
rerun is accepted.
