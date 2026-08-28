//! Typed Windows-to-WSL launcher boundary.
//!
//! Commands are passed directly to `wsl.exe`; no shell parses user input.
//! Secret bytes travel only through the child's stdin and are erased from the
//! caller-provided buffer after the write attempt.

#![deny(missing_docs)]

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};

/// The single WSL distribution supported by the Nimino v1 contract.
pub const SUPPORTED_DISTRIBUTION: &str = "Ubuntu-24.04";
const HANDSHAKE_PREFIX: &str = "NIMINO_WSL_PID_V1";
const SECRET_PREFIX: &[u8] = b"NIMINO_SECRET_V1\0";

/// Validated Linux process invocation for the supported WSL distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    user: String,
    program: String,
    working_directory: String,
    args: Vec<String>,
}

impl LaunchSpec {
    /// Validates a Linux user, executable, working directory, and direct argv.
    pub fn new(
        user: impl Into<String>,
        program: impl Into<String>,
        working_directory: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<Self, String> {
        let user = user.into();
        let program = program.into();
        let working_directory = working_directory.into();
        let args = args.into_iter().map(Into::into).collect::<Vec<_>>();
        if !valid_user(&user) {
            return Err("WSL user must be a lowercase Linux account name".into());
        }
        if !valid_linux_path(&program, &["/home/", "/usr/"]) {
            return Err("WSL program must be an absolute /home or /usr path".into());
        }
        if !valid_linux_path(&working_directory, &["/home/"]) {
            return Err("WSL working directory must be an absolute /home path".into());
        }
        if args.iter().any(|arg| arg.contains('\0')) {
            return Err("WSL argument contains NUL".into());
        }
        Ok(Self {
            user,
            program,
            working_directory,
            args,
        })
    }

    fn command(&self, launcher: &Path) -> Command {
        let mut command = Command::new(launcher);
        command
            .arg("--distribution")
            .arg(SUPPORTED_DISTRIBUTION)
            .arg("--user")
            .arg(&self.user)
            .arg("--cd")
            .arg(&self.working_directory)
            .arg("--exec")
            .arg(&self.program)
            .args(&self.args)
            .env_remove("NIMINO_PRIVATE_KEY")
            .env_remove("NIMINO_API_TOKEN")
            .env_remove("NIMINO_AUTH_TAG")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        command
    }
}

fn valid_user(user: &str) -> bool {
    let mut chars = user.chars();
    matches!(chars.next(), Some('a'..='z' | '_'))
        && user.len() <= 32
        && chars.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-')
        })
}

fn valid_linux_path(path: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| path.starts_with(prefix))
        && !path.contains('\0')
        && !path.contains('\\')
        && path
            .split('/')
            .skip(1)
            .all(|component| !matches!(component, "" | "." | ".."))
}

/// Owned Windows/WSL process pair with verified distribution, user, and PIDs.
pub struct WslProcess {
    distribution: &'static str,
    user: String,
    host_pid: u32,
    linux_pid: u32,
    child: Child,
    _stdout: ChildStdout,
    exited: bool,
}

impl WslProcess {
    /// Returns the verified WSL distribution.
    pub fn distribution(&self) -> &str {
        self.distribution
    }

    /// Returns the verified Linux account.
    pub fn user(&self) -> &str {
        &self.user
    }

    /// Returns the Windows launcher process ID.
    pub fn host_pid(&self) -> u32 {
        self.host_pid
    }

    /// Returns the Linux process ID reported by the verified handshake.
    pub fn linux_pid(&self) -> u32 {
        self.linux_pid
    }

    /// Waits for the owned host process to exit.
    pub fn wait(&mut self) -> std::io::Result<ExitStatus> {
        let status = self.child.wait()?;
        self.exited = true;
        Ok(status)
    }
}

impl Drop for WslProcess {
    fn drop(&mut self) {
        if !self.exited {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Launches a validated WSL process and transfers `secret` through framed stdin.
///
/// The caller's secret buffer is erased on every launch outcome. The returned
/// owner is created only after the child reports the requested distro and user.
pub fn launch_with(
    launcher: &Path,
    spec: &LaunchSpec,
    secret: &mut [u8],
) -> Result<WslProcess, String> {
    if secret.len() > u32::MAX as usize {
        secret.fill(0);
        return Err("secret exceeds stdin frame limit".into());
    }
    let mut child = match spec.command(launcher).spawn() {
        Ok(child) => child,
        Err(error) => {
            secret.fill(0);
            return Err(format!("launch wsl.exe: {error}"));
        }
    };
    let host_pid = child.id();
    let write_result = (|| {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "wsl.exe stdin was not piped".to_string())?;
        stdin
            .write_all(SECRET_PREFIX)
            .and_then(|()| stdin.write_all(&(secret.len() as u32).to_be_bytes()))
            .and_then(|()| stdin.write_all(secret))
            .map_err(|error| format!("write WSL secret frame: {error}"))
    })();
    secret.fill(0);
    if let Err(error) = write_result {
        stop(&mut child);
        return Err(error);
    }

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "wsl.exe stdout was not piped".to_string())?;
    let line = match read_bounded_line(&mut stdout, 512) {
        Ok(line) => line,
        Err(error) => {
            stop(&mut child);
            return Err(error);
        }
    };
    let (distribution, user, linux_pid) = match parse_handshake(&line) {
        Ok(value) => value,
        Err(error) => {
            stop(&mut child);
            return Err(error);
        }
    };
    if distribution != SUPPORTED_DISTRIBUTION || user != spec.user {
        stop(&mut child);
        return Err("WSL process identity did not match requested distro/user".into());
    }
    Ok(WslProcess {
        distribution: SUPPORTED_DISTRIBUTION,
        user,
        host_pid,
        linux_pid,
        child,
        _stdout: stdout,
        exited: false,
    })
}

fn read_bounded_line(reader: &mut impl Read, limit: usize) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(64);
    for _ in 0..limit {
        let mut byte = [0];
        match reader.read(&mut byte) {
            Ok(0) => return Err("WSL process ended before PID handshake".into()),
            Ok(_) if byte[0] == b'\n' => {
                return String::from_utf8(bytes)
                    .map_err(|_| "WSL PID handshake was not UTF-8".into())
            }
            Ok(_) => bytes.push(byte[0]),
            Err(error) => return Err(format!("read WSL PID handshake: {error}")),
        }
    }
    Err("WSL PID handshake exceeded 512 bytes".into())
}

fn parse_handshake(line: &str) -> Result<(String, String, u32), String> {
    let fields = line.split('\t').collect::<Vec<_>>();
    if fields.len() != 4 || fields[0] != HANDSHAKE_PREFIX || !valid_user(fields[2]) {
        return Err("invalid WSL PID handshake".into());
    }
    let linux_pid = fields[3]
        .parse::<u32>()
        .map_err(|_| "invalid Linux PID in WSL handshake".to_string())?;
    if linux_pid == 0 {
        return Err("Linux PID in WSL handshake must be nonzero".into());
    }
    Ok((fields[1].to_string(), fields[2].to_string(), linux_pid))
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    fn strings<'a>(values: impl Iterator<Item = &'a OsStr>) -> Vec<String> {
        values
            .map(|value| value.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn command_is_typed_and_preserves_unicode_arguments() {
        let spec = LaunchSpec::new(
            "nimino",
            "/home/nimino/.local/bin/nimino-desktop",
            "/home/nimino/開発",
            ["--profile", "日本語"],
        )
        .unwrap();
        let command = spec.command(Path::new("wsl.exe"));
        assert_eq!(
            strings(command.get_args()),
            [
                "--distribution",
                SUPPORTED_DISTRIBUTION,
                "--user",
                "nimino",
                "--cd",
                "/home/nimino/開発",
                "--exec",
                "/home/nimino/.local/bin/nimino-desktop",
                "--profile",
                "日本語",
            ]
        );
        assert!(command
            .get_envs()
            .any(|(name, value)| name == "NIMINO_PRIVATE_KEY" && value.is_none()));
    }
}
