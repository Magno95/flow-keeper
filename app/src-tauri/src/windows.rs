//! Windows, menu bar and shortcut. The companion never takes focus
//! while updating; the panel opens next to the companion while staying
//! inside the visible area of the current monitor.

use std::sync::atomic::{AtomicI64, Ordering};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, Window, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tracker_core::store;

use crate::AppState;

pub const COMPANION: &str = "companion";
pub const PANEL: &str = "panel";
pub const EVENT_NAVIGATE: &str = "tracker://navigate";
pub const EVENT_PANEL_OPENED: &str = "tracker://panel-opened";
pub const EVENT_PANEL_CLOSED: &str = "tracker://panel-closed";

/// Direction the companion looks toward the panel.
#[derive(Clone, serde::Serialize)]
pub struct Gaze {
    pub vertical: &'static str,
    pub side: &'static str,
}

/// When the panel was hidden by losing focus: a click on the companion
/// right after must not reopen it.
static BLUR_HIDDEN_AT: AtomicI64 = AtomicI64::new(0);

const GAP: f64 = 10.0;
const MARGIN: f64 = 12.0;
/// Companion size from `tauri.conf.json`, without extra space above.
const COMPANION_SIZE: (f64, f64) = (112.0, 96.0);

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Flow Keeper", true, None::<&str>)?;
    let companion = MenuItem::with_id(app, "companion", "Show or Hide FloK", true, None::<&str>)?;
    let doctor = MenuItem::with_id(app, "doctor", "Check Connections…", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Flow Keeper", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &companion, &doctor, &sep, &quit])?;
    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Flow Keeper")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_panel(app),
            "companion" => {
                let visible = app.get_webview_window(COMPANION).and_then(|w| w.is_visible().ok()).unwrap_or(false);
                set_companion_visible(app, !visible);
                let state = app.state::<AppState>();
                let mut s = state.settings.lock().expect("settings");
                s.companion_visible = !visible;
                let tracker = state.tracker.lock().expect("tracker");
                let _ = tracker_core::settings::save(&tracker.conn, &s);
            }
            "doctor" => {
                show_panel(app);
                let _ = app.emit_to(PANEL, EVENT_NAVIGATE, "doctor");
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle_panel(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn register_shortcut(app: &AppHandle, shortcut: &str) {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    if shortcut.trim().is_empty() {
        return;
    }
    let result = gs.on_shortcut(shortcut, |app, _shortcut, event| {
        if event.state == ShortcutState::Pressed {
            toggle_panel(app);
        }
    });
    if let Err(err) = result {
        log::warn!("shortcut not registered ({shortcut}): {err}");
    }
}

pub fn set_companion_visible(app: &AppHandle, visible: bool) {
    if let Some(w) = app.get_webview_window(COMPANION) {
        let _ = if visible { w.show() } else { w.hide() };
    }
}

/// Extra space above the companion (logical px) for the update speech
/// bubble. The window grows upward so the robot stays put; near the top
/// edge of the screen it moves down as much as needed.
pub fn set_companion_headroom(app: &AppHandle, extra: f64) {
    let Some(w) = app.get_webview_window(COMPANION) else { return };
    let state = app.state::<AppState>();
    let mut headroom = state.companion_headroom.lock().expect("headroom");
    let extra = extra.clamp(0.0, 400.0).round();
    if (headroom.0 - extra).abs() < 0.5 {
        return;
    }
    let (Ok(pos), Ok(scale)) = (w.outer_position(), w.scale_factor()) else { return };
    // Top corner of the window without the extra space.
    let base_y = pos.y as f64 + headroom.1;
    let mut shift = extra * scale;
    if let Ok(Some(m)) = w.current_monitor() {
        shift = shift.min((base_y - m.work_area().position.y as f64).max(0.0));
    }
    *headroom = (extra, shift);
    let _ = w.set_size(LogicalSize::new(COMPANION_SIZE.0, COMPANION_SIZE.1 + extra));
    let _ = w.set_position(PhysicalPosition::new(pos.x as f64, base_y - shift));
}

/// Saved companion position, if still inside a monitor; otherwise the
/// bottom right of the main monitor.
pub fn restore_companion(app: &AppHandle) {
    let Some(w) = app.get_webview_window(COMPANION) else { return };
    let state = app.state::<AppState>();
    let saved = {
        let tracker = state.tracker.lock().expect("tracker");
        store::meta(&tracker.conn, "companion_pos").ok().flatten()
    };
    let size = w.outer_size().unwrap_or_default();
    let saved = saved.and_then(|v| {
        let (x, y) = v.split_once(',')?;
        Some((x.parse::<f64>().ok()?, y.parse::<f64>().ok()?))
    });
    let monitors = w.available_monitors().unwrap_or_default();
    if let Some((x, y)) = saved {
        let inside = monitors.iter().any(|m| {
            let a = m.work_area();
            x >= a.position.x as f64 - 1.0
                && y >= a.position.y as f64 - 1.0
                && x + size.width as f64 <= (a.position.x + a.size.width as i32) as f64 + 1.0
                && y + size.height as f64 <= (a.position.y + a.size.height as i32) as f64 + 1.0
        });
        if inside {
            let _ = w.set_position(PhysicalPosition::new(x, y));
            return;
        }
    }
    if let Ok(Some(m)) = w.primary_monitor() {
        let a = m.work_area();
        let scale = m.scale_factor();
        let x = (a.position.x + a.size.width as i32) as f64 - size.width as f64 - 24.0 * scale;
        let y = (a.position.y + a.size.height as i32) as f64 - size.height as f64 - 24.0 * scale;
        let _ = w.set_position(PhysicalPosition::new(x, y));
    }
}

pub fn toggle_panel(app: &AppHandle) {
    let Some(panel) = app.get_webview_window(PANEL) else { return };
    if panel.is_visible().unwrap_or(false) {
        hide_panel(app);
    } else if tracker_core::now_ms() - BLUR_HIDDEN_AT.load(Ordering::Relaxed) > 350 {
        show_panel(app);
    }
}

pub fn show_panel(app: &AppHandle) {
    let Some(panel) = app.get_webview_window(PANEL) else { return };
    let gaze = position_panel(app);
    let _ = panel.show();
    let _ = panel.set_focus();
    let _ = app.emit(EVENT_PANEL_OPENED, gaze);
}

pub fn hide_panel(app: &AppHandle) {
    if let Some(panel) = app.get_webview_window(PANEL) {
        let _ = panel.hide();
    }
    let _ = app.emit(EVENT_PANEL_CLOSED, ());
}

/// Positions the panel and returns where it sits relative to the companion.
fn position_panel(app: &AppHandle) -> Option<Gaze> {
    let panel = app.get_webview_window(PANEL)?;
    let companion = app.get_webview_window(COMPANION).filter(|c| c.is_visible().unwrap_or(false));
    let anchor = companion.as_ref().and_then(|c| Some((c.outer_position().ok()?, c.outer_size().ok()?, c.current_monitor().ok()??)));
    let psize = panel.outer_size().unwrap_or_default();
    let (pw, ph) = (psize.width as f64, psize.height as f64);

    let mut gaze = None;
    let (x, y) = match anchor {
        Some((pos, size, monitor)) => {
            let a = monitor.work_area();
            let scale = monitor.scale_factor();
            let (ax, ay, aw, ah) = (a.position.x as f64, a.position.y as f64, a.size.width as f64, a.size.height as f64);
            let (cx, cy, cw, ch) = (pos.x as f64, pos.y as f64, size.width as f64, size.height as f64);
            let gap = GAP * scale;
            let margin = MARGIN * scale;
            let right_space = ax + aw - (cx + cw);
            let x = if right_space >= pw + gap + margin { cx + cw + gap } else { cx - pw - gap };
            let y = cy + ch - ph;
            let x = x.clamp(ax + margin, (ax + aw - pw - margin).max(ax + margin));
            let y = y.clamp(ay + margin, (ay + ah - ph - margin).max(ay + margin));
            // The center of the panel's content relative to the companion's face.
            let (fx, fy) = (cx + cw / 2.0, cy + ch / 2.0);
            let (px, py) = (x + pw / 2.0, y + ph / 2.0);
            let side = if (px - fx).abs() < cw { "center" } else if px > fx { "right" } else { "left" };
            gaze = Some(Gaze { vertical: if py < fy { "up" } else { "down" }, side });
            (x, y)
        }
        None => {
            // Without the companion: below the menu bar, on the right.
            let monitor = panel.current_monitor().ok().flatten().or_else(|| panel.primary_monitor().ok().flatten());
            match monitor {
                Some(m) => {
                    let a = m.work_area();
                    let scale = m.scale_factor();
                    ((a.position.x + a.size.width as i32) as f64 - pw - MARGIN * scale, a.position.y as f64 + 6.0 * scale)
                }
                None => (100.0, 100.0),
            }
        }
    };
    let _ = panel.set_position(PhysicalPosition::new(x, y));
    gaze
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let app = window.app_handle();
    match (window.label(), event) {
        (COMPANION, WindowEvent::Moved(pos)) => {
            let state = app.state::<AppState>();
            // Save the position without the speech bubble's space.
            let shift = state.companion_headroom.lock().expect("headroom").1;
            *state.companion_pos.lock().expect("pos") = Some((pos.x as f64, pos.y as f64 + shift));
        }
        (PANEL, WindowEvent::Focused(false)) => {
            let state = app.state::<AppState>();
            if !state.dialog_open.load(Ordering::Relaxed) && window.is_visible().unwrap_or(false) {
                BLUR_HIDDEN_AT.store(tracker_core::now_ms(), Ordering::Relaxed);
                let _ = window.hide();
                let _ = app.emit(EVENT_PANEL_CLOSED, ());
            }
        }
        (label, WindowEvent::CloseRequested { api, .. }) => {
            api.prevent_close();
            let _ = window.hide();
            if label == PANEL {
                let _ = app.emit(EVENT_PANEL_CLOSED, ());
            }
        }
        _ => {}
    }
}
