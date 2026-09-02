# Nimino WSL service lifecycle v1

Nimino Relay runs as one systemd user service in the exact WSL profile fixed by
`wsl-support-v1`. Immutable releases live under
`$XDG_DATA_HOME/nimino/service-releases/<releaseSetId>` and an atomic `current`
symlink selects the active executable.

```bash
scripts/nimino-wsl-service.sh install \
  --release-set-id <64-hex-release-set-id> --bundle ./nimino-wsl-bundle
scripts/nimino-wsl-service.sh update \
  --release-set-id <64-hex-release-set-id> --bundle ./nimino-wsl-bundle
scripts/nimino-wsl-service.sh restart
scripts/nimino-wsl-service.sh uninstall
```

Install and update accept only a complete `nimino.wsl-bundle`. Its fixed binary
inventory, SHA-256 list, release-set identity, and embedded provenance are
verified before installation state is created. The former single `--relay`
input is deleted. A failed post-restart health check atomically restores the
prior target. The unit resolves `NIMINO_BOUNDARY_WORKER` inside the same
immutable release, so relay and Nim policy code cannot drift. Systemd owns the
whole process cgroup, so uninstall stops
descendants before removing the unit, tool links, and immutable executables.

Uninstall retains user data and state. `uninstall --purge-data` is the only
operation that deletes them; use it only for an explicit factory reset.
