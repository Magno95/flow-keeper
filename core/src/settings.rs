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
    /// Sends the use counters (`activity.rs`) to `activity_endpoint`.
    pub activity_sync: bool,
    pub activity_endpoint: String,
    /// Sent as `Authorization: Bearer`; empty for none.
    pub activity_token: String,
    /// The user changed the three values above: from then on the build's
    /// server no longer applies.
    pub activity_custom: bool,
}

/// Server set at build time with `FLOK_ACTIVITY_ENDPOINT` (and the token with
/// `FLOK_ACTIVITY_TOKEN`). Until the user changes the activity settings, every
/// load takes it from the running build: a later build with a different (or a
/// first) server reaches everyone who left the default. Without it nothing is
/// sent until the user sets a server.
const BUILD_ACTIVITY_ENDPOINT: &str = match option_env!("FLOK_ACTIVITY_ENDPOINT") {
    Some(v) => v,
    None => "",
};
const BUILD_ACTIVITY_TOKEN: &str = match option_env!("FLOK_ACTIVITY_TOKEN") {
    Some(v) => v,
    None => "",
};

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
            activity_sync: false,
            activity_endpoint: String::new(),
            activity_token: String::new(),
            activity_custom: false,
        }
        .with_build_activity(BUILD_ACTIVITY_ENDPOINT, BUILD_ACTIVITY_TOKEN)
    }
}

const KEY: &str = "settings";
/// Copy of the last record that could not be read, kept until it is replaced.
const INVALID_KEY: &str = "settings.invalid";
/// Marker read by the diagnostics: the saved preferences were not usable.
pub const INVALID_META: &str = "settings_invalid";

pub const MAX_DAILY_LIMIT: u32 = 10_000;
pub const MAX_MONTHLY_BUDGET_USD: f64 = 10_000.0;

/// `http(s)://host…` without spaces: the rest is up to the server.
fn valid_endpoint(url: &str) -> bool {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"));
    url.len() <= 2048
        && rest.is_some_and(|r| !r.is_empty() && !r.starts_with('/'))
        && url.chars().all(|c| c.is_ascii_graphic())
}

impl Settings {
    /// Checks the values the interface cannot guarantee on its own.
    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.recap_daily_limit > MAX_DAILY_LIMIT {
            return Err(format!("Calls per day must be at most {MAX_DAILY_LIMIT}."));
        }
        if !self.recap_monthly_budget_usd.is_finite()
            || !(0.0..=MAX_MONTHLY_BUDGET_USD).contains(&self.recap_monthly_budget_usd)
        {
            return Err(format!("The monthly budget must be between 0 and {MAX_MONTHLY_BUDGET_USD}."));
        }
        for (label, model) in [("CLI", &self.recap_cli_model), ("API", &self.recap_api_model)] {
            let ok = !model.is_empty()
                && model.len() <= 100
                && !model.starts_with('-')
                && model.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '[' | ']' | '@'));
            if !ok {
                return Err(format!("The {label} model name is not valid."));
            }
        }
        // The accelerator syntax is checked when it is registered: here only what is certainly wrong.
        let shortcut_ok = !self.shortcut.trim().is_empty() && self.shortcut.len() <= 64 && !self.shortcut.chars().any(char::is_control);
        if !shortcut_ok {
            return Err("The shortcut is not valid.".into());
        }
        if !self.activity_endpoint.is_empty() && !valid_endpoint(&self.activity_endpoint) {
            return Err("The server address must start with https:// or http://.".into());
        }
        if self.activity_sync && self.activity_endpoint.is_empty() {
            return Err("Enter the server address first.".into());
        }
        if self.activity_token.len() > 512 || !self.activity_token.chars().all(|c| c.is_ascii_graphic()) {
            return Err("The server token can only contain letters, digits and symbols, without spaces.".into());
        }
        Ok(())
    }

    /// The build's activity server, unless the user chose their own.
    fn with_build_activity(mut self, endpoint: &str, token: &str) -> Settings {
        // A mistyped build value is ignored: it must not make every saved record invalid.
        let (endpoint, token) =
            if valid_endpoint(endpoint) && token.len() <= 512 && token.chars().all(|c| c.is_ascii_graphic()) {
                (endpoint, token)
            } else {
                ("", "")
            };
        if !self.activity_custom {
            self.activity_sync = !endpoint.is_empty();
            self.activity_endpoint = endpoint.into();
            self.activity_token = token.into();
        }
        self
    }

    pub fn same_activity_server(&self, other: &Settings) -> bool {
        (self.activity_sync, &self.activity_endpoint, &self.activity_token)
            == (other.activity_sync, &other.activity_endpoint, &other.activity_token)
    }

    /// Identity (provider id, model) the preferences select, `None` when summaries are off.
    pub fn provider_key(&self) -> Option<(&'static str, &str)> {
        match self.recap_provider {
            RecapProviderKind::ClaudeCli => Some(("claude_cli", &self.recap_cli_model)),
            RecapProviderKind::Api => Some(("api", &self.recap_api_model)),
            RecapProviderKind::Off => None,
        }
    }

    /// Preferences used when the saved ones cannot be read: everything else
    /// keeps its default, but nothing is sent to a model until the user saves
    /// the settings again.
    fn recovered() -> Settings {
        Settings { recap_provider: RecapProviderKind::Off, ..Settings::default() }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub settings: Settings,
    /// Why the saved record was not used; `None` on a first install or a valid record.
    pub invalid: Option<String>,
}

/// Loads the preferences. Only a missing record is a first install: a record
/// that cannot be read keeps its content (copied aside), starts with
/// summaries off, and is replaced only by an explicit save.
pub fn load(conn: &Connection) -> Result<Loaded> {
    let Some(raw) = store::get_setting(conn, KEY)? else {
        store::delete_meta(conn, INVALID_META)?;
        return Ok(Loaded { settings: Settings::default(), invalid: None });
    };
    match parse(&raw) {
        Ok(settings) => {
            store::delete_meta(conn, INVALID_META)?;
            Ok(Loaded { settings, invalid: None })
        }
        Err(reason) => {
            if store::get_setting(conn, INVALID_KEY)?.is_none() {
                store::set_setting(conn, INVALID_KEY, &raw)?;
            }
            store::set_meta(conn, INVALID_META, &reason)?;
            Ok(Loaded { settings: Settings { shortcut: salvage_shortcut(&raw), ..Settings::recovered() }, invalid: Some(reason) })
        }
    }
}

fn parse(raw: &str) -> std::result::Result<Settings, String> {
    let settings: Settings = serde_json::from_str::<Settings>(raw)
        .map_err(|e| format!("unreadable settings: {e}"))?
        .with_build_activity(BUILD_ACTIVITY_ENDPOINT, BUILD_ACTIVITY_TOKEN);
    settings.validate().map_err(|e| format!("invalid settings: {e}"))?;
    Ok(settings)
}

/// The shortcut is the one preference worth keeping even from a damaged record.
fn salvage_shortcut(raw: &str) -> String {
    let default = Settings::default().shortcut;
    let Some(shortcut) = serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|v| v.get("shortcut").and_then(|s| s.as_str().map(str::to_string)))
    else {
        return default;
    };
    let candidate = Settings { shortcut: shortcut.clone(), ..Settings::default() };
    if candidate.validate().is_ok() {
        shortcut
    } else {
        default
    }
}

pub fn save(conn: &Connection, settings: &Settings) -> Result<()> {
    store::set_setting(conn, KEY, &serde_json::to_string(settings)?)?;
    // The user replaced the record that could not be read.
    store::delete_meta(conn, INVALID_META)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_defaults_for_missing_fields() {
        let conn = store::open_in_memory().unwrap();
        let first = load(&conn).unwrap();
        assert_eq!((first.settings, first.invalid), (Settings::default(), None));
        store::set_setting(&conn, KEY, r#"{"notifyFinished":false}"#).unwrap();
        let s = load(&conn).unwrap().settings;
        assert!(!s.notify_finished);
        assert_eq!(s.recap_provider, RecapProviderKind::ClaudeCli);
        save(&conn, &s).unwrap();
        assert_eq!(load(&conn).unwrap().settings, s);
    }

    #[test]
    fn unreadable_records_do_not_enable_summaries_and_are_preserved() {
        for raw in [
            "{not json",
            r#"{"recapProvider":"somebody_else","notifyFinished":false}"#,
            r#"{"recapProvider":"api","recapDailyLimit":"many"}"#,
            r#"{"recapProvider":"api","recapMonthlyBudgetUsd":-5}"#,
            r#"{"recapProvider":"claude_cli","recapCliModel":"--flag injected"}"#,
            r#"["array"]"#,
        ] {
            let conn = store::open_in_memory().unwrap();
            store::set_setting(&conn, KEY, raw).unwrap();
            let loaded = load(&conn).unwrap();
            assert_eq!(loaded.settings.recap_provider, RecapProviderKind::Off, "{raw}");
            assert!(loaded.invalid.is_some(), "{raw}");
            assert_eq!(store::get_setting(&conn, KEY).unwrap().as_deref(), Some(raw), "the record is not overwritten");
            assert_eq!(store::get_setting(&conn, INVALID_KEY).unwrap().as_deref(), Some(raw));
            assert!(store::meta(&conn, INVALID_META).unwrap().is_some());

            // An explicit save recovers.
            let mut fixed = loaded.settings.clone();
            fixed.recap_provider = RecapProviderKind::ClaudeCli;
            save(&conn, &fixed).unwrap();
            let again = load(&conn).unwrap();
            assert_eq!((again.settings.recap_provider, again.invalid), (RecapProviderKind::ClaudeCli, None));
            assert_eq!(store::get_setting(&conn, INVALID_KEY).unwrap().as_deref(), Some(raw), "the copy stays for inspection");
        }
    }

    #[test]
    fn validation_rejects_out_of_range_values() {
        let ok = Settings::default();
        assert!(ok.validate().is_ok());
        let bad = |f: &dyn Fn(&mut Settings)| {
            let mut s = Settings::default();
            f(&mut s);
            s.validate().is_err()
        };
        assert!(bad(&|s| s.recap_daily_limit = MAX_DAILY_LIMIT + 1));
        assert!(bad(&|s| s.recap_monthly_budget_usd = f64::NAN));
        assert!(bad(&|s| s.recap_monthly_budget_usd = -1.0));
        assert!(bad(&|s| s.recap_monthly_budget_usd = f64::INFINITY));
        assert!(bad(&|s| s.recap_api_model = String::new()));
        assert!(bad(&|s| s.recap_cli_model = "a b".into()));
        assert!(bad(&|s| s.recap_cli_model = "--dangerous".into()));
        assert!(bad(&|s| s.shortcut = String::new()));
        assert!(bad(&|s| s.shortcut = "Alt\n+J".into()));
        assert!(bad(&|s| s.shortcut = "x".repeat(65)));
        let s = Settings { recap_cli_model: "sonnet[1m]".into(), shortcut: "CmdOrCtrl+Shift+`".into(), ..Settings::default() };
        assert!(s.validate().is_ok());
    }

    #[test]
    fn the_build_server_applies_until_the_user_changes_it() {
        // Saved by a build without a server: the next build's server applies.
        let saved = Settings { activity_sync: false, activity_endpoint: String::new(), ..Settings::default() };
        let next = saved.clone().with_build_activity("https://stats.example.test/usage", "t");
        assert!(next.activity_sync);
        assert_eq!((next.activity_endpoint.as_str(), next.activity_token.as_str()), ("https://stats.example.test/usage", "t"));
        // Turned off or changed by the user: it stays as they left it.
        let own = Settings { activity_custom: true, ..saved.clone() };
        assert_eq!(own.clone().with_build_activity("https://stats.example.test/usage", "t"), own);
        let typo = saved.with_build_activity("stats.example.test", "t");
        assert!(!typo.activity_sync && typo.validate().is_ok(), "a wrong build value sends nothing");
    }

    #[test]
    fn the_activity_server_must_be_a_web_address() {
        let with = |sync: bool, endpoint: &str, token: &str| Settings {
            activity_sync: sync,
            activity_endpoint: endpoint.into(),
            activity_token: token.into(),
            ..Settings::default()
        };
        assert!(with(false, "", "").validate().is_ok());
        assert!(with(true, "https://stats.example.test/v1/usage", "abc-123").validate().is_ok());
        assert!(with(true, "http://10.0.0.5:8080/usage", "").validate().is_ok());
        assert!(with(true, "", "").validate().is_err(), "on without a server");
        assert!(with(false, "ftp://host", "").validate().is_err());
        assert!(with(false, "https://", "").validate().is_err());
        assert!(with(false, "https:///path", "").validate().is_err());
        assert!(with(false, "https://host/a b", "").validate().is_err());
        assert!(with(false, "", "two words").validate().is_err());
        assert!(with(false, "", "line\nbreak").validate().is_err());
    }
}
