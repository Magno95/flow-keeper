//! Connection to Claude Code and Codex CLI, managed by the app.
//!
//! The plugin with the hooks is embedded in the app. On connect it is written
//! to a local marketplace in the data folder and registered with the CLI
//! (`claude plugin marketplace add <folder>`, `codex plugin marketplace add
//! <folder>`): no GitHub access, no changes to the user's `settings.json` or
//! hooks, and the version always matches the app.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::legacy;
use crate::system::run_with_timeout;

pub const PLUGIN_NAME: &str = "flow-keeper";
/// Local marketplace created by the app.
pub const LOCAL_MARKETPLACE: &str = "flow-keeper-app";
/// Marketplace published in the GitHub repository (manual install).
pub const GITHUB_MARKETPLACE: &str = "flow-keeper";

const PLUGIN_JSON: &str = include_str!("../../plugin/.claude-plugin/plugin.json");
const HOOKS_JSON: &str = include_str!("../../plugin/hooks/hooks.json");
const CAPTURE_SH: &str = include_str!("../../plugin/hooks/capture.sh");
const CODEX_PLUGIN_JSON: &str = include_str!("../../plugin/.codex-plugin/plugin.json");
const CODEX_HOOKS_JSON: &str = include_str!("../../plugin/hooks/codex.json");

pub fn local_plugin_id() -> String {
    format!("{PLUGIN_NAME}@{LOCAL_MARKETPLACE}")
}

fn plugin_version() -> String {
    serde_json::from_str::<Value>(PLUGIN_JSON)
        .ok()
        .and_then(|v| v.get("version").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| "0.0.0".into())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginStatus {
    pub installed: bool,
    pub enabled: bool,
    /// Installed by the app (local marketplace), not by hand from GitHub.
    pub managed: bool,
    /// The installed files match the ones embedded in this version of the app.
    pub up_to_date: bool,
    pub id: Option<String>,
    pub version: Option<String>,
    pub install_path: Option<String>,
}

impl PluginStatus {
    fn absent() -> Self {
        PluginStatus { installed: false, enabled: false, managed: false, up_to_date: false, id: None, version: None, install_path: None }
    }
}

struct Cli<'a> {
    bin: &'a Path,
    path_env: &'a str,
    /// Isolated configuration (`CLAUDE_CONFIG_DIR` or `CODEX_HOME`), used in tests.
    config_dir: Option<&'a Path>,
    name: &'static str,
    config_env: &'static str,
}

impl<'a> Cli<'a> {
    fn claude(bin: &'a Path, path_env: &'a str, config_dir: Option<&'a Path>) -> Self {
        Cli { bin, path_env, config_dir, name: "claude", config_env: "CLAUDE_CONFIG_DIR" }
    }

    fn codex(bin: &'a Path, path_env: &'a str, config_dir: Option<&'a Path>) -> Self {
        Cli { bin, path_env, config_dir, name: "codex", config_env: "CODEX_HOME" }
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(self.bin);
        cmd.args(args).env("PATH", self.path_env).env("FLOW_KEEPER_INTERNAL", "1");
        if let Some(dir) = self.config_dir {
            cmd.env(self.config_env, dir);
        }
        cmd
    }

    fn run(&self, args: &[&str]) -> Result<()> {
        let out = self
            .command(args)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| anyhow!("could not start {}: {e}", self.name))?;
        if out.status.success() {
            return Ok(());
        }
        let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        Err(anyhow!("{} {}: {}", self.name, args.join(" "), crate::engine::truncate_chars(text.trim(), 400)))
    }

    fn json(&self, args: &[&str]) -> Option<Value> {
        let out = run_with_timeout(&mut self.command(args), Duration::from_secs(30))?;
        serde_json::from_slice(&out).ok()
    }
}

/// Writes (or rewrites) the local marketplace with the embedded plugin.
pub fn write_local_marketplace(dir: &Path) -> Result<PathBuf> {
    let plugin = dir.join("plugin");
    fs::create_dir_all(plugin.join(".claude-plugin"))?;
    fs::create_dir_all(plugin.join("hooks"))?;
    fs::create_dir_all(dir.join(".claude-plugin"))?;
    fs::write(plugin.join(".claude-plugin/plugin.json"), PLUGIN_JSON)?;
    fs::write(plugin.join("hooks/hooks.json"), HOOKS_JSON)?;
    let capture = plugin.join("hooks/capture.sh");
    fs::write(&capture, CAPTURE_SH)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&capture, fs::Permissions::from_mode(0o755))?;
    }
    let manifest = json!({
        "name": LOCAL_MARKETPLACE,
        "owner": { "name": "Flow Keeper" },
        "description": "Plugin installed and updated by the Flow Keeper app.",
        "plugins": [{
            "name": PLUGIN_NAME,
            "displayName": "Flow Keeper",
            "description": "Sends conversation events to Flow Keeper, locally.",
            "source": "./plugin",
            "version": plugin_version()
        }]
    });
    fs::write(dir.join(".claude-plugin/marketplace.json"), serde_json::to_string_pretty(&manifest)?)?;
    Ok(dir.to_path_buf())
}

fn files_match(install_path: &Path) -> bool {
    let same = |rel: &str, expected: &str| fs::read_to_string(install_path.join(rel)).is_ok_and(|s| s == expected);
    same(".claude-plugin/plugin.json", PLUGIN_JSON) && same("hooks/hooks.json", HOOKS_JSON) && same("hooks/capture.sh", CAPTURE_SH)
}

fn read_status(cli: &Cli) -> Option<PluginStatus> {
    let list = cli.json(&["plugin", "list", "--json"])?;
    let entries = list.as_array()?;
    let id_of = |p: &Value| p.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let local = local_plugin_id();
    // If both are present, the one managed by the app wins.
    let entry = entries
        .iter()
        .find(|p| id_of(p) == local)
        .or_else(|| entries.iter().find(|p| id_of(p).starts_with(&format!("{PLUGIN_NAME}@")) ));
    Some(match entry {
        None => PluginStatus::absent(),
        Some(p) => {
            let install_path = p.get("installPath").and_then(Value::as_str).map(str::to_string);
            PluginStatus {
                installed: true,
                enabled: p.get("enabled").and_then(Value::as_bool).unwrap_or(false),
                managed: id_of(p) == local,
                up_to_date: install_path.as_deref().is_some_and(|ip| files_match(Path::new(ip))),
                id: Some(id_of(p)),
                version: p.get("version").and_then(Value::as_str).map(str::to_string),
                install_path,
            }
        }
    })
}

pub fn status(claude: &Path, path_env: &str) -> Option<PluginStatus> {
    read_status(&Cli::claude(claude, path_env, None))
}

/// Connects Claude Code: local marketplace, plugin install or update.
pub fn connect(claude: &Path, path_env: &str, dir: &Path) -> Result<PluginStatus> {
    connect_with(&Cli::claude(claude, path_env, None), dir)
}

fn marketplace_known(cli: &Cli, name: &str) -> bool {
    cli.json(&["plugin", "marketplace", "list", "--json"])
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .any(|m| m.get("name").and_then(Value::as_str) == Some(name))
}

fn connect_with(cli: &Cli, dir: &Path) -> Result<PluginStatus> {
    let dir = write_local_marketplace(dir)?;
    let dir_str = dir.to_string_lossy().to_string();
    let id = local_plugin_id();

    if marketplace_known(cli, LOCAL_MARKETPLACE) {
        cli.run(&["plugin", "marketplace", "update", LOCAL_MARKETPLACE])?;
    } else {
        cli.run(&["plugin", "marketplace", "add", &dir_str])?;
    }

    let current = read_status(cli).filter(|s| s.managed);
    match current {
        None => cli.run(&["plugin", "install", &id])?,
        Some(s) if !s.up_to_date => {
            cli.run(&["plugin", "update", &id])?;
            // If the declared version has not changed, the update may not
            // copy the files again: reinstall.
            if !read_status(cli).is_some_and(|s| s.managed && s.up_to_date) {
                cli.run(&["plugin", "uninstall", &id])?;
                cli.run(&["plugin", "install", &id])?;
            }
        }
        Some(_) => {}
    }
    read_status(cli).ok_or_else(|| anyhow!("could not verify the connection"))
}

/// Keeps the plugin already connected by the app in sync (e.g. after an update).
/// It never connects on its own initiative: the first time is the user's choice.
pub fn sync_if_connected(claude: &Path, path_env: &str, dir: &Path) -> Option<PluginStatus> {
    let cli = Cli::claude(claude, path_env, None);
    let s = read_status(&cli)?;
    if s.managed && !s.up_to_date {
        return connect_with(&cli, dir).ok();
    }
    Some(s)
}

/// Disconnects: removes the plugin and the local marketplace added by the app.
pub fn disconnect(claude: &Path, path_env: &str) -> Result<()> {
    disconnect_with(&Cli::claude(claude, path_env, None))
}

fn disconnect_with(cli: &Cli) -> Result<()> {
    if read_status(cli).is_some_and(|s| s.managed) {
        cli.run(&["plugin", "uninstall", &local_plugin_id()])?;
    }
    if marketplace_known(cli, LOCAL_MARKETPLACE) {
        cli.run(&["plugin", "marketplace", "remove", LOCAL_MARKETPLACE])?;
    }
    Ok(())
}

/// Replaces the AI Tracker plugin (connected by the app or installed by hand
/// from GitHub) with the app-managed Flow Keeper one, reconnecting only if it
/// was enabled. `Ok(false)` if there was none.
pub fn migrate_legacy(claude: &Path, path_env: &str, dir: &Path) -> Result<bool> {
    migrate_legacy_with(&Cli::claude(claude, path_env, None), dir)
}

fn migrate_legacy_with(cli: &Cli, dir: &Path) -> Result<bool> {
    let Some(list) = cli.json(&["plugin", "list", "--json"]) else { return Ok(false) };
    let old: Vec<(String, bool)> = list
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|p| {
            let id = p.get("id").and_then(Value::as_str).filter(|id| legacy::is_plugin_id(id))?;
            Some((id.to_string(), p.get("enabled").and_then(Value::as_bool).unwrap_or(false)))
        })
        .collect();
    if old.is_empty() {
        return Ok(false);
    }
    for (id, _) in &old {
        cli.run(&["plugin", "uninstall", id])?;
    }
    for name in [legacy::LOCAL_MARKETPLACE, legacy::GITHUB_MARKETPLACE] {
        if marketplace_known(cli, name) {
            cli.run(&["plugin", "marketplace", "remove", name])?;
        }
    }
    if old.iter().any(|(_, enabled)| *enabled) {
        connect_with(cli, dir)?;
    }
    Ok(true)
}

// ---- Codex CLI ----------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexPluginStatus {
    #[serde(flatten)]
    pub plugin: PluginStatus,
    /// Hooks declared by the plugin and how many have been approved in Codex (`/hooks`).
    pub hooks_total: usize,
    pub hooks_trusted: usize,
    pub hooks_disabled: usize,
    /// Check unavailable: not to be mistaken for hooks already approved.
    pub hooks_error: Option<String>,
}

impl CodexPluginStatus {
    /// Codex runs a plugin's hooks only after the user approves them.
    pub fn needs_review(&self) -> bool {
        self.plugin.installed && (self.hooks_trusted < self.hooks_total || self.hooks_disabled > 0 || self.hooks_error.is_some())
    }
}

fn codex_hooks_total() -> usize {
    serde_json::from_str::<Value>(CODEX_HOOKS_JSON)
        .ok()
        .and_then(|v| v.get("hooks").and_then(Value::as_object).cloned())
        .map(|events| {
            events
                .values()
                .filter_map(Value::as_array)
                .flatten()
                .filter_map(|g| g.get("hooks").and_then(Value::as_array))
                .map(Vec::len)
                .sum()
        })
        .unwrap_or(0)
}

/// Writes (or rewrites) the local Codex marketplace with the embedded plugin.
pub fn write_codex_marketplace(dir: &Path) -> Result<PathBuf> {
    let plugin = dir.join("plugins").join(PLUGIN_NAME);
    fs::create_dir_all(plugin.join(".codex-plugin"))?;
    fs::create_dir_all(plugin.join("hooks"))?;
    fs::create_dir_all(dir.join(".agents/plugins"))?;
    fs::write(plugin.join(".codex-plugin/plugin.json"), CODEX_PLUGIN_JSON)?;
    fs::write(plugin.join("hooks/codex.json"), CODEX_HOOKS_JSON)?;
    let capture = plugin.join("hooks/capture.sh");
    fs::write(&capture, CAPTURE_SH)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&capture, fs::Permissions::from_mode(0o755))?;
    }
    let manifest = json!({
        "name": LOCAL_MARKETPLACE,
        "interface": { "displayName": "Flow Keeper" },
        "plugins": [{
            "name": PLUGIN_NAME,
            "source": { "source": "local", "path": format!("./plugins/{PLUGIN_NAME}") },
            "policy": { "installation": "AVAILABLE", "authentication": "ON_INSTALL" },
            "category": "Developer Tools"
        }]
    });
    fs::write(dir.join(".agents/plugins/marketplace.json"), serde_json::to_string_pretty(&manifest)?)?;
    Ok(dir.to_path_buf())
}

fn codex_files_match(install_path: &Path) -> bool {
    let same = |rel: &str, expected: &str| fs::read_to_string(install_path.join(rel)).is_ok_and(|s| s == expected);
    same(".codex-plugin/plugin.json", CODEX_PLUGIN_JSON) && same("hooks/codex.json", CODEX_HOOKS_JSON) && same("hooks/capture.sh", CAPTURE_SH)
}

fn codex_home_of(cli: &Cli) -> PathBuf {
    cli.config_dir.map(Path::to_path_buf).unwrap_or_else(crate::codex::home)
}

fn codex_read_status(cli: &Cli) -> Option<CodexPluginStatus> {
    let list = cli.json(&["plugin", "list", "--json"])?;
    let entries = list.get("installed")?.as_array()?;
    let id_of = |p: &Value| p.get("pluginId").and_then(Value::as_str).unwrap_or("").to_string();
    let local = local_plugin_id();
    let entry = entries
        .iter()
        .find(|p| id_of(p) == local)
        .or_else(|| entries.iter().find(|p| id_of(p).starts_with(&format!("{PLUGIN_NAME}@"))))
        .filter(|p| p.get("installed").and_then(Value::as_bool) != Some(false));
    let home = codex_home_of(cli);
    let hooks_total = codex_hooks_total();
    Some(match entry {
        None => CodexPluginStatus { plugin: PluginStatus::absent(), hooks_total, hooks_trusted: 0, hooks_disabled: 0, hooks_error: None },
        Some(p) => {
            let id = id_of(p);
            let version = p.get("version").and_then(Value::as_str).map(str::to_string);
            let marketplace = p.get("marketplaceName").and_then(Value::as_str).unwrap_or(LOCAL_MARKETPLACE);
            // Codex copies the plugin to `<home>/plugins/cache/<marketplace>/<plugin>/<version>`.
            let install_path = version
                .as_deref()
                .map(|v| home.join("plugins/cache").join(marketplace).join(PLUGIN_NAME).join(v));
            let enabled = p.get("enabled").and_then(Value::as_bool).unwrap_or(false);
            let (trust, hooks_error) = if enabled {
                match crate::codex::hook_trust(&mut cli.command(&["app-server"]), &crate::paths::home(), &id) {
                    Ok(trust) => (trust, None),
                    Err(error) => (Default::default(), Some(error.to_string())),
                }
            } else {
                (Default::default(), None)
            };
            CodexPluginStatus {
                plugin: PluginStatus {
                    installed: true,
                    enabled,
                    managed: id == local,
                    up_to_date: install_path.as_deref().is_some_and(codex_files_match),
                    id: Some(id),
                    version,
                    install_path: install_path.map(|p| p.to_string_lossy().to_string()),
                },
                hooks_total: hooks_total.max(trust.total),
                hooks_trusted: trust.trusted,
                hooks_disabled: trust.disabled,
                hooks_error,
            }
        }
    })
}

fn codex_marketplace_known(cli: &Cli) -> bool {
    codex_has_marketplace(cli, LOCAL_MARKETPLACE)
}

fn codex_has_marketplace(cli: &Cli, name: &str) -> bool {
    cli.json(&["plugin", "marketplace", "list", "--json"])
        .and_then(|v| v.get("marketplaces").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .any(|m| m.get("name").and_then(Value::as_str) == Some(name))
}

pub fn codex_status(codex: &Path, path_env: &str) -> Option<CodexPluginStatus> {
    codex_read_status(&Cli::codex(codex, path_env, None))
}

/// Connects Codex: local marketplace, plugin install or update.
/// The hooks must then be approved once in Codex with `/hooks`: the app does
/// not write the trust on the user's behalf.
pub fn codex_connect(codex: &Path, path_env: &str, dir: &Path) -> Result<CodexPluginStatus> {
    codex_connect_with(&Cli::codex(codex, path_env, None), dir)
}

fn codex_connect_with(cli: &Cli, dir: &Path) -> Result<CodexPluginStatus> {
    let dir = write_codex_marketplace(dir)?;
    if !codex_marketplace_known(cli) {
        cli.run(&["plugin", "marketplace", "add", &dir.to_string_lossy()])?;
    }
    let current = codex_read_status(cli).filter(|s| s.plugin.managed);
    // `plugin add` copies the files again even if the plugin is already installed.
    if !current.is_some_and(|s| s.plugin.up_to_date && s.plugin.enabled) {
        cli.run(&["plugin", "add", &local_plugin_id()])?;
    }
    codex_read_status(cli).ok_or_else(|| anyhow!("could not verify the connection"))
}

pub fn codex_sync_if_connected(codex: &Path, path_env: &str, dir: &Path) -> Option<CodexPluginStatus> {
    let cli = Cli::codex(codex, path_env, None);
    codex_sync_with(&cli, dir)
}

fn codex_sync_with(cli: &Cli, dir: &Path) -> Option<CodexPluginStatus> {
    let s = codex_read_status(cli)?;
    if s.plugin.managed && s.plugin.enabled && !s.plugin.up_to_date {
        return codex_connect_with(cli, dir).ok();
    }
    Some(s)
}

/// Like `migrate_legacy`, for Codex. The new plugin's hooks must be approved
/// again with `/hooks`: trust is tied to the plugin id.
pub fn codex_migrate_legacy(codex: &Path, path_env: &str, dir: &Path) -> Result<bool> {
    codex_migrate_legacy_with(&Cli::codex(codex, path_env, None), dir)
}

fn codex_migrate_legacy_with(cli: &Cli, dir: &Path) -> Result<bool> {
    let Some(list) = cli.json(&["plugin", "list", "--json"]) else { return Ok(false) };
    let old: Vec<(String, bool)> = list
        .get("installed")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|p| p.get("installed").and_then(Value::as_bool) != Some(false))
        .filter_map(|p| {
            let id = p.get("pluginId").and_then(Value::as_str).filter(|id| legacy::is_plugin_id(id))?;
            Some((id.to_string(), p.get("enabled").and_then(Value::as_bool).unwrap_or(false)))
        })
        .collect();
    if old.is_empty() {
        return Ok(false);
    }
    for (id, _) in &old {
        cli.run(&["plugin", "remove", id])?;
    }
    for name in [legacy::LOCAL_MARKETPLACE, legacy::GITHUB_MARKETPLACE] {
        if codex_has_marketplace(cli, name) {
            cli.run(&["plugin", "marketplace", "remove", name])?;
        }
    }
    if old.iter().any(|(_, enabled)| *enabled) {
        codex_connect_with(cli, dir)?;
    }
    Ok(true)
}

/// Disconnects: removes the plugin and the local marketplace added by the app.
pub fn codex_disconnect(codex: &Path, path_env: &str) -> Result<()> {
    codex_disconnect_with(&Cli::codex(codex, path_env, None))
}

fn codex_disconnect_with(cli: &Cli) -> Result<()> {
    if codex_read_status(cli).is_some_and(|s| s.plugin.managed) {
        cli.run(&["plugin", "remove", &local_plugin_id()])?;
    }
    if codex_marketplace_known(cli) {
        cli.run(&["plugin", "marketplace", "remove", LOCAL_MARKETPLACE])?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_marketplace_contains_executable_hook() {
        let dir = tempfile::tempdir().unwrap();
        write_local_marketplace(dir.path()).unwrap();
        let m: Value = serde_json::from_str(&fs::read_to_string(dir.path().join(".claude-plugin/marketplace.json")).unwrap()).unwrap();
        assert_eq!(m["name"], LOCAL_MARKETPLACE);
        assert_eq!(m["plugins"][0]["version"], plugin_version());
        let capture = dir.path().join("plugin/hooks/capture.sh");
        assert!(files_match(&dir.path().join("plugin")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(capture).unwrap().permissions().mode() & 0o777, 0o755);
        }
    }

    #[test]
    fn codex_marketplace_contains_hooks_for_every_event() {
        let dir = tempfile::tempdir().unwrap();
        write_codex_marketplace(dir.path()).unwrap();
        let m: Value = serde_json::from_str(&fs::read_to_string(dir.path().join(".agents/plugins/marketplace.json")).unwrap()).unwrap();
        assert_eq!(m["name"], LOCAL_MARKETPLACE);
        let plugin = dir.path().join("plugins").join(PLUGIN_NAME);
        assert!(codex_files_match(&plugin));
        let manifest: Value = serde_json::from_str(CODEX_PLUGIN_JSON).unwrap();
        assert_eq!(manifest["version"], plugin_version(), "manifest versions match");
        assert_eq!(manifest["hooks"], "./hooks/codex.json");
        assert_eq!(codex_hooks_total(), 8);
        // Every command passes the event and the tool to capture.sh.
        let hooks: Value = serde_json::from_str(CODEX_HOOKS_JSON).unwrap();
        for (event, groups) in hooks["hooks"].as_object().unwrap() {
            let cmd = groups[0]["hooks"][0]["command"].as_str().unwrap();
            assert_eq!(cmd, format!("\"${{PLUGIN_ROOT}}/hooks/capture.sh\" {event} codex"));
        }
    }

    /// Local marketplace as AI Tracker (0.4) wrote it, for the migration tests.
    fn write_legacy_marketplaces(claude_dir: &Path, codex_dir: &Path) {
        let rename = |path: &Path| {
            let s = fs::read_to_string(path).unwrap().replace(LOCAL_MARKETPLACE, legacy::LOCAL_MARKETPLACE).replace(PLUGIN_NAME, legacy::PLUGIN_NAME);
            fs::write(path, s).unwrap();
        };
        write_local_marketplace(claude_dir).unwrap();
        rename(&claude_dir.join(".claude-plugin/marketplace.json"));
        rename(&claude_dir.join("plugin/.claude-plugin/plugin.json"));
        write_codex_marketplace(codex_dir).unwrap();
        fs::rename(codex_dir.join("plugins").join(PLUGIN_NAME), codex_dir.join("plugins").join(legacy::PLUGIN_NAME)).unwrap();
        rename(&codex_dir.join(".agents/plugins/marketplace.json"));
        rename(&codex_dir.join("plugins").join(legacy::PLUGIN_NAME).join(".codex-plugin/plugin.json"));
    }

    /// Replacing the AI Tracker plugin with the real CLIs, in isolated
    /// configurations: `cargo test -p tracker-core -- --ignored real_migration`
    #[test]
    #[ignore]
    fn real_migration_replaces_legacy_plugins() {
        let path_env = crate::system::search_path(std::env::var("PATH").ok().as_deref());
        let data = tempfile::tempdir().unwrap();
        let (old_claude, old_codex) = (data.path().join("old-claude"), data.path().join("old-codex"));
        write_legacy_marketplaces(&old_claude, &old_codex);

        if let Some(claude) = crate::system::which("claude", &path_env) {
            let config = tempfile::tempdir().unwrap();
            let cli = Cli::claude(&claude, &path_env, Some(config.path()));
            // Also the plugin installed by hand from the old repository's marketplace.
            let old_github = data.path().join("old-github");
            write_legacy_marketplaces(&old_github, &data.path().join("unused"));
            let manifest = old_github.join(".claude-plugin/marketplace.json");
            fs::write(&manifest, fs::read_to_string(&manifest).unwrap().replace(legacy::LOCAL_MARKETPLACE, legacy::GITHUB_MARKETPLACE)).unwrap();
            cli.run(&["plugin", "marketplace", "add", &old_claude.to_string_lossy()]).unwrap();
            cli.run(&["plugin", "install", &legacy::local_plugin_id()]).unwrap();
            cli.run(&["plugin", "marketplace", "add", &old_github.to_string_lossy()]).unwrap();
            cli.run(&["plugin", "install", &format!("{}@{}", legacy::PLUGIN_NAME, legacy::GITHUB_MARKETPLACE)]).unwrap();
            assert!(!read_status(&cli).unwrap().installed, "the old plugin does not count as connected");

            assert!(migrate_legacy_with(&cli, &data.path().join("claude-plugin")).unwrap());
            let s = read_status(&cli).unwrap();
            assert!(s.installed && s.enabled && s.managed && s.up_to_date, "{s:?}");
            let list = cli.json(&["plugin", "list", "--json"]).unwrap();
            assert!(!list.to_string().contains(&format!("{}@", legacy::PLUGIN_NAME)), "{list}");
            assert!(!marketplace_known(&cli, legacy::LOCAL_MARKETPLACE));
            assert!(!marketplace_known(&cli, legacy::GITHUB_MARKETPLACE));
            assert!(!migrate_legacy_with(&cli, &data.path().join("claude-plugin")).unwrap(), "second time: nothing to do");
        }

        if let Some(codex) = crate::system::which("codex", &path_env) {
            let home = tempfile::tempdir().unwrap();
            let cli = Cli::codex(&codex, &path_env, Some(home.path()));
            cli.run(&["plugin", "marketplace", "add", &old_codex.to_string_lossy()]).unwrap();
            cli.run(&["plugin", "add", &legacy::local_plugin_id()]).unwrap();

            assert!(codex_migrate_legacy_with(&cli, &data.path().join("codex-plugin")).unwrap());
            let s = codex_read_status(&cli).unwrap();
            assert!(s.plugin.installed && s.plugin.enabled && s.plugin.managed && s.plugin.up_to_date, "{s:?}");
            assert!(s.needs_review(), "the new hooks must be approved");
            assert!(!codex_has_marketplace(&cli, legacy::LOCAL_MARKETPLACE));
            assert!(!codex_migrate_legacy_with(&cli, &data.path().join("codex-plugin")).unwrap());
        }
    }

    /// Full cycle with the real Codex CLI, in an isolated `CODEX_HOME`
    /// (no credentials needed): `cargo test -p tracker-core -- --ignored real_codex`
    #[test]
    #[ignore]
    fn real_codex_cli_connect_update_disconnect() {
        let path_env = crate::system::search_path(std::env::var("PATH").ok().as_deref());
        let codex = crate::system::which("codex", &path_env).expect("codex in PATH");
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let cli = Cli::codex(&codex, &path_env, Some(home.path()));
        let dir = data.path().join("codex-plugin");

        let s = codex_connect_with(&cli, &dir).unwrap();
        assert!(s.plugin.installed && s.plugin.enabled && s.plugin.managed && s.plugin.up_to_date, "{s:?}");
        assert_eq!(s.hooks_error, None, "{s:?}");
        assert_eq!(s.hooks_trusted, 0);
        assert!(s.needs_review(), "the hooks must be approved in Codex");

        let ip = PathBuf::from(s.plugin.install_path.unwrap());
        let config_path = home.path().join("config.toml");
        let config = fs::read_to_string(&config_path).unwrap();
        assert!(config.contains("enabled = true"));
        let disabled = config.replace("enabled = true", "enabled = false");
        fs::write(&config_path, &disabled).unwrap();
        assert!(!codex_read_status(&cli).unwrap().plugin.enabled);
        // The automatic update must not re-enable a disabled plugin.
        fs::write(ip.join("hooks/capture.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        assert!(!codex_sync_with(&cli, &dir).unwrap().plugin.enabled);
        let s = codex_connect_with(&cli, &dir).unwrap();
        assert!(s.plugin.enabled && s.plugin.up_to_date, "{s:?}");
        // Even when up to date, the Connect button must re-enable it.
        fs::write(&config_path, &disabled).unwrap();
        assert!(codex_connect_with(&cli, &dir).unwrap().plugin.enabled);

        // Trust the current definition, then modify it: Codex must report
        // modified, the old trusted_hash values are not enough.
        let listing = crate::codex::list_hooks(&mut cli.command(&["app-server"]), &crate::paths::home(), Duration::from_secs(10)).unwrap();
        let mut trusted = fs::read_to_string(&config_path).unwrap();
        let mut first_key = None;
        for hook in listing["data"][0]["hooks"].as_array().unwrap() {
            if hook["pluginId"] == local_plugin_id() {
                let key = hook["key"].as_str().unwrap();
                first_key.get_or_insert_with(|| key.to_string());
                trusted.push_str(&format!("\n[hooks.state.{}]\ntrusted_hash = {}\n", json!(key), hook["currentHash"]));
            }
        }
        fs::write(&config_path, &trusted).unwrap();
        let s = codex_read_status(&cli).unwrap();
        assert_eq!(s.hooks_error, None, "{s:?}");
        assert_eq!(s.hooks_trusted, 8, "{s:?}");
        assert!(!s.needs_review());
        let mut changed: Value = serde_json::from_str(CODEX_HOOKS_JSON).unwrap();
        changed["hooks"]["Stop"][0]["hooks"][0]["timeout"] = json!(4);
        fs::write(ip.join("hooks/codex.json"), serde_json::to_string(&changed).unwrap()).unwrap();
        let s = codex_read_status(&cli).unwrap();
        assert_eq!(s.hooks_trusted, 7, "{s:?}");
        assert!(s.needs_review());
        fs::write(ip.join("hooks/codex.json"), CODEX_HOOKS_JSON).unwrap();
        let key = format!("[hooks.state.{}]", json!(first_key.unwrap()));
        fs::write(&config_path, trusted.replace(&key, &format!("{key}\nenabled = false"))).unwrap();
        let s = codex_read_status(&cli).unwrap();
        assert_eq!(s.hooks_disabled, 1, "{s:?}");
        assert_eq!(s.hooks_trusted, 7);
        assert!(s.needs_review());

        fs::write(ip.join("hooks/capture.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        assert!(!codex_read_status(&cli).unwrap().plugin.up_to_date);
        let s = codex_connect_with(&cli, &dir).unwrap();
        assert!(s.plugin.up_to_date, "{s:?}");

        codex_connect_with(&cli, &dir).unwrap();
        codex_disconnect_with(&cli).unwrap();
        assert!(!codex_read_status(&cli).unwrap().plugin.installed);
        assert!(!codex_marketplace_known(&cli));
    }

    /// Full cycle with the real CLI, in an isolated Claude configuration:
    /// `cargo test -p tracker-core -- --ignored real_cli`
    #[test]
    #[ignore]
    fn real_cli_connect_update_disconnect() {
        let path_env = crate::system::search_path(std::env::var("PATH").ok().as_deref());
        let claude = crate::system::which("claude", &path_env).expect("claude in PATH");
        let config = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let cli = Cli::claude(&claude, &path_env, Some(config.path()));

        let s = connect_with(&cli, &data.path().join("claude-plugin")).unwrap();
        assert!(s.installed && s.enabled && s.managed && s.up_to_date, "{s:?}");

        // Installed files altered (e.g. app updated): connecting brings them back in sync.
        let ip = PathBuf::from(s.install_path.unwrap());
        fs::write(ip.join("hooks/capture.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        assert!(!read_status(&cli).unwrap().up_to_date);
        let s = connect_with(&cli, &data.path().join("claude-plugin")).unwrap();
        assert!(s.up_to_date, "{s:?}");

        // Connecting again duplicates nothing.
        connect_with(&cli, &data.path().join("claude-plugin")).unwrap();
        let list = cli.json(&["plugin", "list", "--json"]).unwrap();
        let ours = list.as_array().unwrap().iter().filter(|p| p["id"] == local_plugin_id()).count();
        assert_eq!(ours, 1);

        disconnect_with(&cli).unwrap();
        assert!(!read_status(&cli).unwrap().installed);
    }
}
