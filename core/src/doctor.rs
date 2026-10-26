//! “Check connections”: verifies configuration, version, queue, event
//! delivery and recaps. The plugin being present is not enough: the guided
//! test checks that a real event makes it all the way to the store.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;
use rusqlite::Connection;
use serde::Serialize;

use crate::paths::Paths;
use crate::settings::{RecapProviderKind, Settings};
use crate::store;
use crate::connect::{self, CodexPluginStatus, PluginStatus};
use crate::system;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Ok,
    Warn,
    Error,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: &'static str,
    pub label: String,
    pub level: Level,
    pub detail: String,
    /// Technical details, shown only in the advanced diagnostics.
    pub advanced: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Environment {
    pub claude_path: Option<String>,
    pub claude_version: Option<String>,
    pub plugin: Option<PluginStatus>,
    pub codex_path: Option<String>,
    pub codex_version: Option<String>,
    pub codex_plugin: Option<CodexPluginStatus>,
    pub api_key_present: bool,
}

impl Environment {
    pub fn claude_connected(&self) -> bool {
        self.plugin.as_ref().is_some_and(|p| p.installed && p.enabled)
    }

    pub fn codex_connected(&self) -> bool {
        self.codex_plugin.as_ref().is_some_and(|p| p.plugin.installed && p.plugin.enabled)
    }
}

pub fn detect(path_env: &str) -> Environment {
    let claude = system::which("claude", path_env);
    let version = claude.as_deref().and_then(|c| system::claude_version(c, path_env));
    let plugin = claude.as_deref().and_then(|c| connect::status(c, path_env));
    let codex = system::which("codex", path_env);
    let codex_version = codex.as_deref().and_then(|c| system::codex_version(c, path_env));
    let codex_plugin = codex.as_deref().and_then(|c| connect::codex_status(c, path_env));
    Environment {
        claude_path: claude.map(|p| p.to_string_lossy().to_string()),
        claude_version: version,
        plugin,
        codex_path: codex.map(|p| p.to_string_lossy().to_string()),
        codex_version,
        codex_plugin,
        api_key_present: system::api_key().is_some(),
    }
}

pub fn checks(conn: &Connection, paths: &Paths, env: &Environment, settings: &Settings, now: i64) -> Result<Vec<Check>> {
    let mut out = Vec::new();

    out.push(match (&env.claude_path, &env.claude_version) {
        (None, _) => Check {
            id: "claude",
            label: "Claude Code".into(),
            level: Level::Error,
            detail: "Claude Code is not installed or cannot be reached.".into(),
            advanced: None,
        },
        (Some(path), version) => {
            let v = version.clone().unwrap_or_else(|| "unknown".into());
            let tested = system::TESTED_CLAUDE_VERSIONS.contains(&v.as_str());
            Check {
                id: "claude",
                label: "Claude Code".into(),
                level: if tested { Level::Ok } else { Level::Warn },
                detail: if tested {
                    format!("Version {v}, verified.")
                } else {
                    format!("Version {v}: not yet verified with Flow Keeper. States may be incomplete.")
                },
                advanced: Some(format!("executable: {path}; verified versions: {}", system::TESTED_CLAUDE_VERSIONS.join(", "))),
            }
        }
    });

    let plugin_info = |p: &PluginStatus| {
        format!(
            "plugin {} {}{}; hooks in {}",
            p.id.clone().unwrap_or_default(),
            p.version.clone().unwrap_or_default(),
            if p.managed { " (managed by the app)" } else { " (installed by hand)" },
            p.install_path.clone().unwrap_or_default()
        )
    };
    out.push(match &env.plugin {
        Some(p) if p.installed && p.enabled && (p.up_to_date || !p.managed) => Check {
            id: "plugin",
            label: "Claude Code connection".into(),
            level: Level::Ok,
            detail: "Active.".into(),
            advanced: Some(plugin_info(p)),
        },
        Some(p) if p.installed && p.enabled => Check {
            id: "plugin",
            label: "Claude Code connection".into(),
            level: Level::Warn,
            detail: "Active, but needs an update: use “Connect your agents”.".into(),
            advanced: Some(plugin_info(p)),
        },
        Some(p) if p.installed => Check {
            id: "plugin",
            label: "Claude Code connection".into(),
            level: Level::Error,
            detail: "Disabled in Claude Code.".into(),
            advanced: Some(format!("claude plugin enable {}", p.id.clone().unwrap_or_default())),
        },
        Some(_) => Check {
            id: "plugin",
            label: "Claude Code connection".into(),
            level: Level::Error,
            detail: "Not connected yet: use “Connect your agents”.".into(),
            advanced: None,
        },
        None => Check {
            id: "plugin",
            label: "Claude Code connection".into(),
            level: Level::Warn,
            detail: "Could not verify it.".into(),
            advanced: None,
        },
    });

    // Codex is optional: its checks only appear when it is installed.
    if let Some(path) = &env.codex_path {
        out.extend(codex_checks(path, env));
    }

    let pending = crate::spool::pending(&paths.spool);
    let oldest_age = pending.first().map(|(ns, _)| now - (*ns / 1_000_000) as i64);
    let writable = spool_writable(&paths.spool);
    out.push(Check {
        id: "spool",
        label: "Local capture".into(),
        level: match (writable, oldest_age) {
            (false, _) => Level::Error,
            (true, Some(age)) if age > 120_000 => Level::Warn,
            _ => Level::Ok,
        },
        detail: match (writable, oldest_age) {
            (false, _) => "The capture folder is not writable.".into(),
            (true, Some(age)) if age > 120_000 => format!("{} events waiting to be processed.", pending.len()),
            _ => "Working.".into(),
        },
        advanced: Some(format!("spool: {}; pending: {}", paths.spool.display(), pending.len())),
    });

    let last_event = store::meta(conn, "last_event_at")?.and_then(|v| v.parse::<i64>().ok());
    out.push(Check {
        id: "events",
        label: "Events received".into(),
        level: if last_event.is_some() { Level::Ok } else { Level::Warn },
        detail: match last_event {
            Some(at) => format!("Last event {}.", ago(now - at)),
            None => "No events yet. Start a conversation with Claude Code or Codex, or use the guided test.".into(),
        },
        advanced: store::meta(conn, "last_ingest_error")?.map(|e| format!("last ingest error: {e}")),
    });

    let usage = crate::recap::usage(conn, now)?;
    let failing: i64 = conn.query_row(
        "SELECT count(*) FROM conversations WHERE recap_status IN ('failed', 'retry') AND hidden = 0",
        [],
        |r| r.get(0),
    )?;
    let last_error: Option<String> = conn
        .query_row(
            "SELECT recap_error FROM conversations WHERE recap_error IS NOT NULL ORDER BY recap_due_at DESC LIMIT 1",
            [],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let (level, detail) = match settings.recap_provider {
        RecapProviderKind::Off => (Level::Warn, "AI summaries not set up: capture and states still work.".to_string()),
        RecapProviderKind::Api if !env.api_key_present => (Level::Error, "The API key is missing from the Keychain.".to_string()),
        RecapProviderKind::ClaudeCli if env.claude_path.is_none() => (Level::Error, "Claude Code is not available for summaries.".to_string()),
        _ if usage.calls_24h >= settings.recap_daily_limit as i64 || usage.cost_30d_usd >= settings.recap_monthly_budget_usd => {
            (Level::Warn, "Summary budget reached: summaries will resume when budget is available again.".to_string())
        }
        _ if failing > 0 => (Level::Warn, format!("{failing} summaries to update.")),
        _ => (Level::Ok, "Active.".to_string()),
    };
    out.push(Check {
        id: "recap",
        label: "Summaries".into(),
        level,
        detail,
        advanced: Some(format!(
            "provider: {:?}; calls 24h: {}/{}; cost 30d: {:.3}/{:.2} USD{}",
            settings.recap_provider,
            usage.calls_24h,
            settings.recap_daily_limit,
            usage.cost_30d_usd,
            settings.recap_monthly_budget_usd,
            last_error.map(|e| format!("; last error: {e}")).unwrap_or_default()
        )),
    });

    Ok(out)
}

fn codex_checks(path: &str, env: &Environment) -> Vec<Check> {
    let v = env.codex_version.clone().unwrap_or_else(|| "unknown".into());
    let tested = system::TESTED_CODEX_VERSIONS.contains(&v.as_str());
    let version = Check {
        id: "codex",
        label: "Codex CLI".into(),
        level: if tested { Level::Ok } else { Level::Warn },
        detail: if tested {
            format!("Version {v}, verified.")
        } else {
            format!("Version {v}: not yet verified with Flow Keeper. States may be incomplete.")
        },
        advanced: Some(format!(
            "executable: {path}; verified versions: {}",
            if system::TESTED_CODEX_VERSIONS.is_empty() { "none".into() } else { system::TESTED_CODEX_VERSIONS.join(", ") }
        )),
    };
    let label = || "Codex connection".to_string();
    let info = |s: &CodexPluginStatus| {
        format!(
            "plugin {} {}{}; hooks approved {}/{}{}; in {}",
            s.plugin.id.clone().unwrap_or_default(),
            s.plugin.version.clone().unwrap_or_default(),
            if s.plugin.managed { " (managed by the app)" } else { " (installed by hand)" },
            s.hooks_trusted,
            s.hooks_total,
            if s.hooks_disabled > 0 { format!(", {} disabled", s.hooks_disabled) } else { String::new() },
            s.plugin.install_path.clone().unwrap_or_default()
        )
    };
    let plugin = match &env.codex_plugin {
        Some(s) if s.plugin.installed && s.plugin.enabled && s.hooks_error.is_some() => Check {
            id: "codex_plugin",
            label: label(),
            level: Level::Warn,
            detail: "Could not verify the Codex hooks. Run the connection check again.".into(),
            advanced: Some(format!("{}; {}", info(s), s.hooks_error.as_deref().unwrap_or_default())),
        },
        Some(s) if s.plugin.installed && s.plugin.enabled && s.hooks_disabled > 0 => Check {
            id: "codex_plugin",
            label: label(),
            level: Level::Error,
            detail: "Some Flow Keeper hooks are disabled in Codex: re-enable them with /hooks.".into(),
            advanced: Some(info(s)),
        },
        Some(s) if s.plugin.installed && s.plugin.enabled && s.needs_review() => Check {
            id: "codex_plugin",
            label: label(),
            level: Level::Warn,
            detail: "Needs approval: open Codex, type /hooks and approve the Flow Keeper hooks.".into(),
            advanced: Some(info(s)),
        },
        Some(s) if s.plugin.installed && s.plugin.enabled && s.plugin.managed && !s.plugin.up_to_date => Check {
            id: "codex_plugin",
            label: label(),
            level: Level::Warn,
            detail: "Active, but needs an update: use “Connect your agents”.".into(),
            advanced: Some(info(s)),
        },
        Some(s) if s.plugin.installed && s.plugin.enabled => Check {
            id: "codex_plugin",
            label: label(),
            level: Level::Ok,
            detail: "Active.".into(),
            advanced: Some(info(s)),
        },
        Some(s) if s.plugin.installed => Check {
            id: "codex_plugin",
            label: label(),
            level: Level::Error,
            detail: "Disabled in Codex.".into(),
            advanced: Some(info(s)),
        },
        Some(_) => Check {
            id: "codex_plugin",
            label: label(),
            level: Level::Warn,
            detail: "Not connected yet: use “Connect your agents”.".into(),
            advanced: None,
        },
        None => Check { id: "codex_plugin", label: label(), level: Level::Warn, detail: "Could not verify it.".into(), advanced: None },
    };
    vec![version, plugin]
}

fn spool_writable(dir: &Path) -> bool {
    let probe = dir.join(".tmp").join(".doctor");
    let ok = std::fs::create_dir_all(dir.join(".tmp")).is_ok() && std::fs::write(&probe, b"x").is_ok();
    let _ = std::fs::remove_file(&probe);
    ok
}

pub fn ago(ms: i64) -> String {
    let min = ms / 60_000;
    match min {
        i64::MIN..=0 => "just now".into(),
        1 => "1 minute ago".into(),
        2..=59 => format!("{min} minutes ago"),
        60..=119 => "1 hour ago".into(),
        120..=1439 => format!("{} hours ago", min / 60),
        _ => format!("{} days ago", min / 1440),
    }
}

/// End-to-end guided test: a short `claude -p` session with the plugin
/// enabled. The event is recognized as a test and creates no conversations.
pub fn start_probe(claude: &Path, path_env: &str, workdir: &Path, nonce: &str) -> Result<()> {
    std::fs::create_dir_all(workdir)?;
    let mut cmd = Command::new(claude);
    cmd.arg("-p")
        .arg("Reply only: ok")
        .args(["--model", "haiku"])
        .args(["--tools", ""])
        .arg("--no-session-persistence")
        .arg("--strict-mcp-config")
        .current_dir(workdir)
        .env("PATH", path_env)
        .env("FLOW_KEEPER_PROBE", nonce);
    system::run_with_timeout(&mut cmd, Duration::from_secs(90));
    Ok(())
}

pub fn probe_received(conn: &Connection, nonce: &str) -> Result<bool> {
    Ok(store::meta(conn, &format!("probe:{nonce}"))?.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_is_ready_only_when_current_hooks_are_enabled_and_trusted() {
        let mut status = CodexPluginStatus {
            plugin: PluginStatus {
                installed: true, enabled: true, managed: true, up_to_date: true,
                id: Some(connect::local_plugin_id()), version: Some("0.1.1".into()), install_path: None,
            },
            hooks_total: 8, hooks_trusted: 8, hooks_disabled: 0, hooks_error: None,
        };
        let check = |s: &CodexPluginStatus| {
            let env = Environment { codex_plugin: Some(s.clone()), ..Default::default() };
            codex_checks("/bin/codex", &env).pop().unwrap()
        };
        assert_eq!(check(&status).level, Level::Ok);
        status.hooks_trusted = 7;
        assert!(status.needs_review());
        assert_eq!(check(&status).level, Level::Warn);
        status.hooks_disabled = 1;
        assert_eq!(check(&status).level, Level::Error);
        status.hooks_error = Some("timeout".into());
        let error = check(&status);
        assert_eq!(error.level, Level::Warn);
        assert!(error.detail.contains("Could not verify"));
    }
}
