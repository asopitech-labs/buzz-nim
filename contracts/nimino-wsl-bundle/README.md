# Nimino WSL install bundle v1

The WSL bundle is composed only from artifacts pinned by one verified Nimino
release set. It contains the relay, data verify/repair adapter, CLI, ACP, agent,
and MCP binaries plus the release provenance and exact SHA-256 inventory.

`scripts/nimino-wsl-service.sh` accepts this bundle as its sole install/update
input. It stages the complete release, atomically switches `current`, rolls back
failed health checks, and removes every owned binary link and process on
uninstall. There is no single-relay or Buzz compatibility install path.

`just wsl-bundle-e2e` runs the clean install, update, failure, and uninstall
workflow on real WSL ext4. `just wsl-bundle-certify` additionally requires the
exact `wsl-support-v1` host, WSL, WSLg, network, systemd, and Secret Service
configuration before producing release-candidate evidence.
