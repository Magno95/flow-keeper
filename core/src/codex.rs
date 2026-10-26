//! Codex CLI adapter (documented hooks, see `docs/compatibility.md`).
//!
//! Codex hooks also fire for the desktop app, editor extensions, `codex exec`
//! and subagents (e.g. the automatic review of approvals). Only interactive
//! terminal sessions are tracked: the origin is read from the first line of
//! the rollout (`session_meta`).

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use regex::Regex;
use serde_json::Value;

use crate::event::{record_env, record_pid, record_tty, Event, Kind, NormalizeOptions, Normalized, TOOL_CODEX};
use crate::spool::{Payload, SpoolRecord};

/// `originator` of the terminal interface sessions.
const TUI_ORIGINATORS: &[&str] = &["codex-tui", "codex_cli_rs"];
const EXEC_ORIGINATOR: &str = "codex_exec";
/// Tool Codex uses to ask the user a question.
pub const QUESTION_TOOL: &str = "request_user_input";

/// `CODEX_HOME`, normally `~/.codex`.
pub fn home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::paths::home().join(".codex"))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Origin {
    pub originator: Option<String>,
    /// `cli`, `vscode`, `exec`...; absent for subagents.
    pub source: Option<String>,
    pub subagent: bool,
    pub thread_source: Option<String>,
}

impl Origin {
    /// Why the session should not be tracked, if it should not.
    pub fn exclusion(&self, opts: &NormalizeOptions) -> Option<&'static str> {
        if self.subagent || matches!(self.thread_source.as_deref(), Some("guardian_review" | "subagent")) {
            return Some("Codex subagent");
        }
        let exec = self.originator.as_deref() == Some(EXEC_ORIGINATOR) || self.source.as_deref() == Some("exec");
        if exec {
            return (!opts.track_print_sessions).then_some("non-interactive session");
        }
        match self.originator.as_deref() {
            Some(o) if !TUI_ORIGINATORS.contains(&o) => Some("Codex app session"),
            _ => None,
        }
    }
}

/// Session origin from the `session_meta` at the top of the rollout. The line
/// can exceed the read limit (it contains the base instructions): in that case
/// the fields are extracted from the prefix, where they come before the instructions.
pub fn read_origin(path: &Path) -> Option<Origin> {
    let mut buf = Vec::new();
    File::open(path).ok()?.take(128 * 1024).read_to_end(&mut buf).ok()?;
    let line = buf.split(|b| *b == b'\n').next()?;
    if let Ok(v) = serde_json::from_slice::<Value>(line) {
        if v.get("type").and_then(Value::as_str) != Some("session_meta") {
            return None;
        }
        let p = v.get("payload")?;
        let text = |k: &str| p.get(k).and_then(Value::as_str).map(str::to_string);
        return Some(Origin {
            originator: text("originator"),
            source: text("source"),
            subagent: p.get("source").is_some_and(|s| s.get("subagent").is_some()),
            thread_source: text("thread_source"),
        });
    }
    let raw = String::from_utf8_lossy(line);
    if !raw.contains("\"session_meta\"") {
        return None;
    }
    let prefix = Payload::parse(&raw);
    let subagent = Regex::new(r#""source"\s*:\s*\{\s*"subagent""#).ok()?.is_match(&raw);
    Some(Origin {
        originator: prefix.str("originator"),
        source: if subagent { None } else { prefix.str("source") },
        subagent,
        thread_source: prefix.str("thread_source"),
    })
}

pub fn normalize(rec: &SpoolRecord, opts: &NormalizeOptions) -> Normalized {
    let p = &rec.payload;
    let name = rec
        .header("EVENT")
        .map(str::to_string)
        .or_else(|| p.str("hook_event_name"))
        .unwrap_or_default();

    if let Some(nonce) = rec.header("FLOW_KEEPER_PROBE") {
        return Normalized::Probe { nonce: nonce.to_string(), source_at: rec.source_ms() };
    }
    let Some(native_id) = p.str("session_id").filter(|s| !s.is_empty()) else {
        return Normalized::Ignored("payload without session_id");
    };
    let transcript_path = p.str("transcript_path").filter(|t| !t.is_empty());
    // Rollout not written yet: trust the event, the other filters still apply.
    let origin = transcript_path.as_deref().and_then(|t| read_origin(Path::new(t)));
    if let Some(why) = origin.as_ref().and_then(|o| o.exclusion(opts)) {
        return Normalized::Ignored(why);
    }
    // The TUI uses app-server too (shared daemon in Codex 0.163). The host
    // does not tell CLI and app apart: when the rollout exists, the originator
    // decides. Without metadata, we do not import desktop sessions by mistake.
    if rec.header("CODEX_HOST") == Some("app-server") && origin.is_none() {
        return Normalized::Ignored("Codex session origin not available");
    }

    let mut detail = None;
    let mut prompt_snippet = None;
    let kind = match name.as_str() {
        "SessionStart" => Kind::SessionStart { source: p.str("source").unwrap_or_else(|| "startup".into()) },
        "SessionEnd" => Kind::SessionEnd { reason: p.str("reason").unwrap_or_else(|| "other".into()) },
        "UserPromptSubmit" => {
            prompt_snippet = p.str_prefix("prompt", 200);
            Kind::Prompt
        }
        "PostToolUse" => Kind::ToolActivity,
        "PreToolUse" => match p.str("tool_name") {
            Some(tool) if tool == QUESTION_TOOL => {
                detail = Some(tool.clone());
                Kind::Question { tool }
            }
            _ => Kind::ToolActivity,
        },
        "PermissionRequest" => {
            let tool = p.str("tool_name");
            detail = tool.clone();
            Kind::Permission { tool }
        }
        "Stop" => Kind::Stop,
        "Interrupt" => Kind::Interrupt,
        other => Kind::Other { name: other.to_string() },
    };

    Normalized::Event(Box::new(Event {
        dedup_key: rec.digest.clone(),
        tool: TOOL_CODEX.into(),
        namespace: rec.header("CODEX_HOME").unwrap_or("default").to_string(),
        native_id,
        kind,
        source_at: rec.source_ms(),
        cwd: p.str("cwd"),
        transcript_path,
        prompt_id: p.str("turn_id"),
        pid: record_pid(rec),
        tty: record_tty(rec),
        env: record_env(rec),
        entrypoint: origin.and_then(|o| o.originator),
        prompt_snippet,
        detail,
        probe: None,
    }))
}

/// Name Codex assigns to the session by itself (`session_index.jsonl` in its
/// folder), looked up from the rollout `<home>/sessions/YYYY/MM/DD/rollout-…-<id>.jsonl`.
pub fn thread_name(rollout: &Path) -> Option<String> {
    let stem = rollout.file_stem()?.to_str()?;
    if !stem.starts_with("rollout-") || stem.len() < 36 {
        return None;
    }
    let id = &stem[stem.len() - 36..];
    let sessions = rollout.ancestors().nth(4)?;
    if sessions.file_name()? != "sessions" {
        return None;
    }
    let mut file = File::open(sessions.parent()?.join("session_index.jsonl")).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(256 * 1024))).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    buf.split(|b| *b == b'\n')
        .rev()
        .filter_map(|l| serde_json::from_slice::<Value>(l).ok())
        .find(|v| v.get("id").and_then(Value::as_str) == Some(id))
        .and_then(|v| v.get("thread_name").and_then(Value::as_str).map(|t| t.trim().to_string()))
        .filter(|t| !t.is_empty())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HookTrust {
    pub total: usize,
    pub trusted: usize,
    pub disabled: usize,
}

/// Actual state of the hooks: Codex compares the current hash and resolves the
/// enablement settings. No heuristic reading of `config.toml`.
pub fn hook_trust(command: &mut Command, cwd: &Path, plugin_id: &str) -> Result<HookTrust> {
    let response = list_hooks(command, cwd, Duration::from_secs(10))?;
    hook_trust_from_response(&response, plugin_id)
}

fn hook_trust_from_response(response: &Value, plugin_id: &str) -> Result<HookTrust> {
    let entries = response.get("data").and_then(Value::as_array).context("hooks/list response without data")?;
    if entries.len() != 1 {
        bail!("incomplete hooks/list response");
    }
    let entry = &entries[0];
    let errors = entry.get("errors").and_then(Value::as_array).context("hooks/list response without errors")?;
    if !errors.is_empty() {
        bail!("Codex reports errors loading the hooks");
    }
    let hooks = entry.get("hooks").and_then(Value::as_array).context("hooks/list response without hooks")?;
    let mut trust = HookTrust::default();
    for hook in hooks {
        if hook.get("pluginId").and_then(Value::as_str) != Some(plugin_id) {
            continue;
        }
        trust.total += 1;
        let enabled = hook.get("enabled").and_then(Value::as_bool).context("hook without enabled")?;
        let status = hook.get("trustStatus").and_then(Value::as_str).context("hook without trustStatus")?;
        if !enabled {
            trust.disabled += 1;
        } else if matches!(status, "trusted" | "managed") {
            trust.trusted += 1;
        }
    }
    Ok(trust)
}

/// The probe process starts no threads or turns and runs no hooks. It is always
/// killed and waited for, even on timeout or an invalid response.
struct HookServer(Child);

impl Drop for HookServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub(crate) fn list_hooks(command: &mut Command, cwd: &Path, timeout: Duration) -> Result<Value> {
    let mut server = HookServer(command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?);
    let mut input = server.0.stdin.take().context("app-server stdin not available")?;
    let output = server.0.stdout.take().context("app-server stdout not available")?;
    let (send, recv) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if send.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + timeout;
    let response = |id: u64| -> Result<Value> {
        loop {
            let line = recv.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| anyhow!("Codex did not answer hooks/list in time"))??;
            let value: Value = serde_json::from_str(&line).context("invalid app-server response")?;
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if value.get("error").is_some() {
                bail!("Codex does not support checking hooks or returned an error");
            }
            return value.get("result").cloned().context("app-server response without result");
        }
    };
    writeln!(input, "{}", serde_json::json!({"id":1,"method":"initialize","params":{
        "clientInfo":{"name":"flow_keeper","title":"Flow Keeper","version":env!("CARGO_PKG_VERSION")}
    }}))?;
    response(1)?;
    writeln!(input, "{}", serde_json::json!({"method":"initialized"}))?;
    writeln!(input, "{}", serde_json::json!({"id":2,"method":"hooks/list","params":{"cwds":[cwd]}}))?;
    response(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::tests::{event, rec};
    use std::fs;

    fn rollout(dir: &Path, meta: Value) -> PathBuf {
        let day = dir.join("sessions/2026/10/22");
        fs::create_dir_all(&day).unwrap();
        let p = day.join("rollout-2026-10-22T10-00-00-01a15f10-862c-7570-ba53-4216b8e95b37.jsonl");
        fs::write(&p, format!("{}\n", serde_json::json!({"type":"session_meta","payload":meta}))).unwrap();
        p
    }

    fn codex_rec(event: &str, payload: Value) -> SpoolRecord {
        rec(&format!("TOOL=codex\nEVENT={event}\nPID=4242\nTTY=ttys007\nTERM_PROGRAM=Apple_Terminal"), &payload.to_string())
    }

    #[test]
    fn tui_session_events_are_tracked() {
        let dir = tempfile::tempdir().unwrap();
        let t = rollout(dir.path(), serde_json::json!({"originator":"codex-tui","source":"cli","thread_source":"user"}));
        let e = event(normalize(
            &codex_rec("UserPromptSubmit", serde_json::json!({"session_id":"01a15f10-862c","transcript_path":t,"cwd":"/repo","turn_id":"t1","prompt":"Fix the login"})),
            &Default::default(),
        ));
        assert_eq!(e.tool, "codex");
        assert_eq!(e.kind, Kind::Prompt);
        assert_eq!(e.prompt_id.as_deref(), Some("t1"));
        assert_eq!(e.prompt_snippet.as_deref(), Some("Fix the login"));
        assert_eq!(e.pid, Some(4242));
        assert_eq!(e.tty.as_deref(), Some("/dev/ttys007"));
        assert_eq!(e.env.app, "Terminal");
        assert_eq!(e.entrypoint.as_deref(), Some("codex-tui"));
    }

    #[test]
    fn app_exec_and_subagent_sessions_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let ignored = |meta: Value, opts: &NormalizeOptions| {
            let t = rollout(dir.path(), meta);
            let r = codex_rec("Stop", serde_json::json!({"session_id":"s-123456789","transcript_path":t}));
            matches!(normalize(&r, opts), Normalized::Ignored(_))
        };
        let opts = NormalizeOptions::default();
        assert!(ignored(serde_json::json!({"originator":"codex_work_desktop","source":"vscode"}), &opts));
        assert!(ignored(serde_json::json!({"originator":"codex-tui","source":{"subagent":{"other":"guardian"}},"thread_source":"guardian_review"}), &opts));
        assert!(ignored(serde_json::json!({"originator":"codex_exec","source":"exec"}), &opts));
        assert!(!ignored(serde_json::json!({"originator":"codex_exec","source":"exec"}), &NormalizeOptions { track_print_sessions: true }));

        let r = rec("TOOL=codex\nEVENT=Stop\nCODEX_HOST=app-server", r#"{"session_id":"s-123456789"}"#);
        assert!(matches!(normalize(&r, &opts), Normalized::Ignored(_)));
    }

    #[test]
    fn missing_rollout_trusts_the_event() {
        let r = codex_rec("SessionStart", serde_json::json!({"session_id":"s-123456789","transcript_path":"/does/not/exist.jsonl","source":"startup"}));
        let e = event(normalize(&r, &Default::default()));
        assert_eq!(e.kind, Kind::SessionStart { source: "startup".into() });
        assert_eq!(e.entrypoint, None);
    }

    #[test]
    fn app_server_can_host_a_terminal_session() {
        let dir = tempfile::tempdir().unwrap();
        let t = rollout(dir.path(), serde_json::json!({"originator":"codex-tui","source":"vscode","thread_source":"user"}));
        let r = rec("TOOL=codex\nEVENT=UserPromptSubmit\nPID=4242\nCODEX_HOST=app-server", &serde_json::json!({
            "session_id":"s-123456789","transcript_path":t,"turn_id":"t1","prompt":"Check the login"
        }).to_string());
        let e = event(normalize(&r, &Default::default()));
        assert_eq!(e.kind, Kind::Prompt);
        assert_eq!(e.entrypoint.as_deref(), Some("codex-tui"));
    }

    #[test]
    fn event_mapping() {
        let kind = |name: &str, extra: Value| {
            let mut p = serde_json::json!({"session_id":"s-123456789"});
            p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            event(normalize(&codex_rec(name, p), &Default::default())).kind
        };
        assert_eq!(kind("PreToolUse", serde_json::json!({"tool_name":"request_user_input"})), Kind::Question { tool: "request_user_input".into() });
        assert_eq!(kind("PreToolUse", serde_json::json!({"tool_name":"Bash"})), Kind::ToolActivity);
        assert_eq!(kind("PermissionRequest", serde_json::json!({"tool_name":"Bash"})), Kind::Permission { tool: Some("Bash".into()) });
        assert_eq!(kind("Interrupt", serde_json::json!({})), Kind::Interrupt);
        assert_eq!(kind("SessionEnd", serde_json::json!({})), Kind::SessionEnd { reason: "other".into() });
    }

    #[test]
    fn origin_from_truncated_first_line() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("r.jsonl");
        let long = "x".repeat(200 * 1024);
        fs::write(&p, format!(r#"{{"type":"session_meta","payload":{{"originator":"codex-tui","source":"cli","base_instructions":"{long}"}}}}"#)).unwrap();
        let o = read_origin(&p).unwrap();
        assert_eq!(o.originator.as_deref(), Some("codex-tui"));
        assert_eq!(o.source.as_deref(), Some("cli"));
        assert!(!o.subagent);
    }

    #[test]
    fn thread_name_from_session_index() {
        let dir = tempfile::tempdir().unwrap();
        let t = rollout(dir.path(), serde_json::json!({"originator":"codex-tui"}));
        fs::write(
            dir.path().join("session_index.jsonl"),
            concat!(
                r#"{"id":"01a15f10-862c-7570-ba53-4216b8e95b37","thread_name":"Old","updated_at":"x"}"#, "\n",
                r#"{"id":"other","thread_name":"Other","updated_at":"x"}"#, "\n",
                r#"{"id":"01a15f10-862c-7570-ba53-4216b8e95b37","thread_name":"Broken login","updated_at":"y"}"#, "\n",
            ),
        )
        .unwrap();
        assert_eq!(thread_name(&t).as_deref(), Some("Broken login"));
        assert_eq!(thread_name(Path::new("/tmp/t.jsonl")), None);
    }

    #[test]
    fn hook_trust_uses_current_status_and_enabled_flag() {
        let id = "flow-keeper@flow-keeper-app";
        let hook = |plugin: &str, status: &str, enabled: bool| serde_json::json!({
            "pluginId":plugin,"trustStatus":status,"enabled":enabled
        });
        let response = serde_json::json!({"data":[{"errors":[],"hooks":[
            hook(id, "trusted", true), hook(id, "managed", true),
            hook(id, "modified", true), hook(id, "untrusted", true),
            hook(id, "trusted", false), hook(id, "futureStatus", true),
            hook("other@plugin", "trusted", true)
        ]}]});
        assert_eq!(hook_trust_from_response(&response, id).unwrap(), HookTrust { total: 6, trusted: 2, disabled: 1 });
        for response in [
            serde_json::json!({"data":[]}),
            serde_json::json!({"data":[{"errors":[{"message":"plugin not readable"}],"hooks":[]}]}),
            serde_json::json!({"data":[{"errors":[],"hooks":[{"pluginId":id,"enabled":true}]}]}),
        ] {
            assert!(hook_trust_from_response(&response, id).is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn hooks_rpc_initializes_before_listing_and_ignores_notifications() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", r#"read -r request
case "$request" in *'"method":"initialize"'*) ;; *) exit 1 ;; esac
printf '%s\n' '{"id":1,"result":{}}'
read -r request
case "$request" in *'"method":"initialized"'*) ;; *) exit 2 ;; esac
read -r request
case "$request" in *'"method":"hooks/list"'*) ;; *) exit 3 ;; esac
printf '%s\n' '{"method":"notification"}' '{"id":2,"result":{"data":[{"errors":[],"hooks":[]}]}}'
read -r request
"#]);
        assert_eq!(list_hooks(&mut command, Path::new("/tmp"), Duration::from_secs(2)).unwrap()["data"][0]["hooks"], serde_json::json!([]));
    }

    #[cfg(unix)]
    #[test]
    fn hooks_rpc_fails_on_unsupported_method_or_timeout() {
        let mut error = Command::new("/bin/sh");
        error.args(["-c", "printf '%s\\n' '{\"id\":1,\"error\":{\"code\":-32601}}'"]);
        assert!(list_hooks(&mut error, Path::new("/tmp"), Duration::from_secs(2)).is_err());
        let mut slow = Command::new("/bin/sh");
        slow.args(["-c", "exec sleep 5"]);
        let start = Instant::now();
        assert!(list_hooks(&mut slow, Path::new("/tmp"), Duration::from_millis(100)).is_err());
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
