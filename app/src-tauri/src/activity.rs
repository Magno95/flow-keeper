//! Use counters (`tracker_core::activity`): recorded on the panel's actions
//! and, when a server is set in the settings, sent to it in the background.
//! When the server is out of reach the request fails quickly and the hours wait
//! in the store until the next attempt.

use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use anyhow::{bail, Result};
use tauri::{AppHandle, Manager};
use tracker_core::activity::{self, Counter};
use tracker_core::now_ms;
use tracker_core::settings::Settings;

use crate::AppState;

const TICK: Duration = Duration::from_secs(60);
const FIRST_AFTER_MS: i64 = 30_000;
const EVERY_MS: i64 = 15 * 60 * 1000;
/// After a failure: 5, 10, 20, 40, then every 60 minutes.
const RETRY_MIN_MS: i64 = 5 * 60 * 1000;
const RETRY_MAX_MS: i64 = 60 * 60 * 1000;
const PRUNE_EVERY_MS: i64 = 24 * 60 * 60 * 1000;

static NEXT_AT: AtomicI64 = AtomicI64::new(0);
static FAILURES: AtomicU32 = AtomicU32::new(0);
/// One send at a time: the worker and “Send now” never overlap.
static SENDING: Mutex<()> = Mutex::new(());

/// Counts in the background: the caller can be the main thread.
pub fn record(app: &AppHandle, counter: Counter) {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let Ok(t) = state.tracker.lock() else { return };
        if let Err(e) = activity::record(&t.conn, counter, now_ms()) {
            log::warn!("activity not recorded: {e}");
        }
    });
}

/// The server settings changed: the next attempt is right away.
pub fn reschedule() {
    FAILURES.store(0, Ordering::Relaxed);
    NEXT_AT.store(0, Ordering::Relaxed);
}

pub fn start(app: AppHandle) {
    NEXT_AT.store(now_ms() + FIRST_AFTER_MS, Ordering::Relaxed);
    thread::Builder::new().name("activity".into()).spawn(move || worker(app)).expect("thread activity");
}

fn worker(app: AppHandle) {
    let state = app.state::<AppState>();
    let mut pruned_at = 0;
    loop {
        let now = now_ms();
        if now - pruned_at >= PRUNE_EVERY_MS {
            pruned_at = now;
            if let Ok(t) = state.tracker.lock() {
                let _ = activity::prune(&t.conn, now);
            }
        }
        let s = state.settings();
        let pending = state.tracker.lock().ok().and_then(|t| activity::pending_count(&t.conn).ok()).unwrap_or(0);
        if s.activity_sync && pending > 0 && now >= NEXT_AT.load(Ordering::Relaxed) {
            let _ = sync(&app);
        }
        thread::sleep(TICK);
    }
}

/// Sends every pending hour, in batches; at least one request even when
/// nothing is pending, so “Send now” also checks the server. Returns the
/// hours sent.
pub fn sync(app: &AppHandle) -> Result<usize> {
    let _sending = SENDING.lock().unwrap_or_else(|e| e.into_inner());
    let state = app.state::<AppState>();
    let s = state.settings();
    if !s.activity_sync || s.activity_endpoint.is_empty() {
        bail!("Sending to a server is off.");
    }
    let version = app.package_info().version.to_string();
    let mut sent = 0;
    let result = send_pending(state.inner(), &s, &version, &mut sent);
    let now = now_ms();
    let error = result.as_ref().err().map(|e| e.to_string());
    if let Ok(t) = state.tracker.lock() {
        let _ = activity::finish_attempt(&t.conn, now, error.as_deref());
    }
    match &error {
        None => {
            FAILURES.store(0, Ordering::Relaxed);
            NEXT_AT.store(now + EVERY_MS, Ordering::Relaxed);
        }
        Some(e) => {
            let n = FAILURES.fetch_add(1, Ordering::Relaxed).min(4);
            NEXT_AT.store(now + (RETRY_MIN_MS << n).min(RETRY_MAX_MS), Ordering::Relaxed);
            log::info!("activity not sent ({sent} hours sent first): {e}");
        }
    }
    result.map(|()| sent)
}

/// The store is held only to read and mark the rows, never during the request.
fn send_pending(state: &AppState, s: &Settings, version: &str, sent: &mut usize) -> Result<()> {
    loop {
        let (id, rows) = {
            let t = state.tracker.lock().expect("tracker");
            (activity::install_id(&t.conn)?, activity::pending(&t.conn, activity::BATCH)?)
        };
        let body = activity::payload(&id, version, &rows, now_ms());
        activity::send(&s.activity_endpoint, &s.activity_token, &body)?;
        activity::mark_synced(&state.tracker.lock().expect("tracker").conn, &rows)?;
        *sent += rows.len();
        if rows.len() < activity::BATCH {
            return Ok(());
        }
    }
}
