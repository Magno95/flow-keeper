//! User preferences, saved in the local store.

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::store;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecapProviderKind {
    /// `claude -p` with the subscription already set up in Claude Code.
    ClaudeCli,
    /// Anthropic API with the key in the keychain.
    Api,
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub recap_provider: RecapProviderKind,
    pub recap_cli_model: String,
    pub recap_api_model: String,
    pub recap_daily_limit: u32,
    pub recap_monthly_budget_usd: f64,
    pub notify_attention: bool,
    pub notify_finished: bool,
    pub do_not_disturb: bool,
    pub track_print_sessions: bool,
    pub shortcut: String,
    pub companion_visible: bool,
    pub launch_at_login: bool,
    pub onboarding_done: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            recap_provider: RecapProviderKind::ClaudeCli,
            recap_cli_model: "haiku".into(),
            recap_api_model: "claude-haiku-4-5".into(),
            recap_daily_limit: 300,
            recap_monthly_budget_usd: 10.0,
            notify_attention: true,
            notify_finished: true,
            do_not_disturb: false,
            track_print_sessions: false,
            shortcut: "Alt+Super+J".into(),
            companion_visible: true,
            launch_at_login: true,
            onboarding_done: false,
        }
    }
}

const KEY: &str = "settings";

pub fn load(conn: &Connection) -> Result<Settings> {
    Ok(store::get_setting(conn, KEY)?
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default())
}

pub fn save(conn: &Connection, settings: &Settings) -> Result<()> {
    store::set_setting(conn, KEY, &serde_json::to_string(settings)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_defaults_for_missing_fields() {
        let conn = store::open_in_memory().unwrap();
        assert_eq!(load(&conn).unwrap(), Settings::default());
        store::set_setting(&conn, KEY, r#"{"notifyFinished":false}"#).unwrap();
        let s = load(&conn).unwrap();
        assert!(!s.notify_finished);
        assert_eq!(s.recap_provider, RecapProviderKind::ClaudeCli);
        save(&conn, &s).unwrap();
        assert_eq!(load(&conn).unwrap(), s);
    }
}
