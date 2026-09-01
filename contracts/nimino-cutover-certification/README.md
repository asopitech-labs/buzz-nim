# Nimino cutover certification v1

This contract is the source gate for issue #67. It binds the existing clean
checkout CI, real Chirps cluster evidence, Relay install smoke, Desktop matrix,
supported WSL qualification, compatibility-negative checks, and signed supply
chain evidence into one candidate definition.

`just cutover-certification-contract` verifies the wiring without publishing
anything. A candidate is certified only when the tag-triggered workflows pass
on their declared runners and emit `nimino-release-candidate`. Promotion remains
the separate, manual issue #68 workflow.
