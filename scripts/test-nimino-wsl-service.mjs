#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  chmodSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = process.cwd();
const lifecycle = join(root, "scripts/nimino-wsl-service.sh");
const contract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-wsl-service/v1/contract.json"),
    "utf8",
  ),
);
const work = mkdtempSync(join(tmpdir(), "nimino-wsl-service-"));
const home = join(work, "home", "nimino");
const dataHome = join(home, ".local", "share");
const stateHome = join(home, ".local", "state");
const configHome = join(home, ".config");
const systemctl = join(work, "systemctl");
const log = join(work, "systemctl.log");
const pidFile = join(work, "service.pid");
const failNext = join(work, "fail-next-active");
const first = "1".repeat(64);
const second = "2".repeat(64);

assert.deepEqual(Object.keys(contract).sort(), [
  "activation",
  "commands",
  "contract",
  "healthGate",
  "issue",
  "platformContract",
  "processOwnership",
  "purgeRequiresExplicitFlag",
  "releaseIdentity",
  "releaseLayout",
  "restartPolicy",
  "rollback",
  "schemaVersion",
  "serviceManager",
  "uninstallRetention",
  "unit",
]);
assert.equal(contract.contract, "nimino.wsl-service");
assert.equal(contract.issue, 19);
assert.equal(contract.serviceManager, "systemd-user");
assert.equal(contract.processOwnership, "control-group");
assert.equal(contract.purgeRequiresExplicitFlag, true);
assert.deepEqual(contract.commands, [
  "install",
  "update",
  "restart",
  "uninstall",
]);

mkdirSync(home, { recursive: true });
writeFileSync(
  systemctl,
  `#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$*" >> "$NIMINO_FAKE_SYSTEMCTL_LOG"
[[ "$1" == "--user" ]] && shift
stop_process() {
  if [[ -f "$NIMINO_FAKE_PID_FILE" ]]; then
    pid="$(<"$NIMINO_FAKE_PID_FILE")"
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    rm -f "$NIMINO_FAKE_PID_FILE"
  fi
}
case "$1" in
  daemon-reload) ;;
  enable|restart)
    stop_process
    sleep 300 >/dev/null 2>&1 &
    printf '%s\\n' "$!" > "$NIMINO_FAKE_PID_FILE"
    ;;
  is-active)
    if [[ -f "$NIMINO_FAKE_FAIL_NEXT" ]]; then
      rm -f "$NIMINO_FAKE_FAIL_NEXT"
      exit 1
    fi
    [[ -f "$NIMINO_FAKE_PID_FILE" ]] && kill -0 "$(<"$NIMINO_FAKE_PID_FILE")"
    ;;
  disable) stop_process ;;
  *) exit 2 ;;
esac
`,
);
chmodSync(systemctl, 0o755);

function binary(name) {
  const path = join(work, name);
  writeFileSync(path, `#!/usr/bin/env bash\necho ${name}\n`);
  chmodSync(path, 0o755);
  return path;
}

const environment = {
  ...process.env,
  HOME: home,
  XDG_DATA_HOME: dataHome,
  XDG_STATE_HOME: stateHome,
  XDG_CONFIG_HOME: configHome,
  NIMINO_SYSTEMCTL: systemctl,
  NIMINO_FAKE_SYSTEMCTL_LOG: log,
  NIMINO_FAKE_PID_FILE: pidFile,
  NIMINO_FAKE_FAIL_NEXT: failNext,
};

function run(...args) {
  return spawnSync("bash", [lifecycle, ...args], {
    cwd: root,
    env: environment,
    encoding: "utf8",
  });
}

const relay1 = binary("relay-v1");
const relay2 = binary("relay-v2");
assert.equal(
  run("install", "--release-set-id", first, "--relay", relay1).status,
  0,
);
const app = join(dataHome, "nimino");
const releases = join(app, "service-releases");
const current = join(app, "current");
const unit = join(configHome, "systemd", "user", "nimino-relay.service");
assert.deepEqual(readdirSync(releases), [first]);
assert.equal(readlinkSync(current), join(releases, first));
assert.match(readFileSync(unit, "utf8"), /KillMode=control-group/);
assert.match(readFileSync(unit, "utf8"), /Restart=on-failure/);

writeFileSync(join(app, "data", "retained.db"), "user-data");
assert.equal(
  run("install", "--release-set-id", first, "--relay", relay1).status,
  0,
);
assert.deepEqual(readdirSync(releases), [first]);

writeFileSync(failNext, "1");
const failed = run("update", "--release-set-id", second, "--relay", relay2);
assert.notEqual(failed.status, 0);
assert.match(failed.stderr, /rolled back/i);
assert.equal(readlinkSync(current), join(releases, first));
assert(!existsSync(join(releases, second)));

assert.equal(
  run("update", "--release-set-id", second, "--relay", relay2).status,
  0,
);
assert.equal(readlinkSync(current), join(releases, second));
assert.equal(run("restart").status, 0);
const servicePid = Number(readFileSync(pidFile, "utf8").trim());
assert.doesNotThrow(() => process.kill(servicePid, 0));

assert.equal(run("uninstall").status, 0);
assert.throws(() => process.kill(servicePid, 0));
assert(!existsSync(unit));
assert(!existsSync(current));
assert(!existsSync(releases));
assert.equal(
  readFileSync(join(app, "data", "retained.db"), "utf8"),
  "user-data",
);

assert.equal(
  run("install", "--release-set-id", first, "--relay", relay1).status,
  0,
);
assert.equal(run("uninstall", "--purge-data").status, 0);
assert(!existsSync(app));
assert.match(
  readFileSync(log, "utf8"),
  /--user disable --now nimino-relay\.service/,
);

console.log(
  "Nimino WSL service tests passed: rerun, rollback, restart, uninstall/reinstall",
);
