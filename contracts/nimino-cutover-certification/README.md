# Nimino cutover certification v1

This contract is the source gate for issue #67. It binds the existing clean
checkout CI, real Chirps cluster evidence, Relay install smoke, Desktop matrix,
supported WSL qualification, compatibility-negative checks, and signed supply
chain evidence into one candidate definition.

`just cutover-certification-contract` verifies the wiring without claiming
readiness. `just cutover-certify-source` is the tag gate and fails while the
readiness manifest contains a source blocker. A candidate is certified only
when that gate and the tag-triggered workflows pass on their declared runners
and emit `nimino-release-candidate`. Promotion remains the separate, manual
issue #68 workflow.
