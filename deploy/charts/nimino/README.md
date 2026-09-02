# Nimino Helm chart

This chart deploys the Nimino relay with PostgreSQL and S3-compatible object
storage. Live socket delivery and admission caches are node-local. Cluster
negotiation uses Chirps; Redis is not part of the runtime or chart.

## Profiles

| Profile | Intended use | State services |
| --- | --- | --- |
| Production | GitOps-managed or multi-node deployments | External PostgreSQL and S3, existing Kubernetes Secret |
| Quickstart | Evaluation and CI | Bundled PostgreSQL and MinIO, chart-generated secrets |

Quickstart:

```bash
openssl req -x509 -newkey rsa:2048 -nodes -days 1 \
  -subj /CN=nimino-chirps -keyout /tmp/nimino-chirps.pem \
  -out /tmp/nimino-chirps.crt
openssl x509 -in /tmp/nimino-chirps.crt -outform DER \
  -out /tmp/nimino-chirps.crt.der
openssl pkcs8 -topk8 -nocrypt -in /tmp/nimino-chirps.pem -outform DER \
  -out /tmp/nimino-chirps.key.der
kubectl create namespace nimino --dry-run=client -o yaml | kubectl apply -f -
kubectl -n nimino create secret generic nimino-chirps \
  --from-file=tls.crt=/tmp/nimino-chirps.crt.der \
  --from-file=tls.key=/tmp/nimino-chirps.key.der \
  --from-file=ca.crt=/tmp/nimino-chirps.crt.der
helm dependency build deploy/charts/nimino
helm upgrade --install nimino deploy/charts/nimino \
  --namespace nimino \
  --set quickstart=true \
  --set postgresql.enabled=true \
  --set minio.enabled=true \
  --set relayUrl=wss://nimino.example.test \
  --set ownerPubkey=<64-lowercase-hex-pubkey>
```

Production should set `secrets.existingSecret`, `externalPostgresql.url`, and
the `s3` values. `cluster.tlsSecret` must name a Secret containing DER
`tls.crt`, PKCS#8 DER `tls.key`, and DER `ca.crt`. Pin `image.digest` to the
digest certified by the Nimino release workflow; tags alone are not a release
identity.

## Required state

- PostgreSQL stores canonical relational state and durable lifecycle fences.
- S3-compatible storage holds media and Git object state.
- Each relay keeps only bounded ephemeral delivery, presence, replay, and rate
  limit state in memory.
- The relay is a StatefulSet. Each ordinal owns one `ReadWriteOnce` sync-store
  PVC; do not share that volume between replicas.
- The chart derives one fixed 16-byte Chirps identity per StatefulSet ordinal
  and injects the matching voter set. A persisted identity mismatch fails the
  init container instead of silently changing cluster authority.
- Chirps supplies authenticated peer negotiation and secure transport only.
  Nimino owns admission, replication, synchronization, conflict policy, and
readiness decisions.

When autoscaling is enabled, `minReplicas` must retain a majority of
`maxReplicas`; all possible ordinals belong to the fixed voter set. Change that
set through the replicated control reconfiguration procedure, not by editing a
live PVC identity.

Run schema migration before a production upgrade unless
`migrate.autoMigrate=true` is an intentional deployment choice.

## Validation

```bash
helm lint deploy/charts/nimino
helm template nimino deploy/charts/nimino \
  -f deploy/charts/nimino/tests/fixtures/ha-values.yaml
```

The templates reject incompatible ingress modes, malformed image references,
missing PostgreSQL sources, and invalid S3 configuration. See `values.yaml`,
`values.schema.json`, and `examples/` for the complete surface.

## Backup and recovery

Back up PostgreSQL, the S3 bucket, the relay private key, Chirps trust material,
the Git hook HMAC secret, and every ordinal's sync-store PVC in the same
maintenance window. Restore state before starting relay replicas, then verify
`/_readiness` and the release smoke workflow.
