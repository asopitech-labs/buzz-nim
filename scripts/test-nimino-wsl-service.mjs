#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  appendFileSync,
  chmodSync,
  existsSync,
  mkdtempSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const root = process.cwd();
const lifecycle = join(root, "scripts/nimino-wsl-service.sh");
const releaseCli = join(root, "scripts/nimino-release-set.mjs");
const bundleCli = join(root, "scripts/nimino-wsl-bundle.mjs");
const serviceContract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-wsl-service/v1/contract.json"),
    "utf8",
  ),
);
const bundleContract = JSON.parse(
  readFileSync(
    join(root, "contracts/nimino-wsl-bundle/v1/contract.json"),
    "utf8",
  ),
);
const work = mkdtempSync(join(tmpdir(), "nimino-wsl-service-"));
const home = join(work, "home", "nimino");
const dataHome = join(home, ".local", "share");
const stateHome = join(home, ".local", "state");
const configHome = join(home, ".config");
const binHome = join(home, ".local", "bin");
const systemctl = join(work, "systemctl");
const log = join(work, "systemctl.log");
const pidFile = join(work, "service.pid");
const failNext = join(work, "fail-next-active");
const app = join(dataHome, "nimino");
const releases = join(app, "service-releases");
const current = join(app, "current");
const unit = join(configHome, "systemd", "user", "nimino-relay.service");

function cleanup() {
  if (existsSync(pidFile)) {
    const pid = Number(readFileSync(pidFile, "utf8").trim());
    try {
      process.kill(pid, "SIGTERM");
    } catch {
      // Already stopped by uninstall.
    }
  }
  rmSync(work, { recursive: true, force: true });
}
process.on("exit", cleanup);

assert.deepEqual(Object.keys(serviceContract).sort(), [
  "activation",
  "bundleContract",
  "bundleInput",
  "commands",
  "contract",
  "healthGate",
  "installedTools",
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
assert.equal(serviceContract.contract, "nimino.wsl-service");
assert.equal(serviceContract.issue, 19);
assert.equal(serviceContract.bundleContract, bundleContract.contract);
assert.equal(serviceContract.bundleInput, "verified-complete-bundle-only");
assert.equal(serviceContract.serviceManager, "systemd-user");
assert.equal(serviceContract.processOwnership, "control-group");
assert.equal(serviceContract.purgeRequiresExplicitFlag, true);
assert.deepEqual(serviceContract.commands, [
  "install",
  "update",
  "restart",
  "uninstall",
]);
assert.deepEqual(
  serviceContract.installedTools,
  bundleContract.components.map(({ installName }) => installName),
);

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
    "$NIMINO_FAKE_CURRENT/bin/nimino-relay" --health >> "$NIMINO_FAKE_SYSTEMCTL_LOG"
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

const environment = {
  ...process.env,
  HOME: home,
  XDG_DATA_HOME: dataHome,
  XDG_STATE_HOME: stateHome,
  XDG_CONFIG_HOME: configHome,
  XDG_BIN_HOME: binHome,
  NIMINO_SYSTEMCTL: systemctl,
  NIMINO_FAKE_SYSTEMCTL_LOG: log,
  NIMINO_FAKE_PID_FILE: pidFile,
  NIMINO_FAKE_FAIL_NEXT: failNext,
  NIMINO_FAKE_CURRENT: current,
};

function run(command, args, env = process.env) {
  return spawnSync(command, args, { cwd: root, env, encoding: "utf8" });
}

function runService(...args) {
  return run("bash", [lifecycle, ...args], environment);
}

function makeBundle(version, commit, label) {
  const artifacts = join(work, `artifacts-${label}`);
  const releaseSet = join(work, `release-set-${label}.json`);
  const bundle = join(work, `bundle-${label}`);
  mkdirSync(artifacts);
  const specifications = bundleContract.components.map(
    ({ artifactId, installName }) => {
      const path = join(artifacts, `${artifactId}-${label}`);
      writeFileSync(
        path,
        `#!/usr/bin/env bash\nprintf '%s\\n' '${label}:${installName}:'"$*"\n`,
      );
      chmodSync(path, 0o755);
      return `${artifactId}:0.1.0:${path}`;
    },
  );
  const created = run(process.execPath, [
    releaseCli,
    "create",
    "--version",
    version,
    "--tag",
    `nimino-v${version}`,
    "--commit",
    commit,
    "--output",
    releaseSet,
    ...specifications.flatMap((specification) => ["--artifact", specification]),
  ]);
  assert.equal(created.status, 0, created.stderr);
  const composed = run(process.execPath, [
    bundleCli,
    "compose",
    "--release-set",
    releaseSet,
    "--resolved-tag-commit",
    commit,
    "--artifact-dir",
    artifacts,
    "--output",
    bundle,
  ]);
  assert.equal(composed.status, 0, composed.stderr);
  return { bundle, id: composed.stdout.trim() };
}

function installArgs(command, release) {
  return [command, "--release-set-id", release.id, "--bundle", release.bundle];
}

function assertWorkflow(label) {
  for (const { installName } of bundleContract.components) {
    const argument = installName === "nimino-data-ops" ? "verify" : "--version";
    const result = run(join(binHome, installName), [argument]);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, new RegExp(`^${label}:${installName}:`));
    assert.equal(
      readlinkSync(join(binHome, installName)),
      join(current, "bin", installName),
    );
  }
}

const first = makeBundle("1.0.0", "a".repeat(40), "v1");
const second = makeBundle("1.0.1", "b".repeat(40), "v2");
const corrupt = makeBundle("1.0.2", "c".repeat(40), "corrupt");
appendFileSync(join(corrupt.bundle, "bin", "nimino-data-ops"), "tampered\n");

assert.equal(runService(...installArgs("install", first)).status, 0);
assert.equal(runService(...installArgs("install", first)).status, 0);
assert.deepEqual(readdirSync(releases), [first.id]);
assert.equal(readlinkSync(current), join(releases, first.id));
assert.match(readFileSync(unit, "utf8"), /KillMode=control-group/);
assert.match(readFileSync(unit, "utf8"), /Restart=on-failure/);
assertWorkflow("v1");
const installedManifest = JSON.parse(
  readFileSync(join(current, "nimino-wsl-bundle.json"), "utf8"),
);
assert.equal(installedManifest.releaseSetId, first.id);
assert.equal(
  JSON.parse(readFileSync(join(current, "release-set.json"), "utf8"))
    .releaseSetId,
  first.id,
);

writeFileSync(join(app, "data", "retained.db"), "user-data");
const checksumFailure = runService(...installArgs("update", corrupt));
assert.notEqual(checksumFailure.status, 0);
assert.match(checksumFailure.stderr, /checksum verification failed/i);
assert(!existsSync(join(releases, corrupt.id)));

writeFileSync(failNext, "1");
const failed = runService(...installArgs("update", second));
assert.notEqual(failed.status, 0);
assert.match(failed.stderr, /rolled back/i);
assert.equal(readlinkSync(current), join(releases, first.id));
assert(!existsSync(join(releases, second.id)));
assert(!readdirSync(releases).some((name) => name.startsWith(".staging.")));
assertWorkflow("v1");

assert.equal(runService(...installArgs("update", second)).status, 0);
assert.equal(readlinkSync(current), join(releases, second.id));
assertWorkflow("v2");
assert.equal(runService("restart").status, 0);
const servicePid = Number(readFileSync(pidFile, "utf8").trim());
assert.doesNotThrow(() => process.kill(servicePid, 0));

assert.equal(runService("uninstall").status, 0);
assert.throws(() => process.kill(servicePid, 0));
assert(!existsSync(unit));
assert(!existsSync(current));
assert(!existsSync(releases));
assert.deepEqual(existsSync(binHome) ? readdirSync(binHome) : [], []);
assert.equal(
  readFileSync(join(app, "data", "retained.db"), "utf8"),
  "user-data",
);

assert.equal(runService(...installArgs("install", first)).status, 0);
assert.equal(runService("uninstall", "--purge-data").status, 0);
assert(!existsSync(app));
assert.match(
  readFileSync(log, "utf8"),
  /--user disable --now nimino-relay\.service/,
);

console.log(
  "Nimino WSL bundle lifecycle passed: manifest install, workflow, rollback, cleanup, uninstall",
);
