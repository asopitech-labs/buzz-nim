# Nimino moderation policy v1

`domain.moderation.policy` owns report intake, community ban/timeout
transitions, active-expiry evaluation, moderator authority, and open-report
resolution. The same policy evaluates ban admission (including attested-owner
cascade) and timeout write denial. Duplicate commands, self-targeting, missing
provenance, and cross-community facts fail closed.

Rust retains signature/tag codecs, tenant-scoped fact acquisition, DB locks,
compare-and-set guards, audit persistence, notification/disconnect delivery,
and enforcement of the returned decision. A pubkey-only report target is
explicitly resolved into the request community by the Rust fact adapter before
the policy call.

The Rust decision sites listed in the contract are removed during Issue #12's
incompatible cutover. No legacy moderation path, fallback, or dual runtime is
added.
