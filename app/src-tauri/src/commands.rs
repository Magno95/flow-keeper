//! Panel commands. The UI never writes to the store directly:
//! every change goes through here and the engine's rules.

use std::sync::atomic::Ordering;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::{NotificationExt, PermissionState};
use tracker_core::access::{self, AccessResult};
use tracker_core::doctor::{self, Check};
use tracker_core::settings::{self, Settings};
use tracker_core::views::{self, CompanionState, ConversationCard, Overview, ProjectDetail, ProjectOption};
use tracker_core::{connect, editors, now_ms, recap, system};

use crate::service::{self, notify_changed};
use crate::updates::{self, UpdateState};
use crate::windows;
use crate::AppState;

type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn get_overview(state: State<'_, AppState>) -> Res<Overview> {
    let t = state.tracker.lock().map_err(err)?;
    views::overview(&t.conn, now_ms()).map_err(err)
}

#[tauri::command]
pub async fn get_project(state: State<'_, AppState>, id: Option<i64>) -> Res<Option<ProjectDetail>> {
    let t = state.tracker.lock().map_err(err)?;
    views::project_detail(&t.conn, id, now_ms()).map_err(err)
}

#[tauri::command]
pub async fn get_removed(state: State<'_, AppState>) -> Res<Vec<ConversationCard>> {
    let t = state.tracker.lock().map_err(err)?;
    views::removed(&t.conn, now_ms()).map_err(err)
}

#[tauri::command]
pub async fn search(state: State<'_, AppState>, query: String, include_hidden: bool) -> Res<Vec<ConversationCard>> {
    let t = state.tracker.lock().map_err(err)?;
    views::search(&t.conn, &query, include_hidden, now_ms()).map_err(err)
}

#[tauri::command]
pub async fn get_companion(app: AppHandle) -> Res<Option<CompanionState>> {
    Ok(service::companion_state(&app))
}

#[tauri::command]
pub async fn list_projects(state: State<'_, AppState>) -> Res<Vec<ProjectOption>> {
    let t = state.tracker.lock().map_err(err)?;
    views::project_options(&t.conn).map_err(err)
}

#[tauri::command]
pub async fn toggle_panel(app: AppHandle) -> Res<()> {
    let a = app.clone();
    app.run_on_main_thread(move || windows::toggle_panel(&a)).map_err(err)
}

#[tauri::command]
pub async fn hide_panel(app: AppHandle) -> Res<()> {
    let a = app.clone();
    app.run_on_main_thread(move || windows::hide_panel(&a)).map_err(err)
}

fn target(state: &AppState, id: i64) -> Res<access::Target> {
    let t = state.tracker.lock().map_err(err)?;
    access::target(&t.conn, id).map_err(err)?.ok_or_else(|| "conversation not found".to_string())
}

/// Going to the conversation counts as reading its result.
fn mark_read(app: &AppHandle, state: &AppState, id: i64) -> Res<()> {
    state.tracker.lock().map_err(err)?.mark_read(id).map_err(err)?;
    notify_changed(app);
    Ok(())
}

#[tauri::command]
pub async fn open_conversation(app: AppHandle, state: State<'_, AppState>, id: i64) -> Res<AccessResult> {
    let t = target(&state, id)?;
    mark_read(&app, &state, id)?;
    Ok(access::focus(&t))
}

#[tauri::command]
pub async fn resume_conversation(app: AppHandle, state: State<'_, AppState>, id: i64) -> Res<AccessResult> {
    let t = target(&state, id)?;
    mark_read(&app, &state, id)?;
    Ok(access::resume(&t))
}

#[tauri::command]
pub async fn mark_project_read(app: AppHandle, state: State<'_, AppState>, id: Option<i64>) -> Res<()> {
    state.tracker.lock().map_err(err)?.mark_project_read(id).map_err(err)?;
    notify_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn choose_folder_and_resume(app: AppHandle, state: State<'_, AppState>, id: i64) -> Res<Option<AccessResult>> {
    state.dialog_open.store(true, Ordering::Relaxed);
    let picked = app.dialog().file().set_title("Choose the conversation's folder").blocking_pick_folder();
    state.dialog_open.store(false, Ordering::Relaxed);
    let Some(folder) = picked.and_then(|f| f.into_path().ok()) else { return Ok(None) };
    {
        let mut t = state.tracker.lock().map_err(err)?;
        t.update_cwd(id, &folder.to_string_lossy()).map_err(err)?;
    }
    let t = target(&state, id)?;
    Ok(Some(access::resume(&t)))
}

#[tauri::command]
pub async fn hide_conversation(app: AppHandle, state: State<'_, AppState>, id: i64) -> Res<()> {
    state.tracker.lock().map_err(err)?.hide(id, now_ms()).map_err(err)?;
    notify_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn unhide_conversation(app: AppHandle, state: State<'_, AppState>, id: i64) -> Res<()> {
    state.tracker.lock().map_err(err)?.unhide(id).map_err(err)?;
    notify_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn dismiss_attention(app: AppHandle, state: State<'_, AppState>, id: i64) -> Res<()> {
    state.tracker.lock().map_err(err)?.dismiss_attention(id).map_err(err)?;
    notify_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn rename_conversation(app: AppHandle, state: State<'_, AppState>, id: i64, title: Option<String>) -> Res<()> {
    state.tracker.lock().map_err(err)?.rename(id, title.as_deref()).map_err(err)?;
    notify_changed(&app);
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveRequest {
    pub id: i64,
    pub project_id: Option<i64>,
    pub new_project_name: Option<String>,
    /// Also creates a rule for the folder (only when explicitly chosen).
    pub associate_folder: bool,
}

#[tauri::command]
pub async fn move_conversation(app: AppHandle, state: State<'_, AppState>, req: MoveRequest) -> Res<Option<i64>> {
    let mut t = state.tracker.lock().map_err(err)?;
    let now = now_ms();
    let project = match (&req.new_project_name, req.project_id) {
        (Some(name), _) if !name.trim().is_empty() => Some(t.create_project(name, now).map_err(err)?),
        (_, id) => id,
    };
    t.move_conversation(req.id, project).map_err(err)?;
    if req.associate_folder {
        if let Some(pid) = project {
            let folder: Option<String> = t
                .conn
                .query_row("SELECT COALESCE(worktree_root, cwd) FROM conversations WHERE id = ?1", [req.id], |r| r.get(0))
                .map_err(err)?;
            if let Some(folder) = folder {
                t.associate_folder(&folder, pid).map_err(err)?;
            }
        }
    }
    drop(t);
    notify_changed(&app);
    Ok(project)
}

#[tauri::command]
pub async fn request_recap(app: AppHandle, state: State<'_, AppState>, id: i64) -> Res<()> {
    let t = state.tracker.lock().map_err(err)?;
    recap::request(&t.conn, id, now_ms()).map_err(err)?;
    drop(t);
    notify_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn update_project(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    name: Option<String>,
    description: Option<String>,
    set_description: bool,
) -> Res<()> {
    let desc = set_description.then_some(description.as_deref());
    state.tracker.lock().map_err(err)?.update_project(id, name.as_deref(), desc).map_err(err)?;
    notify_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_project_excluded(app: AppHandle, state: State<'_, AppState>, id: i64, excluded: bool) -> Res<()> {
    state.tracker.lock().map_err(err)?.set_project_excluded(id, excluded).map_err(err)?;
    notify_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn merge_projects(app: AppHandle, state: State<'_, AppState>, from: i64, into: i64) -> Res<()> {
    state.tracker.lock().map_err(err)?.merge_projects(from, into).map_err(err)?;
    notify_changed(&app);
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub settings: Settings,
    pub api_key_present: bool,
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Res<SettingsView> {
    Ok(SettingsView { settings: state.settings(), api_key_present: system::api_key().is_some() })
}

#[tauri::command]
pub async fn save_settings(app: AppHandle, state: State<'_, AppState>, settings: Settings) -> Res<()> {
    let previous = state.settings();
    {
        let mut t = state.tracker.lock().map_err(err)?;
        settings::save(&t.conn, &settings).map_err(err)?;
        t.normalize_options.track_print_sessions = settings.track_print_sessions;
        if previous.recap_provider != settings.recap_provider
            || previous.recap_daily_limit != settings.recap_daily_limit
            || previous.recap_monthly_budget_usd != settings.recap_monthly_budget_usd
        {
            recap::reschedule_blocked(&t.conn, now_ms()).map_err(err)?;
        }
    }
    *state.settings.lock().map_err(err)? = settings.clone();
    let a = app.clone();
    let s = settings.clone();
    app.run_on_main_thread(move || {
        if previous.shortcut != s.shortcut {
            windows::register_shortcut(&a, &s.shortcut);
        }
        windows::set_companion_visible(&a, s.companion_visible);
    })
    .map_err(err)?;
    if !cfg!(debug_assertions) && previous.launch_at_login != settings.launch_at_login {
        use tauri_plugin_autostart::ManagerExt;
        let _ = if settings.launch_at_login { app.autolaunch().enable() } else { app.autolaunch().disable() };
    }
    notify_changed(&app);
    Ok(())
}

#[tauri::command]
pub async fn set_api_key(app: AppHandle, state: State<'_, AppState>, key: Option<String>) -> Res<()> {
    system::set_api_key(key.as_deref()).map_err(err)?;
    let t = state.tracker.lock().map_err(err)?;
    recap::reschedule_blocked(&t.conn, now_ms()).map_err(err)?;
    drop(t);
    service::refresh_environment(&app);
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupState {
    pub detected: bool,
    pub claude_found: bool,
    pub claude_version: Option<String>,
    pub version_tested: bool,
    pub plugin_installed: bool,
    pub plugin_enabled: bool,
    pub plugin_managed: bool,
    pub codex_found: bool,
    pub codex_version: Option<String>,
    pub codex_version_tested: bool,
    pub codex_plugin_installed: bool,
    pub codex_plugin_enabled: bool,
    pub codex_plugin_managed: bool,
    /// Connected, but the hooks still need approval in Codex with `/hooks`.
    pub codex_hooks_pending: bool,
    pub conversations_seen: i64,
    pub onboarding_done: bool,
}

fn setup_state(state: &AppState) -> Res<SetupState> {
    let env = state.environment.lock().map_err(err)?.clone();
    let seen: i64 = {
        let t = state.tracker.lock().map_err(err)?;
        t.conn.query_row("SELECT count(*) FROM conversations", [], |r| r.get(0)).map_err(err)?
    };
    let s = state.settings();
    Ok(match env {
        None => SetupState {
            detected: false,
            claude_found: false,
            claude_version: None,
            version_tested: false,
            plugin_installed: false,
            plugin_enabled: false,
            plugin_managed: false,
            codex_found: false,
            codex_version: None,
            codex_version_tested: false,
            codex_plugin_installed: false,
            codex_plugin_enabled: false,
            codex_plugin_managed: false,
            codex_hooks_pending: false,
            conversations_seen: seen,
            onboarding_done: s.onboarding_done,
        },
        Some(env) => SetupState {
            detected: true,
            claude_found: env.claude_path.is_some(),
            version_tested: env.claude_version.as_deref().is_some_and(|v| system::TESTED_CLAUDE_VERSIONS.contains(&v)),
            claude_version: env.claude_version,
            plugin_installed: env.plugin.as_ref().is_some_and(|p| p.installed),
            plugin_enabled: env.plugin.as_ref().is_some_and(|p| p.installed && p.enabled),
            plugin_managed: env.plugin.as_ref().is_some_and(|p| p.managed),
            codex_found: env.codex_path.is_some(),
            codex_version_tested: env.codex_version.as_deref().is_some_and(|v| system::TESTED_CODEX_VERSIONS.contains(&v)),
            codex_plugin_installed: env.codex_plugin.as_ref().is_some_and(|p| p.plugin.installed),
            codex_plugin_enabled: env.codex_plugin.as_ref().is_some_and(|p| p.plugin.installed && p.plugin.enabled),
            codex_plugin_managed: env.codex_plugin.as_ref().is_some_and(|p| p.plugin.managed),
            codex_hooks_pending: env.codex_plugin.as_ref().is_some_and(|p| p.needs_review()),
            codex_version: env.codex_version,
            conversations_seen: seen,
            onboarding_done: s.onboarding_done,
        },
    })
}

#[tauri::command]
pub async fn get_setup_state(state: State<'_, AppState>) -> Res<SetupState> {
    setup_state(&state)
}

#[tauri::command]
pub async fn refresh_environment(app: AppHandle, state: State<'_, AppState>) -> Res<SetupState> {
    service::refresh_environment(&app);
    setup_state(&state)
}

#[tauri::command]
pub async fn connect_claude(app: AppHandle, state: State<'_, AppState>) -> Res<SetupState> {
    let claude = system::which("claude", &state.path_env).ok_or("Claude Code not found")?;
    connect::connect(&claude, &state.path_env, &state.paths.base.join("claude-plugin")).map_err(err)?;
    service::refresh_environment(&app);
    setup_state(&state)
}

#[tauri::command]
pub async fn disconnect_claude(app: AppHandle, state: State<'_, AppState>) -> Res<SetupState> {
    let claude = system::which("claude", &state.path_env).ok_or("Claude Code not found")?;
    connect::disconnect(&claude, &state.path_env).map_err(err)?;
    service::refresh_environment(&app);
    setup_state(&state)
}

#[tauri::command]
pub async fn connect_codex(app: AppHandle, state: State<'_, AppState>) -> Res<SetupState> {
    let codex = system::which("codex", &state.path_env).ok_or("Codex not found")?;
    connect::codex_connect(&codex, &state.path_env, &state.paths.base.join("codex-plugin")).map_err(err)?;
    service::refresh_environment(&app);
    setup_state(&state)
}

#[tauri::command]
pub async fn disconnect_codex(app: AppHandle, state: State<'_, AppState>) -> Res<SetupState> {
    let codex = system::which("codex", &state.path_env).ok_or("Codex not found")?;
    connect::codex_disconnect(&codex, &state.path_env).map_err(err)?;
    service::refresh_environment(&app);
    setup_state(&state)
}

#[tauri::command]
pub async fn run_doctor(app: AppHandle, state: State<'_, AppState>) -> Res<Vec<Check>> {
    let env = service::refresh_environment(&app);
    let t = state.tracker.lock().map_err(err)?;
    doctor::checks(&t.conn, &state.paths, &env, &state.settings(), now_ms()).map_err(err)
}

#[tauri::command]
pub async fn start_probe(state: State<'_, AppState>) -> Res<String> {
    let claude = system::which("claude", &state.path_env).ok_or("Claude Code not found")?;
    let nonce = format!("p{}{}", now_ms(), std::process::id());
    let path_env = state.path_env.clone();
    let workdir = state.paths.base.join("probe-work");
    let n = nonce.clone();
    std::thread::spawn(move || {
        let _ = doctor::start_probe(&claude, &path_env, &workdir, &n);
    });
    Ok(nonce)
}

#[tauri::command]
pub async fn probe_status(state: State<'_, AppState>, nonce: String) -> Res<bool> {
    let t = state.tracker.lock().map_err(err)?;
    doctor::probe_received(&t.conn, &nonce).map_err(err)
}

#[tauri::command]
pub async fn request_notification_permission(app: AppHandle) -> Res<bool> {
    let n = app.notification();
    let state = match n.permission_state().map_err(err)? {
        PermissionState::Granted => PermissionState::Granted,
        _ => n.request_permission().map_err(err)?,
    };
    Ok(matches!(state, PermissionState::Granted))
}

#[tauri::command]
pub async fn quit_app(app: AppHandle) -> Res<()> {
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub async fn get_editors() -> Res<Vec<editors::EditorStatus>> {
    Ok(editors::detect())
}

fn editor(bundle: &str) -> Res<&'static editors::Editor> {
    editors::by_bundle(bundle).ok_or_else(|| "Editor not supported".to_string())
}

#[tauri::command]
pub async fn install_editor_extension(state: State<'_, AppState>, bundle: String) -> Res<Vec<editors::EditorStatus>> {
    let work = state.paths.base.join("editor-extension");
    std::fs::create_dir_all(&work).map_err(err)?;
    editor(&bundle)?.install(&work).map_err(err)?;
    Ok(editors::detect())
}

#[tauri::command]
pub async fn uninstall_editor_extension(bundle: String) -> Res<Vec<editors::EditorStatus>> {
    editor(&bundle)?.uninstall().map_err(err)?;
    Ok(editors::detect())
}

#[tauri::command]
pub async fn get_update(state: State<'_, AppState>) -> Res<UpdateState> {
    Ok(state.updates.view())
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Res<UpdateState> {
    Ok(updates::check(&app))
}

#[tauri::command]
pub async fn dismiss_update(app: AppHandle) -> Res<()> {
    updates::dismiss(&app);
    Ok(())
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Res<()> {
    updates::install(&app)
}

/// Click on the speech bubble: the panel opens on the update settings.
#[tauri::command]
pub async fn open_updates(app: AppHandle) -> Res<()> {
    let a = app.clone();
    app.run_on_main_thread(move || {
        windows::show_panel(&a);
        let _ = a.emit_to(windows::PANEL, windows::EVENT_NAVIGATE, "updates");
    })
    .map_err(err)
}

#[tauri::command]
pub async fn set_companion_headroom(app: AppHandle, extra: f64) -> Res<()> {
    let a = app.clone();
    app.run_on_main_thread(move || windows::set_companion_headroom(&a, extra)).map_err(err)
}
