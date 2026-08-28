# Nimino relay release v1

One `nimino-v<semver>` candidate builds the native amd64 and arm64 relay image,
the `nimino` OCI chart, and the Compose bundle. The pipeline installs the chart
into a clean kind cluster and consumes the real three-node Chirps and data
convergence evidence before it creates a verified release set.

The image, chart, Compose bundle, release-set manifest, and evidence are pinned
by digest. OCI artifacts and blobs receive keyless Sigstore signatures, and the
release-set manifest receives a GitHub provenance attestation. No `latest`,
`main`, `relay-v`, `chart-v`, Block, or Buzz publication alias is produced.

This issue publishes immutable candidates only. Promotion/rollback belongs to
#63; physical deletion of disabled predecessor workflows belongs to #65.
