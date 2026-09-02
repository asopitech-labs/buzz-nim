#[cfg(unix)]
use std::fs;
use std::path::Path;

#[cfg(unix)]
use nimino_wsl_launcher::SUPPORTED_DISTRIBUTION;
use nimino_wsl_launcher::{launch_with, LaunchSpec};

#[test]
fn user_and_linux_paths_fail_closed() {
    for (user, program, cwd) in [
        ("Root User", "/usr/bin/nimino", "/home/nimino"),
        ("nimino", "/mnt/c/nimino", "/home/nimino"),
        ("nimino", "/usr/bin/nimino", "/mnt/c/work"),
        ("nimino", "/usr/bin/nimino", "/home/nimino/../other"),
    ] {
        assert!(LaunchSpec::new(user, program, cwd, std::iter::empty::<&str>()).is_err());
    }
}

#[test]
fn spawn_failure_erases_secret() {
    let spec = LaunchSpec::new(
        "nimino",
        "/usr/bin/nimino",
        "/home/nimino",
        std::iter::empty::<&str>(),
    )
    .unwrap();
    let mut secret = b"erase-even-before-stdin".to_vec();
    assert!(launch_with(Path::new("/missing/wsl.exe"), &spec, &mut secret).is_err());
    assert!(secret.iter().all(|byte| *byte == 0));
}

#[cfg(unix)]
#[test]
fn launch_tracks_both_pids_and_uses_only_stdin_for_secret() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let work = std::env::temp_dir().join(format!("nimino-wsl-launcher-{nonce}"));
    fs::create_dir(&work).unwrap();
    let launcher = work.join("wsl.exe");
    let args_log = work.join("args");
    let stdin_log = work.join("stdin");
    fs::write(
        &launcher,
        format!(
            "#!/usr/bin/env bash\nset -euo pipefail\nprintf '%s\\n' \"$@\" > '{}'\ncat > '{}'\nprintf 'NIMINO_WSL_PID_V1\\tUbuntu-24.04\\tnimino\\t4242\\n'\n",
            args_log.display(),
            stdin_log.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();

    let spec = LaunchSpec::new("nimino", "/usr/bin/nimino", "/home/nimino", ["--version"]).unwrap();
    let mut secret = b"never-in-argv-or-env".to_vec();
    let mut process = launch_with(&launcher, &spec, &mut secret).unwrap();
    assert_eq!(process.distribution(), SUPPORTED_DISTRIBUTION);
    assert_eq!(process.user(), "nimino");
    assert!(process.host_pid() > 0);
    assert_eq!(process.linux_pid(), 4242);
    assert!(process.wait().unwrap().success());
    assert!(secret.iter().all(|byte| *byte == 0));

    let args = fs::read(&args_log).unwrap();
    assert!(!args
        .windows(b"never-in-argv-or-env".len())
        .any(|value| value == b"never-in-argv-or-env"));
    let frame = fs::read(&stdin_log).unwrap();
    assert!(frame.starts_with(b"NIMINO_SECRET_V1\0"));
    assert!(frame.ends_with(b"never-in-argv-or-env"));
    fs::remove_dir_all(work).unwrap();
}

#[cfg(unix)]
#[test]
fn handshake_from_another_distribution_is_rejected() {
    use std::os::unix::fs::PermissionsExt;

    let work = std::env::temp_dir().join(format!("nimino-wsl-mismatch-{}", std::process::id()));
    fs::create_dir_all(&work).unwrap();
    let launcher = work.join("wsl.exe");
    fs::write(
        &launcher,
        "#!/bin/sh\ncat >/dev/null\nprintf 'NIMINO_WSL_PID_V1\\tDebian\\tnimino\\t4242\\n'\n",
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();

    let spec = LaunchSpec::new(
        "nimino",
        "/usr/bin/nimino",
        "/home/nimino",
        std::iter::empty::<&str>(),
    )
    .unwrap();
    let mut secret = b"not-for-another-distro".to_vec();
    assert!(launch_with(&launcher, &spec, &mut secret).is_err());
    assert!(secret.iter().all(|byte| *byte == 0));
    fs::remove_dir_all(work).unwrap();
}
