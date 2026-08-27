# Nimino data operations contract

Version 1 separates repair authority from repair I/O. Nim compares bounded
health facts from 1, 3, or 5 nodes and accepts only a strict majority with the
same canonical checkpoint/digest, projection digest, object digest, and unknown
effect count. A mismatching or unreadable target is quarantined; no quorum
fails closed.

`nimino-data-ops verify` emits facts but never chooses a winner. After an
authorized operator supplies the Nim-selected source and target,
`nimino-data-ops repair` verifies a redb candidate and declared objects before
installing them. Existing stores and corrupt objects move to explicit
no-clobber quarantine paths. Repeating an already converged repair is a no-op.

Run `just nimino-data-ops-contract`, `just nim-test`, and
`just nimino-data-ops-scenarios`.
