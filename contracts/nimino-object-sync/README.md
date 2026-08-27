# Nimino object sync contract

Version 1 applies #50's bounded transport pattern to content-addressed media,
Git packs, and Git manifests without reimplementing event sync. Nim validates
manifests and owns eager/lazy fetch, revisioned pins, deterministic origin
selection, and grace-period GC. Chirps only carries chunks and facts.

`nimino-object-store` is the local byte adapter. It accepts at most 1 MiB per
chunk, fsyncs exact-offset partials, resumes them after restart, streams SHA-256
verification, and installs with an atomic no-clobber hard link only after size
and digest match. Existing matching objects are idempotent; conflicting or
incomplete bytes never replace an installed object.

Pins force fetch after rejoin and block GC. GC also retains manifest references,
active partials, unverified objects, and objects inside the epoch grace window.
Because raw CAS bytes may be shared across communities, the reference and pin
snapshots supplied to GC must be complete across all communities.

Run `just nimino-object-sync-contract`, `just nim-test`, and
`cargo test -p nimino-object-store` for large-object, checksum, missing-origin,
pin, GC, restart, and atomic-install coverage.
