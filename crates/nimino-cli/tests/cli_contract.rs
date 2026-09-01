use std::path::Path;
use std::process::{Command, Output};

fn nimino(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nimino"))
        .env_remove("NIMINO_PRIVATE_KEY")
        .env_remove("NIMINO_AUTH_TAG")
        .args(args)
        .output()
        .expect("run nimino")
}

fn stderr_json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stderr).unwrap_or_else(|error| {
        panic!(
            "stderr is not JSON: {error}; stderr={}",
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn help_and_usage_follow_the_v1_stream_contract() {
    let help = nimino(&["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Nimino CLI"));
    assert!(help.stderr.is_empty());

    let usage = nimino(&["not-a-command"]);
    assert_eq!(usage.status.code(), Some(1));
    assert!(usage.stdout.is_empty());
    let error = stderr_json(&usage);
    assert_eq!(error["error"], "user_error");
    assert_eq!(error["retryable"], false);
}

#[test]
fn missing_identity_uses_the_auth_exit_contract() {
    let output = nimino(&["channels", "list"]);
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let error = stderr_json(&output);
    assert_eq!(error["error"], "auth_error");
    assert_eq!(error["retryable"], false);
}

#[test]
fn malformed_relay_uses_the_network_exit_contract() {
    let output = nimino(&[
        "--relay",
        "http://[",
        "--private-key",
        "0000000000000000000000000000000000000000000000000000000000000001",
        "channels",
        "list",
    ]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let error = stderr_json(&output);
    assert_eq!(error["error"], "network_error");
    assert_eq!(error["retryable"], false);
}

#[test]
fn old_buzz_binary_is_not_built() {
    let nimino = Path::new(env!("CARGO_BIN_EXE_nimino"));
    let legacy = nimino.with_file_name(if cfg!(windows) { "buzz.exe" } else { "buzz" });
    assert!(
        !legacy.exists(),
        "legacy CLI artifact exists: {}",
        legacy.display()
    );
    assert!(!include_str!("../Cargo.toml").contains("name = \"buzz\""));
}
