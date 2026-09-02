# Nimino agent bundle v1

The agent bundle is a clean directory assembled from one verified
`nimino.release-set`. It contains four real executables and no multicall binary,
symlink, or compatibility alias:

- `nimino`
- `nimino-acp`
- `nimino-agent`
- `nimino-dev-mcp`

Each installed executable retains the version, digest, size, and source
artifact ID pinned by the release set. Other release artifacts are neither
required locally nor copied into the bundle.

```bash
node scripts/nimino-agent-bundle.mjs compose \
  --release-set dist/nimino-release-set.json \
  --resolved-tag-commit <40-hex-commit> \
  --artifact-dir dist/components \
  --output dist/nimino-agent-bundle
```

The output path must not already exist. This prevents an old binary from
surviving a rebuild and appearing in the inventory.
