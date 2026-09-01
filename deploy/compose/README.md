# Nimino Docker Compose deployment

This is the single-node/VPS deployment bundle. It is intentionally separate from
the root `docker-compose.yml`, which remains local development infrastructure.

## Quick start

```bash
cd deploy/compose
cp .env.example .env
$EDITOR .env       # replace every CHANGE_ME value
# Add DER chirps/tls.crt, PKCS#8 DER chirps/tls.key, and DER chirps/ca.crt.
# chmod 600 chirps/tls.key
./run.sh start
```

For a public VPS with automatic Let's Encrypt certificates:

```bash
cd deploy/compose
NIMINO_COMPOSE_TLS=true ./run.sh start
```

The bootstrap script should eventually replace manual `.env` editing for normal
users. It is responsible for generating stable secrets and, optionally, an owner
keypair.

## Production notes

- Requires Docker Compose v2.24.4 or newer; the TLS override uses Compose's
  `!reset` tag to remove the direct relay port when Caddy terminates HTTPS.
- `NIMINO_IMAGE_DIGEST` is required and must be the SHA-256 digest from a
  verified release set. Compose fixes the repository to
  `ghcr.io/asopitech-labs/nimino`; tags and predecessor repositories cannot be
  substituted.
- Keep `NIMINO_RELAY_PRIVATE_KEY`, `NIMINO_GIT_HOOK_HMAC_SECRET`, database, and
  S3 secrets stable across restarts.
- `RELAY_OWNER_PUBKEY` is intentionally not prefixed with `NIMINO_`; it must be a
  64-character hex Nostr pubkey when closed relay mode is enabled.
- `NIMINO_AUTO_MIGRATE` is opt-in. Set `NIMINO_AUTO_MIGRATE=true` or run
  `nimino-admin migrate` before starting the relay when bootstrapping a fresh
  database. Auto-migration requires an image that includes embedded SQLx
  migrations.
- The stack uses PostgreSQL, MinIO, a Git work volume, and a persistent
  per-node Chirps sync-store volume. Ephemeral socket delivery and admission
  caches stay inside each relay process.
- `chirps/tls.crt`, `chirps/tls.key`, and `chirps/ca.crt` are mandatory DER
  trust material. The private key must not be group/world-readable. Keep the
  identity file in the cluster volume stable across restarts.
- The bundled Compose stack fixes the relay endpoint to `http://minio:9000` and
  `NIMINO_S3_ADDRESSING_STYLE=path`: Docker DNS resolves `minio`, not
  `<bucket>.minio`. It is not configurable for an external S3 provider through
  `.env`; use the Helm chart or a custom Compose configuration for providers
  such as new Railway Storage Buckets that require `virtual` addressing.

Run `./run.sh backup-hint` for the backup checklist.

## Validation

Before sharing an install link publicly, verify a fresh install with:

```bash
cd deploy/compose
cp .env.example .env
$EDITOR .env
./run.sh config
./run.sh start
curl -fsS "http://127.0.0.1:$(grep -E '^NIMINO_HTTP_PORT=' .env | cut -d= -f2-)/_liveness"
./run.sh status
```
