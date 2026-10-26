//! Updates: periodically checks the latest release and, on request,
//! downloads the new bundle, verifies it and leaves the swap to
//! `install.sh` after the app quits. If the download fails the app
//! stays open as it was.

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tracker_core::updates::{self, Release};
use tracker_core::{legacy, now_ms, store, system};

use crate::AppState;

pub const EVENT_UPDATE: &str = "tracker://update";
/// Releases are rare: one check every half hour is enough, plus the manual one.
const CHECK_EVERY_MS: i64 = 30 * 60 * 1000;
/// First check shortly after launch, without slowing it down.
const FIRST_CHECK_AFTER: Duration = Duration::from_secs(20);
/// The clock is checked on every tick: after the Mac sleeps, the check doesn't wait.
const TICK: Duration = Duration::from_secs(60);
/// Version whose speech bubble has already been seen (click on the companion).
const META_DISMISSED: &str = "update_dismissed";

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub current: String,
    pub latest: Option<String>,
    pub url: Option<String>,
    pub available: bool,
    /// The companion shows the speech bubble: there's a new version not yet seen.
    pub announce: bool,
    pub checking: bool,
    pub installing: bool,
    pub checked_at: Option<i64>,
    pub error: Option<String>,
    /// Why the app can't update itself (dev build, protected folder).
    pub blocker: Option<String>,
}

#[derive(Default)]
pub struct Updates {
    state: Mutex<UpdateState>,
    release: Mutex<Option<Release>>,
}

impl Updates {
    pub fn view(&self) -> UpdateState {
        self.state.lock().expect("update").clone()
    }
}

fn emit(app: &AppHandle) {
    let _ = app.emit(EVENT_UPDATE, app.state::<AppState>().updates.view());
}

fn modify(app: &AppHandle, f: impl FnOnce(&mut UpdateState)) {
    f(&mut app.state::<AppState>().updates.state.lock().expect("update"));
    emit(app);
}

pub fn start(app: AppHandle) {
    let state = app.state::<AppState>();
    let bundle = std::env::current_exe().ok().and_then(|e| updates::running_bundle(&e));
    {
        let mut s = state.updates.state.lock().expect("update");
        s.current = app.package_info().version.to_string();
        s.blocker = if cfg!(debug_assertions) {
            Some("Development build: update from source".into())
        } else {
            updates::install_blocker(bundle.as_deref())
        };
    }
    // Leftovers from an update that's already installed.
    let _ = fs::remove_dir_all(state.paths.base.join("update"));
    thread::Builder::new()
        .name("updates".into())
        .spawn(move || {
            thread::sleep(FIRST_CHECK_AFTER);
            loop {
                let due = app.state::<AppState>().updates.view().checked_at.is_none_or(|t| now_ms() - t >= CHECK_EVERY_MS);
                if due {
                    check(&app);
                }
                thread::sleep(TICK);
            }
        })
        .expect("thread updates");
}

/// Checks the latest release. A check already in progress doesn't start another.
pub fn check(app: &AppHandle) -> UpdateState {
    let state = app.state::<AppState>();
    {
        let mut s = state.updates.state.lock().expect("update");
        if s.checking || s.installing {
            return s.clone();
        }
        s.checking = true;
    }
    emit(app);
    let result = system::which("gh", &state.path_env)
        .ok_or_else(|| "GitHub CLI is required: brew install gh && gh auth login".to_string())
        .and_then(|gh| updates::latest(&gh, &state.path_env, &updates::repo()).map_err(|e| e.to_string()));
    let dismissed = {
        let tracker = state.tracker.lock().expect("tracker");
        store::meta(&tracker.conn, META_DISMISSED).ok().flatten()
    };
    let mut s = state.updates.state.lock().expect("update");
    s.checking = false;
    s.checked_at = Some(now_ms());
    match result {
        Ok(release) => {
            s.error = None;
            s.available = release.as_ref().is_some_and(|r| updates::is_newer(&r.version, &s.current));
            s.latest = release.as_ref().map(|r| r.version.clone());
            s.url = release.as_ref().map(|r| r.url.clone());
            s.announce = s.available && dismissed.as_deref() != s.latest.as_deref();
            if s.available {
                log::info!("update available: {} → {}", s.current, s.latest.as_deref().unwrap_or("?"));
            }
            *state.updates.release.lock().expect("release") = release;
        }
        // A network error doesn't clear a new version that was already found.
        Err(err) => {
            log::warn!("update check failed: {err}");
            s.error = Some(err);
        }
    }
    let view = s.clone();
    drop(s);
    emit(app);
    view
}

/// The speech bubble has been seen: it won't show again for this version.
pub fn dismiss(app: &AppHandle) {
    let state = app.state::<AppState>();
    let latest = state.updates.view().latest;
    if let Some(v) = latest {
        let tracker = state.tracker.lock().expect("tracker");
        let _ = store::set_meta(&tracker.conn, META_DISMISSED, &v);
    }
    modify(app, |s| s.announce = false);
}

/// Downloads and verifies the new version, then quits, leaving `install.sh`
/// to replace the bundle and reopen the app.
pub fn install(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let release = state.updates.release.lock().expect("release").clone();
    {
        let mut s = state.updates.state.lock().expect("update");
        if let Some(b) = &s.blocker {
            return Err(b.clone());
        }
        if !s.available || release.is_none() {
            return Err("No update available".into());
        }
        if s.installing {
            return Ok(());
        }
        s.installing = true;
        s.error = None;
    }
    emit(app);
    let result = prepare_and_launch(&state, release.as_ref().expect("release"));
    match result {
        Ok(()) => {
            log::info!("update to {} started: the app will reopen", release.as_ref().map_or("?", |r| r.version.as_str()));
            let a = app.clone();
            // Give the UI time to show “Restarting…”.
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(600));
                a.exit(0);
            });
            Ok(())
        }
        Err(err) => {
            log::error!("update failed: {err}");
            modify(app, |s| {
                s.installing = false;
                s.error = Some(err.clone());
            });
            Err(err)
        }
    }
}

fn prepare_and_launch(state: &AppState, release: &Release) -> Result<(), String> {
    let gh = system::which("gh", &state.path_env).ok_or("GitHub CLI is required: brew install gh && gh auth login")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let current = updates::running_bundle(&exe).ok_or("The app isn't running from an installed bundle")?;
    let dest = current.parent().ok_or("invalid app folder")?;
    let work = state.paths.base.join("update");
    let bundle = updates::download(&gh, &state.path_env, &updates::repo(), release, &work.join(&release.version))
        .map_err(|e| format!("Download failed: {e}"))?;
    launch_installer(&work, &bundle, dest, &state.path_env, &state.paths.logs.join("update.log"))
}

/// Launches `install.sh` to install `bundle` into `dest` after the app quits.
fn launch_installer(work: &Path, bundle: &Path, dest: &Path, path_env: &str, log: &Path) -> Result<(), String> {
    fs::create_dir_all(work).map_err(|e| e.to_string())?;
    let script = work.join("install.sh");
    fs::write(&script, updates::INSTALL_SH).map_err(|e| e.to_string())?;
    let log = fs::File::create(log).map_err(|e| e.to_string())?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new("/bin/bash");
    cmd.arg(&script)
        .arg("--bundle")
        .arg(bundle)
        .arg("--wait-pid")
        .arg(std::process::id().to_string())
        .env("FLOW_KEEPER_APP_DIR", dest)
        .env("FLOW_KEEPER_REPO", updates::repo())
        .env("PATH", path_env)
        .stdin(std::process::Stdio::null())
        .stdout(log)
        .stderr(log_err);
    // Its own process group: the script outlives the app.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map_err(|e| format!("Couldn't start the installation: {e}"))?;
    Ok(())
}

/// Updating from 0.4: the old app installs the new bundle under its own
/// name, `AI Tracker.app`. On first launch the install script copies it
/// as `Flow Keeper.app`, removes the old one and reopens the app. `true` if
/// the app must quit right away, before touching any data.
pub fn relocate_legacy_bundle() -> bool {
    if cfg!(debug_assertions) {
        return false;
    }
    let Some(current) = std::env::current_exe().ok().and_then(|e| updates::running_bundle(&e)) else { return false };
    if current.file_name().is_none_or(|n| n != legacy::APP_BUNDLE) {
        return false;
    }
    let Some(dest) = current.parent() else { return false };
    // If the move fails the script reopens the old bundle: no retry
    // for a while, the app starts as it is.
    let work = std::env::temp_dir().join("flow-keeper-relocate");
    let recent = fs::metadata(&work)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().unwrap_or_default() < Duration::from_secs(600));
    if recent {
        return false;
    }
    let _ = fs::remove_dir_all(&work);
    launch_installer(&work, &current, dest, "/usr/bin:/bin:/usr/sbin:/sbin", &work.join("install.log")).is_ok()
}
