//! Normalization of Claude Code events (adapter verified on 2.1.292,
//! see `docs/compatibility.md`). Codex events go through `crate::codex`.

use crate::spool::SpoolRecord;

pub const TOOL_CLAUDE: &str = "claude";
pub const TOOL_CODEX: &str = "codex";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// `source`: startup | resume | clear | compact | fork
    SessionStart { source: String },
    /// `reason`: clear | resume | logout | prompt_input_exit | other
    SessionEnd { reason: String },
    Prompt,
    ToolActivity,
    /// A question or plan approval (AskUserQuestion / ExitPlanMode).
    Question { tool: String },
    Permission { tool: Option<String> },
    /// Notification that signals required input (e.g. elicitation).
    AttentionNotice { notification_type: String },
    /// Informational Notification (e.g. idle_prompt): only updates the observation.
    InfoNotice { notification_type: String },
    PermissionDenied,
    Stop,
    StopFailure { error: Option<String> },
    /// Turn interrupted by the user (Codex `Interrupt`).
    Interrupt,
    Other { name: String },
}

impl Kind {
    pub fn code(&self) -> String {
        match self {
            Kind::SessionStart { source } => format!("session_start:{source}"),
            Kind::SessionEnd { reason } => format!("session_end:{reason}"),
            Kind::Prompt => "prompt".into(),
            Kind::ToolActivity => "tool".into(),
            Kind::Question { tool } => format!("question:{tool}"),
            Kind::Permission { .. } => "permission".into(),
            Kind::AttentionNotice { notification_type } => format!("notice:{notification_type}"),
            Kind::InfoNotice { notification_type } => format!("info:{notification_type}"),
            Kind::PermissionDenied => "permission_denied".into(),
            Kind::Stop => "stop".into(),
            Kind::StopFailure { .. } => "stop_failure".into(),
            Kind::Interrupt => "interrupt".into(),
            Kind::Other { name } => format!("other:{name}"),
        }
    }

    /// Real activity at the source: it can bring back a removed entry.
    /// Closes, compactions and informational notifications are not enough.
    pub fn is_significant(&self) -> bool {
        match self {
            Kind::SessionStart { source } => source != "compact",
            Kind::Prompt
            | Kind::ToolActivity
            | Kind::Question { .. }
            | Kind::Permission { .. }
            | Kind::AttentionNotice { .. }
            | Kind::PermissionDenied
            | Kind::Stop
            | Kind::StopFailure { .. }
            | Kind::Interrupt => true,
            Kind::SessionEnd { .. } | Kind::InfoNotice { .. } | Kind::Other { .. } => false,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EnvInfo {
    /// Familiar name of the environment: Terminal, iTerm, VS Code...
    pub app: String,
    pub bundle_id: Option<String>,
    pub term_program: Option<String>,
    pub term_session: Option<String>,
    pub tmux: bool,
}

impl EnvInfo {
    pub fn detect(bundle_id: Option<&str>, term_program: Option<&str>, tmux: bool) -> Self {
        let by_bundle = bundle_id.and_then(|b| match b {
            "com.apple.Terminal" => Some("Terminal"),
            "com.googlecode.iterm2" => Some("iTerm"),
            "com.microsoft.VSCode" => Some("VS Code"),
            "com.microsoft.VSCodeInsiders" => Some("VS Code Insiders"),
            "com.todesktop.230313mzl4w4u92" => Some("Cursor"),
            "com.exafunction.windsurf" => Some("Windsurf"),
            "com.mitchellh.ghostty" => Some("Ghostty"),
            "dev.warp.Warp-Stable" => Some("Warp"),
            "net.kovidgoyal.kitty" => Some("kitty"),
            "org.alacritty" => Some("Alacritty"),
            "dev.zed.Zed" => Some("Zed"),
            b if b.starts_with("com.jetbrains.") => Some("JetBrains"),
            _ => None,
        });
        let by_program = term_program.and_then(|p| match p {
            "Apple_Terminal" => Some("Terminal"),
            "iTerm.app" => Some("iTerm"),
            "vscode" => Some("VS Code"),
            "ghostty" => Some("Ghostty"),
            "WarpTerminal" => Some("Warp"),
            "WezTerm" => Some("WezTerm"),
            _ => None,
        });
        EnvInfo {
            app: by_bundle.or(by_program).unwrap_or("Terminal").to_string(),
            bundle_id: bundle_id.map(str::to_string),
            term_program: term_program.map(str::to_string),
            term_session: None,
            tmux,
        }
    }

    /// True when we have no clue about the originating app.
    pub fn is_guess(&self) -> bool {
        self.bundle_id.is_none() && self.term_program.is_none()
    }
}

#[derive(Debug, Clone)]
pub struct Event {
    pub dedup_key: String,
    pub tool: String,
    pub namespace: String,
    pub native_id: String,
    pub kind: Kind,
    pub source_at: i64,
    pub cwd: Option<String>,
    pub transcript_path: Option<String>,
    pub prompt_id: Option<String>,
    pub pid: Option<i64>,
    pub tty: Option<String>,
    pub env: EnvInfo,
    pub entrypoint: Option<String>,
    /// Start of the prompt, used only as a local provisional title.
    pub prompt_snippet: Option<String>,
    /// Short verified reason (e.g. the name of the tool to approve).
    pub detail: Option<String>,
    /// Nonce of the doctor's guided test.
    pub probe: Option<String>,
}

#[derive(Debug)]
pub enum Normalized {
    Event(Box<Event>),
    /// Doctor test event: confirms receipt without creating conversations.
    Probe { nonce: String, source_at: i64 },
    /// Event deliberately not tracked (e.g. `claude -p` sessions).
    Ignored(&'static str),
}

pub struct NormalizeOptions {
    pub track_print_sessions: bool,
}

impl Default for NormalizeOptions {
    fn default() -> Self {
        NormalizeOptions { track_print_sessions: false }
    }
}

pub fn normalize(rec: &SpoolRecord, opts: &NormalizeOptions) -> Normalized {
    if rec.header("TOOL") == Some(TOOL_CODEX) {
        return crate::codex::normalize(rec, opts);
    }
    let p = &rec.payload;
    let name = rec
        .header("EVENT")
        .map(str::to_string)
        .or_else(|| p.str("hook_event_name"))
        .unwrap_or_default();

    if let Some(nonce) = rec.header("FLOW_KEEPER_PROBE") {
        return Normalized::Probe { nonce: nonce.to_string(), source_at: rec.source_ms() };
    }

    let entrypoint = rec.header("CLAUDE_CODE_ENTRYPOINT").map(str::to_string);
    if !opts.track_print_sessions && entrypoint.as_deref().is_some_and(|e| e.starts_with("sdk")) {
        return Normalized::Ignored("non-interactive session");
    }

    let Some(native_id) = p.str("session_id").filter(|s| !s.is_empty()) else {
        return Normalized::Ignored("payload without session_id");
    };

    let mut detail = None;
    let mut prompt_snippet = None;
    let kind = match name.as_str() {
        "SessionStart" => Kind::SessionStart { source: p.str("source").unwrap_or_else(|| "startup".into()) },
        "SessionEnd" => Kind::SessionEnd { reason: p.str("reason").unwrap_or_else(|| "other".into()) },
        "UserPromptSubmit" => {
            prompt_snippet = p.str_prefix("prompt", 200);
            Kind::Prompt
        }
        "PostToolUse" | "PostToolUseFailure" | "PostToolBatch" => Kind::ToolActivity,
        "PreToolUse" => match p.str("tool_name").as_deref() {
            Some(tool @ ("AskUserQuestion" | "ExitPlanMode")) => {
                detail = Some(tool.to_string());
                Kind::Question { tool: tool.to_string() }
            }
            _ => Kind::ToolActivity,
        },
        "PermissionRequest" => {
            let tool = p.str("tool_name");
            detail = tool.clone();
            Kind::Permission { tool }
        }
        "PermissionDenied" => Kind::PermissionDenied,
        "Notification" => {
            let nt = p.str("notification_type").unwrap_or_default();
            match nt.as_str() {
                "permission_prompt" | "elicitation_dialog" | "elicitation_url_dialog" | "agent_needs_input" => {
                    detail = p.str_prefix("message", 160);
                    Kind::AttentionNotice { notification_type: nt }
                }
                _ => Kind::InfoNotice { notification_type: nt },
            }
        }
        "Stop" => Kind::Stop,
        "StopFailure" => {
            let error = p.str("error_type").or_else(|| p.str("error"));
            detail = error.clone();
            Kind::StopFailure { error }
        }
        other => Kind::Other { name: other.to_string() },
    };

    Normalized::Event(Box::new(Event {
        dedup_key: rec.digest.clone(),
        tool: TOOL_CLAUDE.into(),
        namespace: rec.header("CLAUDE_CONFIG_DIR").unwrap_or("default").to_string(),
        native_id,
        kind,
        source_at: rec.source_ms(),
        cwd: p.str("cwd"),
        transcript_path: p.str("transcript_path"),
        prompt_id: p.str("prompt_id"),
        pid: record_pid(rec),
        tty: record_tty(rec),
        env: record_env(rec),
        entrypoint,
        prompt_snippet,
        detail,
        probe: None,
    }))
}

/// Originating environment declared in the spool header.
pub fn record_env(rec: &SpoolRecord) -> EnvInfo {
    let mut env = EnvInfo::detect(
        rec.header("__CFBundleIdentifier"),
        rec.header("TERM_PROGRAM"),
        rec.header("TMUX").is_some(),
    );
    env.term_session = rec
        .header("ITERM_SESSION_ID")
        .or_else(|| rec.header("TERM_SESSION_ID"))
        .map(str::to_string);
    env
}

pub fn record_tty(rec: &SpoolRecord) -> Option<String> {
    rec.header("TTY").filter(|t| t.starts_with("tty")).map(|t| format!("/dev/{t}"))
}

pub fn record_pid(rec: &SpoolRecord) -> Option<i64> {
    rec.header("PID").and_then(|v| v.parse().ok()).filter(|pid: &i64| *pid > 1)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::spool::parse_bytes;
    use std::path::Path;

    pub(crate) fn rec(header: &str, payload: &str) -> SpoolRecord {
        let raw = format!("flow-keeper-spool 1\n{header}\n\n{payload}");
        parse_bytes(Path::new("t.evt"), raw.as_bytes(), 5_000_000).unwrap()
    }

    pub(crate) fn event(n: Normalized) -> Event {
        match n {
            Normalized::Event(e) => *e,
            other => panic!("expected an event, found {other:?}"),
        }
    }

    #[test]
    fn session_start_with_environment() {
        let r = rec(
            "EVENT=SessionStart\nPID=900\nTTY=ttys004\nCLAUDE_CODE_ENTRYPOINT=cli\n__CFBundleIdentifier=com.googlecode.iterm2\nTERM_PROGRAM=iTerm.app\nITERM_SESSION_ID=w0t1p0:ABC",
            r#"{"session_id":"s-1","cwd":"/repo","transcript_path":"/t.jsonl","source":"resume"}"#,
        );
        let e = event(normalize(&r, &Default::default()));
        assert_eq!(e.kind, Kind::SessionStart { source: "resume".into() });
        assert_eq!(e.pid, Some(900));
        assert_eq!(e.tty.as_deref(), Some("/dev/ttys004"));
        assert_eq!(e.env.app, "iTerm");
        assert_eq!(e.env.term_session.as_deref(), Some("w0t1p0:ABC"));
        assert_eq!(e.namespace, "default");
    }

    #[test]
    fn print_sessions_are_ignored_by_default() {
        let r = rec("EVENT=Stop\nCLAUDE_CODE_ENTRYPOINT=sdk-cli", r#"{"session_id":"s"}"#);
        assert!(matches!(normalize(&r, &Default::default()), Normalized::Ignored(_)));
        let opts = NormalizeOptions { track_print_sessions: true };
        assert!(matches!(normalize(&r, &opts), Normalized::Event(_)));
    }

    #[test]
    fn notification_types_map_to_attention_or_info() {
        let r = rec("EVENT=Notification", r#"{"session_id":"s","notification_type":"permission_prompt","message":"Claude needs your permission to use Bash"}"#);
        let e = event(normalize(&r, &Default::default()));
        assert!(matches!(e.kind, Kind::AttentionNotice { .. }));
        assert_eq!(e.detail.as_deref(), Some("Claude needs your permission to use Bash"));
        let r = rec("EVENT=Notification", r#"{"session_id":"s","notification_type":"idle_prompt"}"#);
        assert!(matches!(event(normalize(&r, &Default::default())).kind, Kind::InfoNotice { .. }));
    }

    #[test]
    fn pre_tool_use_question_is_attention() {
        let r = rec("EVENT=PreToolUse", r#"{"session_id":"s","tool_name":"AskUserQuestion"}"#);
        let e = event(normalize(&r, &Default::default()));
        assert_eq!(e.kind, Kind::Question { tool: "AskUserQuestion".into() });
    }

    #[test]
    fn probe_events_are_separated() {
        let r = rec("EVENT=SessionStart\nFLOW_KEEPER_PROBE=n123\nCLAUDE_CODE_ENTRYPOINT=sdk-cli", r#"{"session_id":"s"}"#);
        assert!(matches!(normalize(&r, &Default::default()), Normalized::Probe { nonce, .. } if nonce == "n123"));
    }

    #[test]
    fn unknown_environment_defaults_to_terminal() {
        let env = EnvInfo::detect(None, None, false);
        assert_eq!(env.app, "Terminal");
        assert!(env.is_guess());
    }
}
