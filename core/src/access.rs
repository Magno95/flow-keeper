//! Access to the conversation: bring the open session to the front or resume
//! it with the right CLI. Commands are built only from validated values
//! (native ID, existing folder), never from model or transcript text.

use std::path::Path;
use std::process::Command;

use anyhow::{bail, Result};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

#[derive(Debug, Clone)]
pub struct Target {
    pub tool: String,
    pub native_id: String,
    pub cwd: Option<String>,
    pub live: bool,
    pub pid: Option<i64>,
    pub tty: Option<String>,
    pub bundle_id: Option<String>,
    pub env_app: Option<String>,
    pub term_program: Option<String>,
}

pub fn target(conn: &Connection, id: i64) -> Result<Option<Target>> {
    Ok(conn
        .query_row(
            "SELECT c.tool, c.native_id, c.cwd, c.state, i.ended_at, i.pid, i.tty, i.bundle_id, i.env_app, i.term_program
             FROM conversations c LEFT JOIN instances i ON i.id = c.current_instance_id WHERE c.id = ?1",
            params![id],
            |r| {
                let state: String = r.get(3)?;
                let ended: Option<i64> = r.get(4)?;
                let pid: Option<i64> = r.get(5)?;
                Ok(Target {
                    tool: r.get(0)?,
                    native_id: r.get(1)?,
                    cwd: r.get(2)?,
                    live: state != crate::engine::state::CLOSED && ended.is_none() && pid.is_some(),
                    pid,
                    tty: r.get(6)?,
                    bundle_id: r.get(7)?,
                    env_app: r.get(8)?,
                    term_program: r.get(9)?,
                })
            },
        )
        .optional()?)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum AccessResult {
    /// The session is in front.
    Focused { app: String },
    /// The app is in front but the exact terminal could not be selected.
    AppActivated { app: String, note: String },
    /// New session started by resuming.
    Resumed { app: String },
    /// Destination unreachable; the command is a secondary fallback.
    Unreachable { message: String, command: Option<String> },
    /// The recorded folder no longer exists: a new one must be chosen.
    NeedsFolder { message: String },
}

pub fn valid_native_id(id: &str) -> bool {
    Regex::new(r"^[A-Za-z0-9][A-Za-z0-9_-]{7,127}$").map(|re| re.is_match(id)).unwrap_or(false)
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Resume command for the tool, with syntax verified for the supported version.
pub fn resume_command(t: &Target) -> Result<String> {
    if !valid_native_id(&t.native_id) {
        bail!("invalid conversation identifier");
    }
    let Some(cwd) = t.cwd.as_deref().filter(|c| Path::new(c).is_dir()) else {
        bail!("folder not available");
    };
    match t.tool.as_str() {
        "claude" => Ok(format!("cd {} && claude --resume {}", shell_quote(cwd), t.native_id)),
        "codex" => Ok(format!("cd {} && codex resume {}", shell_quote(cwd), t.native_id)),
        other => bail!("resume not supported for {other}"),
    }
}

fn osascript(lines: &[&str], args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("/usr/bin/osascript");
    for l in lines {
        cmd.arg("-e").arg(l);
    }
    cmd.args(args);
    let out = cmd.output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

const TERMINAL_FOCUS: &[&str] = &[
    "on run argv",
    "set targetTty to item 1 of argv",
    "if application \"Terminal\" is not running then return \"notrunning\"",
    "tell application \"Terminal\"",
    "repeat with w in windows",
    "repeat with t in tabs of w",
    "if tty of t is targetTty then",
    "set selected of t to true",
    "set index of w to 1",
    "activate",
    "return \"ok\"",
    "end if",
    "end repeat",
    "end repeat",
    "end tell",
    "return \"notfound\"",
    "end run",
];

const ITERM_FOCUS: &[&str] = &[
    "on run argv",
    "set targetTty to item 1 of argv",
    "if application \"iTerm2\" is not running then return \"notrunning\"",
    "tell application \"iTerm2\"",
    "repeat with w in windows",
    "repeat with t in tabs of w",
    "repeat with s in sessions of t",
    "if tty of s is targetTty then",
    "select w",
    "select t",
    "select s",
    "activate",
    "return \"ok\"",
    "end if",
    "end repeat",
    "end repeat",
    "end repeat",
    "end tell",
    "return \"notfound\"",
    "end run",
];

fn activate_bundle(bundle: &str) -> bool {
    Command::new("/usr/bin/open").args(["-b", bundle]).status().map(|s| s.success()).unwrap_or(false)
}

fn is_terminal(t: &Target) -> bool {
    t.bundle_id.as_deref() == Some("com.apple.Terminal") || t.term_program.as_deref() == Some("Apple_Terminal")
}

fn is_iterm(t: &Target) -> bool {
    t.bundle_id.as_deref() == Some("com.googlecode.iterm2") || t.term_program.as_deref() == Some("iTerm.app")
}

/// Open conversation: brings the already open instance to the front.
pub fn focus(t: &Target) -> AccessResult {
    // Label saved by versions before the English translation.
    let app = match t.env_app.as_deref() {
        None | Some("Terminale") => "Terminal".to_string(),
        Some(app) => app.to_string(),
    };
    let fallback = resume_command(t).ok();
    if !t.live {
        return AccessResult::Unreachable {
            message: "The conversation is no longer open in a terminal.".into(),
            command: fallback,
        };
    }
    if let Some(tty) = t.tty.as_deref() {
        let script = if is_terminal(t) {
            Some(TERMINAL_FOCUS)
        } else if is_iterm(t) {
            Some(ITERM_FOCUS)
        } else {
            None
        };
        if let Some(script) = script {
            if osascript(script, &[tty]).as_deref() == Some("ok") {
                return AccessResult::Focused { app };
            }
        }
    }
    if let Some(result) = crate::editors::focus(t, &app) {
        return result;
    }
    if let Some(bundle) = t.bundle_id.as_deref() {
        if activate_bundle(bundle) {
            return AccessResult::AppActivated {
                app: app.clone(),
                note: format!("{app} is in front: select the conversation's terminal."),
            };
        }
    }
    AccessResult::Unreachable { message: "Can't reach this terminal.".into(), command: fallback }
}

/// Resume: starts the CLI in the recorded folder. Never starts a second
/// agent on a conversation that is still active.
pub fn resume(t: &Target) -> AccessResult {
    if t.live {
        return focus(t);
    }
    if !t.cwd.as_deref().is_some_and(|c| Path::new(c).is_dir()) {
        return AccessResult::NeedsFolder {
            message: "The conversation's folder no longer exists. Choose where to resume it.".into(),
        };
    }
    let command = match resume_command(t) {
        Ok(c) => c,
        Err(e) => return AccessResult::Unreachable { message: format!("Can't resume the conversation: {e}."), command: None },
    };
    let (app, ok) = if is_iterm(t) {
        let script = [
            "on run argv",
            "tell application \"iTerm2\"",
            "set w to (create window with default profile)",
            "tell current session of w to write text (item 1 of argv)",
            "activate",
            "end tell",
            "end run",
        ];
        ("iTerm".to_string(), osascript(&script, &[&command]).is_some())
    } else {
        let script = [
            "on run argv",
            "tell application \"Terminal\"",
            "do script (item 1 of argv)",
            "activate",
            "end tell",
            "end run",
        ];
        ("Terminal".to_string(), osascript(&script, &[&command]).is_some())
    };
    if ok {
        AccessResult::Resumed { app }
    } else {
        AccessResult::Unreachable { message: "Can't open a new terminal.".into(), command: Some(command) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(id: &str, cwd: &str) -> Target {
        Target {
            tool: "claude".into(),
            native_id: id.into(),
            cwd: Some(cwd.into()),
            live: false,
            pid: None,
            tty: None,
            bundle_id: None,
            env_app: None,
            term_program: None,
        }
    }

    #[test]
    fn resume_command_is_built_from_validated_parts() {
        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("it's here");
        std::fs::create_dir(&odd).unwrap();
        let cmd = resume_command(&t("06f55790-efe7-498b-b053-66a90e022f50", odd.to_str().unwrap())).unwrap();
        assert!(cmd.ends_with("&& claude --resume 06f55790-efe7-498b-b053-66a90e022f50"));
        assert!(cmd.contains(r"it'\''s here"));
        assert!(resume_command(&t("abc; rm -rf ~", odd.to_str().unwrap())).is_err());
        let codex = Target { tool: "codex".into(), ..t("01a15f10-862c-7570-ba53-4216b8e95b37", odd.to_str().unwrap()) };
        assert!(resume_command(&codex).unwrap().ends_with("&& codex resume 01a15f10-862c-7570-ba53-4216b8e95b37"));
        assert!(resume_command(&t("06f55790-efe7-498b", "/does/not/exist")).is_err());
    }

    #[test]
    fn missing_folder_asks_for_a_new_one() {
        assert!(matches!(resume(&t("06f55790-efe7-498b", "/does/not/exist")), AccessResult::NeedsFolder { .. }));
    }

    #[test]
    fn closed_conversation_cannot_be_focused() {
        assert!(matches!(focus(&t("06f55790-efe7-498b", "/tmp")), AccessResult::Unreachable { .. }));
    }
}
