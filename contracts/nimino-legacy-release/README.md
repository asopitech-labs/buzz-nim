# Legacy release authority retirement

Issue #65 freezes the complete cutover disposition for repository workflows,
release scripts, referenced GitHub credentials, and Block/Square deployment
authorities. It does not delete or disable them: Issue #66 performs that atomic
source/build/runtime cleanup, and Issue #68 alone publishes Nimino.

The machine-readable source is `v1/manifest.json`. `RELEASING.md` is the only
operator release runbook. Upstream documents and workflows are evidence to
delete or rewrite, never a second release authority.

## Dry-run decommission checklist

Run these read-only checks before the #66 cleanup commit:

1. `just legacy-release-deletion-contract` must report an exact workflow and
   GitHub secret/variable inventory with zero unclassified internal-reference
   paths.
2. Confirm `.github/workflows/nimino-platform-release.yml`,
   `nimino-relay-release.yml`, and `nimino-promote.yml` are the only release
   workflows marked `keep`; CI remains a non-publishing gate.
3. Confirm every `delete` script is used only by a workflow or command also
   marked `delete`. The shared Nimino artifact helpers remain.
4. Inventory GitHub configuration with `gh workflow list`, `gh secret list`,
   `gh variable list`, and the protected `nimino-production` environment. Do
   not print secret values.
5. Confirm the external Square repositories and their Buildkite, Artifactory,
   ECR, ArgoCD, and Blox hooks cannot publish from this fork. If credentials
   were copied into the fork, revoke them at their provider as well as deleting
   the GitHub names in the manifest.
6. Preserve the shared Nimino Apple and Tauri signing credentials. Delete only
   `CODESIGN_S3_BUCKET`, `OSX_CODESIGN_ROLE`, the legacy release-tagger App
   credentials, and `GHCR_SPRIG_IMAGE` after #66 removes all consumers.
7. Re-run the contract after cleanup, then run the #67 clean-clone and
   compatibility-negative matrix. Do not create a `nimino-v*` tag.

## Stop conditions

Stop the cutover if an unclassified workflow, script reference, credential, or
internal target appears; if a retained workflow can publish a Buzz/Block
artifact; or if a credential marked for deletion still has a consumer. There
is no compatibility fallback. Fix the inventory or cleanup commit and rerun the
gate.

After #66, update this manifest atomically to `phase: removed`, remove deleted
paths from the exact inventories, and retain the denylist/negative fixtures that
prove predecessor targets cannot return.
