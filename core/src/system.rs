//! System integration: shell PATH, executables, processes, keychain,
//! plugin status.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::Result;

use crate::paths::home;

pub const KEYCHAIN_SERVICE: &str = "Flow Keeper";
pub const KEYCHAIN_ACCOUNT: &str = "anthropic-api-key";

/// Claude Code versions the adapter has been verified on.
pub const TESTED_CLAUDE_VERSIONS: &[&str] = &["2.1.292"];
/// Codex CLI versions the adapter has been verified on with real sessions.
/// Empty until the test with real sessions is done (`docs/compatibility.md`).
pub const TESTED_CODEX_VERSIONS: &[&str] = &[];

/// PATH of the user's login shell. Apps launched from the Finder have a
/// minimal PATH and would not find `claude` or `git`.
pub fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let out = run_with_timeout(
        Command::new(shell).args(["-l", "-c", "printf '%s' \"$PATH\""]),
        Duration::from_secs(5),
    )?;
    let path = String::from_utf8_lossy(&out).trim().to_string();
    (!path.is_empty()).then_some(path)
}

pub fn search_path(path_env: Option<&str>) -> String {
    let mut dirs: Vec<String> = path_env.map(|p| p.split(':').map(str::to_string).collect()).unwrap_or_default();
    let h = home();
    for extra in [
        h.join(".local/bin"),
        h.join(".claude/local"),
        h.join(".npm-global/bin"),
        h.join(".bun/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
    ] {
        let s = extra.to_string_lossy().to_string();
        if !dirs.contains(&s) {
            dirs.push(s);
        }
    }
    dirs.join(":")
}

pub fn which(name: &str, path_env: &str) -> Option<PathBuf> {
    path_env
        .split(':')
        .map(|d| Path::new(d).join(name))
        .find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

pub fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Option<Vec<u8>> {
    use std::process::Stdio;
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let out = child.wait_with_output().ok()?;
                return status.success().then_some(out.stdout);
            }
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => return None,
        }
    }
}

/// Claude Code version, e.g. "2.1.292".
pub fn claude_version(claude: &Path, path_env: &str) -> Option<String> {
    let out = run_with_timeout(Command::new(claude).arg("--version").env("PATH", path_env), Duration::from_secs(10))?;
    let text = String::from_utf8_lossy(&out);
    text.split_whitespace().next().map(str::to_string)
}

/// Codex CLI version, e.g. "0.163.0" (from “codex-cli 0.163.0”).
pub fn codex_version(codex: &Path, path_env: &str) -> Option<String> {
    let out = run_with_timeout(
        Command::new(codex).arg("--version").env("PATH", path_env).env("FLOW_KEEPER_INTERNAL", "1"),
        Duration::from_secs(10),
    )?;
    let text = String::from_utf8_lossy(&out);
    text.split_whitespace().last().map(str::to_string)
}

/// Checks whether the processes are alive and still `claude` or `codex` (guards against PID reuse).
/// Returns `None` when the check is not possible.
pub fn processes_alive(pids: &[i64]) -> Option<Vec<(i64, bool)>> {
    if pids.is_empty() {
        return Some(Vec::new());
    }
    let list = pids.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(",");
    let out = Command::new("/bin/ps").args(["-o", "pid=,comm=", "-p", &list]).output().ok()?;
    // ps exits with 1 if no PID exists: it is still a valid answer.
    let text = String::from_utf8_lossy(&out.stdout);
    let running: Vec<(i64, String)> = text
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let (pid, comm) = l.split_once(char::is_whitespace)?;
            Some((pid.parse().ok()?, comm.trim().to_string()))
        })
        .collect();
    Some(
        pids.iter()
            .map(|pid| {
                let alive = running.iter().any(|(p, comm)| {
                    let name = Path::new(comm).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                    p == pid && (name.contains("claude") || name == "codex" || name == "node")
                });
                (*pid, alive)
            })
            .collect(),
    )
}

// ---- Keychain -----------------------------------------------------------------

#[cfg(target_os = "macos")]
pub fn api_key() -> Option<String> {
    let bytes = security_framework::passwords::get_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).ok()?;
    String::from_utf8(bytes).ok().filter(|k| !k.trim().is_empty())
}

#[cfg(target_os = "macos")]
pub fn set_api_key(key: Option<&str>) -> Result<()> {
    use security_framework::passwords::{delete_generic_password, set_generic_password};
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => set_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT, k.as_bytes())?,
        None => {
            let _ = delete_generic_password(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT);
        }
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn api_key() -> Option<String> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn set_api_key(_key: Option<&str>) -> Result<()> {
    Err(anyhow::anyhow!("keychain not supported on this system"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_is_not_claude_and_dead_pid_is_dead() {
        let me = std::process::id() as i64;
        let res = processes_alive(&[me, 999_999]).unwrap();
        assert_eq!(res, vec![(me, false), (999_999, false)]);
    }

    #[test]
    fn search_path_adds_common_locations() {
        let p = search_path(Some("/x/bin"));
        assert!(p.starts_with("/x/bin:"));
        assert!(p.contains("/opt/homebrew/bin"));
        assert!(which("sh", &p).is_some());
    }
}
