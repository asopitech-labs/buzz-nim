# Unified cutover readiness

This is the single index of workstream keep/delete decisions consumed by the
hard-cut gate. It composes existing versioned evidence instead of copying each
domain manifest. Every Epic #2–#11 names retained value, predecessor deletion,
implementation owners, the #66 cleanup owner, and runnable proof.

`v1/manifest.json` freezes source readiness only. It does not claim that GitHub
issues were closed, repository settings exist, a supported WSL host passed, or
a release was published. Those observable external gates remain #67 and #68.
The tracker audit must show every workstream Epic closed before promotion, but
this repository contract never mutates GitHub issue state.

Run `just cutover-readiness-contract`. Any missing proof file or recipe,
unclassified Epic, compatibility mode, or source blocker stops #66.
