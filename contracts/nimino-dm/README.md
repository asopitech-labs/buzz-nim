# Nimino DM policy v1

`domain.dm.policy` owns two pure decisions: immutable participant-set
mutations (`open`, expanded `add`, per-viewer `hide`) and participant/viewer
access (`read`, `write`, visibility snapshot). Rust retains NIP-17
encryption/signing, event codecs, tenant-scoped fact reads, participant hashing,
transactions, and effect execution.

`hide` is a viewer-local presentation state: it does not change the immutable
participant set or revoke that participant's read/write eligibility.

The Rust decision sites listed in the contract are shrunk during Issue #12's
incompatible cutover. No legacy DM path, fallback, or dual runtime is added.
