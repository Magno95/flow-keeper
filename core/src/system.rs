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

/// Output kept per stream; a process that writes more is still drained.
pub const MAX_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
/// How long to wait, in total, for the I/O threads after the process has
/// exited or been killed (a descendant may have inherited the pipes).
const IO_GRACE: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct Captured {
    pub status: std::process::ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// A stream exceeded `MAX_CAPTURE_BYTES`: the rest was read and discarded.
    pub truncated: bool,
    /// A pipe was still open after the grace period (a descendant kept it): the
    /// descendants were killed and the output may be incomplete.
    pub incomplete: bool,
}

#[derive(Debug)]
pub enum ExecError {
    Start(std::io::Error),
    Timeout(Duration),
    Io(std::io::Error),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Start(e) => write!(f, "could not start the process: {e}"),
            ExecError::Timeout(d) => write!(f, "timed out after {}s", d.as_secs()),
            ExecError::Io(e) => write!(f, "process error: {e}"),
        }
    }
}

impl std::error::Error for ExecError {}

/// An I/O thread with a way to wait for it for a bounded time.
struct Task<T> {
    handle: std::thread::JoinHandle<T>,
    done: std::sync::mpsc::Receiver<()>,
}

impl<T: Send + 'static> Task<T> {
    fn spawn(work: impl FnOnce() -> T + Send + 'static) -> Self {
        let (tx, done) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            let value = work();
            let _ = tx.send(());
            value
        });
        Task { handle, done }
    }

    /// The thread's result if it finishes before `deadline`; otherwise it is left
    /// to end on its own (its pipe is closed once the process tree is gone).
    fn join_until(self, deadline: std::time::Instant) -> Option<T> {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match self.done.recv_timeout(left) {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => self.handle.join().ok(),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
        }
    }
}

type Shared = std::sync::Arc<std::sync::Mutex<(Vec<u8>, bool)>>;

/// Reads a pipe until EOF, keeping the first `MAX_CAPTURE_BYTES` and
/// discarding the rest: stopping the reading would leave the child blocked on
/// a full pipe.
fn drain(mut pipe: impl std::io::Read + Send + 'static) -> (Shared, Task<()>) {
    let shared: Shared = Default::default();
    let writer = shared.clone();
    let task = Task::spawn(move || {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut guard = writer.lock().expect("capture");
                    let room = MAX_CAPTURE_BYTES.saturating_sub(guard.0.len());
                    guard.0.extend_from_slice(&chunk[..n.min(room)]);
                    guard.1 |= n > room;
                }
            }
        }
    });
    (shared, task)
}

/// Kills the process and, on Unix, every process in its group (the command is
/// started as the leader of its own group).
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    // SAFETY: plain syscall on the pid of a child we spawned as a group leader.
    unsafe {
        libc::killpg(child.id() as libc::pid_t, libc::SIGKILL);
    }
    let _ = child.kill();
}

/// Runs a command with stdin (if given), stdout and stderr handled
/// concurrently and a deadline that covers the whole run, writing included.
/// The command leads its own process group: on timeout, or when a descendant
/// keeps the pipes open after it exited, the whole group is killed and the
/// process reaped. Output beyond `MAX_CAPTURE_BYTES` per stream is drained,
/// not kept.
pub fn run_command(cmd: &mut Command, stdin: Option<&[u8]>, timeout: Duration) -> Result<Captured, ExecError> {
    use std::process::Stdio;
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(cmd, 0);
    let mut child = cmd
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(ExecError::Start)?;
    let (out, out_task) = drain(child.stdout.take().expect("piped stdout"));
    let (err, err_task) = drain(child.stderr.take().expect("piped stderr"));
    // A child that never reads stdin must not hold the caller: the write fails
    // or ends when the child is killed or exits.
    let writer = match (child.stdin.take(), stdin) {
        (Some(mut pipe), Some(input)) => {
            let input = input.to_vec();
            Some(Task::spawn(move || {
                use std::io::Write;
                pipe.write_all(&input).is_ok()
            }))
        }
        _ => None,
    };
    let start = std::time::Instant::now();
    let outcome = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if start.elapsed() >= timeout => break Err(ExecError::Timeout(timeout)),
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => break Err(ExecError::Io(e)),
        }
    };
    let deadline = std::time::Instant::now() + IO_GRACE;
    let mut incomplete = false;
    if outcome.is_err() {
        kill_tree(&mut child);
        let _ = child.wait();
    }
    let done_out = out_task.join_until(deadline).is_some();
    let done_err = err_task.join_until(deadline).is_some();
    if !(done_out && done_err) {
        // A descendant of the finished command still holds a pipe.
        // Killing the group closes the pipes: the reader threads end by themselves.
        incomplete = true;
        kill_tree(&mut child);
    }
    if let Some(writer) = writer {
        let _ = writer.join_until(deadline.max(std::time::Instant::now() + Duration::from_millis(200)));
    }
    let status = outcome?;
    let (stdout, cut_out) = {
        let guard = out.lock().expect("capture");
        (guard.0.clone(), guard.1)
    };
    let (stderr, cut_err) = {
        let guard = err.lock().expect("capture");
        (guard.0.clone(), guard.1)
    };
    Ok(Captured { status, stdout, stderr, truncated: cut_out || cut_err, incomplete })
}

/// Stdout of a command that exits successfully within the deadline.
pub fn run_with_timeout(cmd: &mut Command, timeout: Duration) -> Option<Vec<u8>> {
    let out = run_command(cmd, None, timeout).ok()?;
    out.status.success().then_some(out.stdout)
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

    fn sh(script: &str) -> Command {
        let mut cmd = Command::new("/bin/sh");
        cmd.args(["-c", script]);
        cmd
    }

    #[test]
    fn output_larger_than_the_pipe_buffer_does_not_look_like_a_timeout() {
        // 1 MiB on stdout and on stderr: more than a pipe holds.
        let out = run_command(
            &mut sh("head -c 1048576 /dev/zero; head -c 1048576 /dev/zero >&2"),
            None,
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(out.status.success());
        assert_eq!((out.stdout.len(), out.stderr.len(), out.truncated), (1 << 20, 1 << 20, false));
        assert_eq!(run_with_timeout(&mut sh("head -c 1048576 /dev/zero"), Duration::from_secs(10)).map(|o| o.len()), Some(1 << 20));
    }

    #[test]
    fn output_over_the_limit_is_drained_not_kept() {
        let n = MAX_CAPTURE_BYTES + 1_000_000;
        let out = run_command(&mut sh(&format!("head -c {n} /dev/zero")), None, Duration::from_secs(20)).unwrap();
        assert!(out.status.success());
        assert_eq!((out.stdout.len(), out.truncated), (MAX_CAPTURE_BYTES, true));
    }

    fn alive(pid: &str) -> bool {
        Command::new("/bin/kill").args(["-0", pid]).status().map(|s| s.success()).unwrap_or(false)
    }

    fn wait_gone(pid: &str) -> bool {
        for _ in 0..50 {
            if !alive(pid) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    #[test]
    fn child_that_ignores_a_large_stdin_ends_by_the_deadline() {
        let input = vec![b'x'; 4 * 1024 * 1024];
        let started = std::time::Instant::now();
        let res = run_command(&mut sh("sleep 30"), Some(&input), Duration::from_millis(300));
        assert!(matches!(res, Err(ExecError::Timeout(_))), "{res:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        // A child that exits without reading its input is not an error of ours.
        let out = run_command(&mut sh("exit 3"), Some(&input), Duration::from_secs(10)).unwrap();
        assert_eq!(out.status.code(), Some(3));
    }

    #[test]
    fn timeout_kills_the_descendants_too_and_repeated_timeouts_leave_nothing() {
        let dir = tempfile::tempdir().unwrap();
        for round in 0..3 {
            let pidfile = dir.path().join(format!("child{round}.pid"));
            // A descendant that keeps stdin, stdout and stderr open.
            let script = format!("sleep 30 & echo $! > '{}'; wait", pidfile.display());
            let res = run_command(&mut sh(&script), Some(b"input"), Duration::from_millis(300));
            assert!(matches!(res, Err(ExecError::Timeout(_))));
            let pid = std::fs::read_to_string(&pidfile).unwrap().trim().to_string();
            assert!(wait_gone(&pid), "descendant {pid} survived the timeout");
        }
    }

    #[test]
    fn stdin_is_delivered_and_exit_codes_and_failures_are_reported() {
        let out = run_command(&mut sh("cat"), Some(b"hello"), Duration::from_secs(10)).unwrap();
        assert_eq!(out.stdout, b"hello");
        let out = run_command(&mut sh("echo oops >&2; exit 1"), None, Duration::from_secs(10)).unwrap();
        assert_eq!((out.status.success(), String::from_utf8_lossy(&out.stderr).trim().to_string()), (false, "oops".into()));
        assert!(run_with_timeout(&mut sh("exit 1"), Duration::from_secs(10)).is_none());
        assert!(matches!(run_command(&mut Command::new("/nonexistent/x"), None, Duration::from_secs(1)), Err(ExecError::Start(_))));
    }

    #[test]
    fn a_descendant_holding_the_pipes_is_killed_and_the_output_flagged_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("bg.pid");
        let started = std::time::Instant::now();
        // The shell exits at once; a background process keeps stdout and stderr open.
        let script = format!("echo done; sleep 30 & echo $! > '{}'", pidfile.display());
        let out = run_command(&mut sh(&script), None, Duration::from_secs(10)).unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "done");
        assert!(out.incomplete && !out.truncated);
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
        let pid = std::fs::read_to_string(&pidfile).unwrap().trim().to_string();
        assert!(wait_gone(&pid), "background process {pid} left behind");
        // A normal run is complete.
        let out = run_command(&mut sh("echo ok"), None, Duration::from_secs(10)).unwrap();
        assert!(!out.incomplete && !out.truncated);
    }

    #[test]
    fn search_path_adds_common_locations() {
        let p = search_path(Some("/x/bin"));
        assert!(p.starts_with("/x/bin:"));
        assert!(p.contains("/opt/homebrew/bin"));
        assert!(which("sh", &p).is_some());
    }
}
