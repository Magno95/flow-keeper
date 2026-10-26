//! Desktop shell: starts the local service (collection + recap), the
//! companion and panel windows, and the menu bar item.

mod commands;
mod service;
mod updates;
mod windows;

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;

use tauri::{Manager, RunEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tracker_core::doctor::Environment;
use tracker_core::engine::Tracker;
use tracker_core::paths::Paths;
use tracker_core::projects::GitInspector;
use tracker_core::settings::{self, Settings};
use tracker_core::{store, system};

pub struct AppState {
    pub tracker: Mutex<Tracker>,
    pub paths: Paths,
    pub path_env: String,
    pub settings: Mutex<Settings>,
    pub environment: Mutex<Option<Environment>>,
    /// Tracker problem (collection or connection), shown by the companion.
    pub problem: AtomicBool,
    /// A native dialog is open: the panel doesn't hide when it loses focus.
    pub dialog_open: AtomicBool,
    pub companion_pos: Mutex<Option<(f64, f64)>>,
    /// Extra space above the companion: logical px and shift in physical px.
    pub companion_headroom: Mutex<(f64, f64)>,
    pub updates: updates::Updates,
}

impl AppState {
    pub fn settings(&self) -> Settings {
        self.settings.lock().expect("settings").clone()
    }

    pub fn environment_problem(&self) -> bool {
        match &*self.environment.lock().expect("environment") {
            // At least one connected agent is needed; Codex connected but with hooks
            // still awaiting approval collects nothing.
            Some(env) => {
                !(env.claude_connected() || env.codex_connected())
                    || env.codex_plugin.as_ref().is_some_and(|p| p.needs_review())
            }
            None => false,
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            windows::show_panel(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, None))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            if updates::relocate_legacy_bundle() {
                std::process::exit(0);
            }
            let migrated = tracker_core::legacy::migrate();
            let paths = Paths::default_location();
            paths.ensure()?;
            tracker_core::logging::init(&paths.logs);
            log::info!("starting Flow Keeper {}", env!("CARGO_PKG_VERSION"));
            for note in migrated {
                log::info!("migration from AI Tracker: {note}");
            }
            let path_env = system::search_path(system::login_shell_path().as_deref());
            let git = system::which("git", &path_env)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|| "git".into());
            let conn = store::open(&paths.db)?;
            let settings = settings::load(&conn)?;
            let mut tracker = Tracker::new(conn, Box::new(GitInspector::new(git)));
            tracker.normalize_options.track_print_sessions = settings.track_print_sessions;

            app.manage(AppState {
                tracker: Mutex::new(tracker),
                paths,
                path_env,
                settings: Mutex::new(settings.clone()),
                environment: Mutex::new(None),
                problem: AtomicBool::new(false),
                dialog_open: AtomicBool::new(false),
                companion_pos: Mutex::new(None),
                companion_headroom: Mutex::new((0.0, 0.0)),
                updates: updates::Updates::default(),
            });

            windows::setup_tray(app.handle())?;
            windows::restore_companion(app.handle());
            if settings.companion_visible {
                windows::set_companion_visible(app.handle(), true);
            }
            windows::register_shortcut(app.handle(), &settings.shortcut);
            if !cfg!(debug_assertions) {
                let autostart = app.autolaunch();
                let enabled = autostart.is_enabled().unwrap_or(false);
                if settings.launch_at_login && !enabled {
                    let _ = autostart.enable();
                } else if !settings.launch_at_login && enabled {
                    let _ = autostart.disable();
                }
            }
            if !settings.onboarding_done {
                windows::show_panel(app.handle());
            }
            service::start(app.handle().clone());
            updates::start(app.handle().clone());
            Ok(())
        })
        .on_window_event(windows::on_window_event)
        .invoke_handler(tauri::generate_handler![
            commands::get_overview,
            commands::get_project,
            commands::get_removed,
            commands::search,
            commands::get_companion,
            commands::list_projects,
            commands::toggle_panel,
            commands::hide_panel,
            commands::open_conversation,
            commands::resume_conversation,
            commands::choose_folder_and_resume,
            commands::hide_conversation,
            commands::unhide_conversation,
            commands::dismiss_attention,
            commands::rename_conversation,
            commands::move_conversation,
            commands::request_recap,
            commands::update_project,
            commands::set_project_excluded,
            commands::merge_projects,
            commands::get_settings,
            commands::save_settings,
            commands::set_api_key,
            commands::get_setup_state,
            commands::refresh_environment,
            commands::connect_claude,
            commands::disconnect_claude,
            commands::connect_codex,
            commands::disconnect_codex,
            commands::run_doctor,
            commands::start_probe,
            commands::probe_status,
            commands::request_notification_permission,
            commands::quit_app,
            commands::get_editors,
            commands::install_editor_extension,
            commands::uninstall_editor_extension,
            commands::get_update,
            commands::check_update,
            commands::dismiss_update,
            commands::install_update,
            commands::open_updates,
            commands::set_companion_headroom,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start the app")
        .run(|_app, event| {
            // Closing the windows doesn't stop collection: you can only quit from the menu.
            if let RunEvent::ExitRequested { api, code, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
