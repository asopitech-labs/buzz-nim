# Nimino WSL service lifecycle v1

Nimino Relay runs as one systemd user service in the exact WSL profile fixed by
`wsl-support-v1`. Immutable releases live under
`$XDG_DATA_HOME/nimino/service-releases/<releaseSetId>` and an atomic `current`
symlink selects the active executable.

```bash
scripts/nimino-wsl-service.sh install \
  --release-set-id <64-hex-release-set-id> --relay ./nimino-relay
scripts/nimino-wsl-service.sh update \
  --release-set-id <64-hex-release-set-id> --relay ./nimino-relay
scripts/nimino-wsl-service.sh restart
scripts/nimino-wsl-service.sh uninstall
```

Install and update require an executable with content unique to the release-set
ID. A failed post-restart health check atomically restores the prior target.
Systemd owns the whole process cgroup, so uninstall stops descendants before
removing the unit and immutable executables.

Uninstall retains user data and state. `uninstall --purge-data` is the only
operation that deletes them; use it only for an explicit factory reset.
