//! Local service: the only writer. It collects events from the spool,
//! checks processes, runs recaps and notifies the UI.

use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tracker_core::engine::{NotifyKind, NotifyRequest};
use tracker_core::recap::{self, ApiProvider, ClaudeCliProvider, RecapProvider, RunResult};
use tracker_core::settings::RecapProviderKind;
use tracker_core::views::{self, CompanionState};
use tracker_core::{doctor, now_ms, store, system};

use crate::AppState;

pub const EVENT_CHANGED: &str = "tracker://changed";
pub const EVENT_COMPANION: &str = "tracker://companion";
pub const EVENT_TURN_FINISHED: &str = "tracker://turn-finished";

const INGEST_EVERY: Duration = Duration::from_millis(400);
const SWEEP_EVERY: Duration = Duration::from_secs(15);
/// Refresh the UI periodically anyway: Idle and Unverifiable depend on time.
const REFRESH_EVERY: Duration = Duration::from_secs(30);
const ENV_EVERY: Duration = Duration::from_secs(600);
/// Window for grouping notifications that arrive close together.
const NOTIFY_WINDOW: Duration = Duration::from_secs(3);

pub fn start(app: AppHandle) {
    let a = app.clone();
    thread::Builder::new().name("collector".into()).spawn(move || collector(a)).expect("thread collector");
    let a = app.clone();
    thread::Builder::new().name("recap".into()).spawn(move || recap_worker(a)).expect("thread recap");
    thread::Builder::new().name("environment".into()).spawn(move || environment_worker(app)).expect("thread env");
}

pub fn notify_changed(app: &AppHandle) {
    let _ = app.emit(EVENT_CHANGED, ());
    push_companion(app, true);
}

fn push_companion(app: &AppHandle, force: bool) {
    static LAST: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
    let Some(state) = companion_state(app) else { return };
    let key = serde_json::to_string(&state).unwrap_or_default();
    let mut last = LAST.lock().expect("companion");
    if force || last.as_deref() != Some(&key) {
        *last = Some(key);
        let _ = app.emit(EVENT_COMPANION, state);
    }
}

pub fn companion_state(app: &AppHandle) -> Option<CompanionState> {
    let state = app.state::<AppState>();
    let problem = state.problem.load(Ordering::Relaxed) || state.environment_problem();
    let tracker = state.tracker.lock().ok()?;
    views::companion(&tracker.conn, now_ms(), problem).ok()
}

fn collector(app: AppHandle) {
    let state = app.state::<AppState>();
    let mut last_sweep = Instant::now() - SWEEP_EVERY;
    let mut last_refresh = Instant::now();
    let mut pending: Vec<NotifyRequest> = Vec::new();
    let mut pending_since: Option<Instant> = None;
    loop {
        let now = now_ms();
        let mut changed = false;
        {
            let mut tracker = state.tracker.lock().expect("tracker");
            match tracker.ingest_spool(&state.paths.spool, now) {
                Ok(report) => {
                    state.problem.store(false, Ordering::Relaxed);
                    changed |= report.changed || !report.probes.is_empty();
                    if report.notifications.iter().any(|n| n.kind == NotifyKind::Finished) {
                        let _ = app.emit(EVENT_TURN_FINISHED, ());
                    }
                    if !report.notifications.is_empty() {
                        pending_since.get_or_insert_with(Instant::now);
                        pending.extend(report.notifications);
                    }
                }
                Err(err) => {
                    log::error!("ingest failed: {err:#}");
                    state.problem.store(true, Ordering::Relaxed);
                }
            }
            if last_sweep.elapsed() >= SWEEP_EVERY {
                last_sweep = Instant::now();
                if let Ok(pids) = tracker.live_pids() {
                    if let Some(alive) = system::processes_alive(&pids) {
                        let lookup = |pid: i64| alive.iter().find(|(p, _)| *p == pid).map(|(_, a)| *a);
                        if let Ok(closed) = tracker.sweep_liveness(now, &lookup) {
                            changed |= !closed.is_empty();
                        }
                    }
                }
            }
            if let Some((x, y)) = state.companion_pos.lock().expect("pos").take() {
                let _ = store::set_meta(&tracker.conn, "companion_pos", &format!("{x},{y}"));
            }
        }
        if pending_since.is_some_and(|t| t.elapsed() >= NOTIFY_WINDOW) {
            send_notifications(&app, std::mem::take(&mut pending));
            pending_since = None;
        }
        if changed || last_refresh.elapsed() >= REFRESH_EVERY {
            last_refresh = Instant::now();
            if changed {
                let _ = app.emit(EVENT_CHANGED, ());
            } else {
                let _ = app.emit(EVENT_CHANGED, "tick");
            }
        }
        push_companion(&app, false);
        thread::sleep(INGEST_EVERY);
    }
}

/// Notifications deduplicated per event and grouped per project.
fn send_notifications(app: &AppHandle, reqs: Vec<NotifyRequest>) {
    let state = app.state::<AppState>();
    let settings = state.settings();
    if settings.do_not_disturb {
        return;
    }
    let now = now_ms();
    let mut groups: BTreeMap<(String, &'static str), Vec<views::ConversationCard>> = BTreeMap::new();
    {
        let tracker = state.tracker.lock().expect("tracker");
        for req in &reqs {
            let enabled = match req.kind {
                NotifyKind::Attention => settings.notify_attention,
                NotifyKind::Finished => settings.notify_finished,
            };
            if !enabled {
                continue;
            }
            let Ok(Some(card)) = views::conversation(&tracker.conn, req.conversation_id, now) else { continue };
            // The state may have changed already during the grouping window.
            let still = match req.kind {
                NotifyKind::Attention => card.state == views::DisplayState::Waiting,
                NotifyKind::Finished => card.state == views::DisplayState::Finished,
            };
            if !still || card.hidden {
                continue;
            }
            let project = project_name(&tracker.conn, card.project_id);
            let list = groups.entry((project, req.kind.code())).or_default();
            if !list.iter().any(|c| c.id == card.id) {
                list.push(card);
            }
        }
    }
    for ((project, kind), cards) in groups {
        let (title, body) = match (kind, cards.as_slice()) {
            ("attention", [one]) => (
                format!("Waiting for you · {}", one.title),
                format!("{} — {}", project, one.state_reason.clone().unwrap_or_else(|| "Needs your input".into())),
            ),
            ("attention", many) => (format!("{} conversations waiting for you", many.len()), project.clone()),
            (_, [one]) => (format!("Finished · {}", one.title), format!("{project} — Response complete")),
            (_, many) => (format!("{} conversations finished", many.len()), project.clone()),
        };
        let _ = app.notification().builder().title(title).body(body).show();
    }
}

fn project_name(conn: &store::Connection, id: Option<i64>) -> String {
    id.and_then(|id| conn.query_row("SELECT name FROM projects WHERE id = ?1", [id], |r| r.get::<_, String>(0)).ok())
        .unwrap_or_else(|| views::UNASSIGNED_NAME.to_string())
}

pub fn build_provider(state: &AppState) -> Result<Box<dyn RecapProvider>, String> {
    let settings = state.settings();
    match settings.recap_provider {
        RecapProviderKind::ClaudeCli => {
            let binary = system::which("claude", &state.path_env).ok_or("Claude Code not found")?;
            Ok(Box::new(ClaudeCliProvider {
                binary,
                model: settings.recap_cli_model.clone(),
                path_env: Some(state.path_env.clone()),
                workdir: state.paths.base.join("recap-work"),
                timeout: Duration::from_secs(180),
            }))
        }
        RecapProviderKind::Api => {
            let api_key = system::api_key().ok_or("API key missing from the keychain")?;
            Ok(Box::new(ApiProvider { api_key, model: settings.recap_api_model.clone() }))
        }
        RecapProviderKind::Off => Err("summaries are turned off".into()),
    }
}

fn recap_worker(app: AppHandle) {
    let state = app.state::<AppState>();
    loop {
        thread::sleep(Duration::from_secs(3));
        let settings = state.settings();
        let job = {
            let tracker = state.tracker.lock().expect("tracker");
            recap::next_job(&tracker.conn, &settings, now_ms())
        };
        let job = match job {
            Ok(Some(job)) => job,
            Ok(None) => continue,
            Err(err) => {
                log::error!("recap queue: {err:#}");
                continue;
            }
        };
        let (provider_id, result) = match build_provider(&state) {
            Ok(provider) => (provider.id(), recap::run(&job, provider.as_ref())),
            Err(err) => ("none", RunResult { chunks: vec![], error: Some(err), more: false }),
        };
        {
            let mut tracker = state.tracker.lock().expect("tracker");
            if let Err(err) = recap::store_result(&mut tracker.conn, &job, provider_id, &result, now_ms()) {
                log::error!("saving recap: {err:#}");
            }
        }
        let _ = app.emit(EVENT_CHANGED, ());
    }
}

/// Periodically detects CLIs and plugins (slow, done outside the lock).
fn environment_worker(app: AppHandle) {
    loop {
        refresh_environment(&app);
        thread::sleep(ENV_EVERY);
    }
}

pub fn refresh_environment(app: &AppHandle) -> doctor::Environment {
    let state = app.state::<AppState>();
    // After an app update, the already connected plugin is resynced;
    // the AI Tracker one is replaced.
    if let Some(claude) = system::which("claude", &state.path_env) {
        let dir = state.paths.base.join("claude-plugin");
        match tracker_core::connect::migrate_legacy(&claude, &state.path_env, &dir) {
            Ok(true) => log::info!("replaced the AI Tracker plugin in Claude Code"),
            Ok(false) => {}
            Err(err) => log::warn!("replacing the AI Tracker plugin in Claude Code: {err}"),
        }
        let _ = tracker_core::connect::sync_if_connected(&claude, &state.path_env, &dir);
    }
    if let Some(codex) = system::which("codex", &state.path_env) {
        let dir = state.paths.base.join("codex-plugin");
        match tracker_core::connect::codex_migrate_legacy(&codex, &state.path_env, &dir) {
            Ok(true) => log::info!("replaced the AI Tracker plugin in Codex"),
            Ok(false) => {}
            Err(err) => log::warn!("replacing the AI Tracker plugin in Codex: {err}"),
        }
        let _ = tracker_core::connect::codex_sync_if_connected(&codex, &state.path_env, &dir);
    }
    tracker_core::editors::sync_installed(&state.paths.base.join("editor-extension"));
    let env = doctor::detect(&state.path_env);
    *state.environment.lock().expect("environment") = Some(env.clone());
    push_companion(app, true);
    env
}
