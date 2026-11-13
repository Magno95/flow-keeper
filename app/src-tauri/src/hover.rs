//! Hover on the companion while another app is active. The webview only
//! tracks the mouse while Flow Keeper is the active app, which it almost
//! never is: a tracking area active at all times tells the companion when
//! the cursor enters its window.

use tauri::AppHandle;

#[cfg(target_os = "macos")]
pub fn track_companion(app: &AppHandle) {
    use objc2::rc::Retained;
    use objc2::runtime::NSObject;
    use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSEvent, NSTrackingArea, NSTrackingAreaOptions, NSView};
    use objc2_foundation::NSRect;
    use tauri::{Emitter, Manager};

    use crate::windows::{COMPANION, EVENT_COMPANION_HOVER};

    define_class!(
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "FlowKeeperCompanionHover"]
        #[ivars = AppHandle]
        struct Tracker;

        impl Tracker {
            #[unsafe(method(mouseEntered:))]
            fn mouse_entered(&self, _event: &NSEvent) {
                let _ = self.ivars().emit_to(COMPANION, EVENT_COMPANION_HOVER, ());
            }

            #[unsafe(method(mouseExited:))]
            fn mouse_exited(&self, _event: &NSEvent) {}
        }
    );

    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("companion hover not tracked: setup outside the main thread");
        return;
    };
    let Some(view) = app.get_webview_window(COMPANION).and_then(|w| w.ns_view().ok()) else {
        return;
    };
    // SAFETY: Tauri returns the window's content view, alive as long as the
    // companion, which is never closed.
    let view = unsafe { &*(view as *const NSView) };
    let tracker = mtm.alloc::<Tracker>().set_ivars(app.clone());
    let tracker: Retained<Tracker> = unsafe { msg_send![super(tracker), init] };
    // The rect follows the view when the window grows for the bubble.
    let options = NSTrackingAreaOptions::MouseEnteredAndExited
        | NSTrackingAreaOptions::ActiveAlways
        | NSTrackingAreaOptions::InVisibleRect;
    let area = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            NSRect::ZERO,
            options,
            Some(&tracker),
            None,
        )
    };
    view.addTrackingArea(&area);
    // The area doesn't retain its owner: it lives as long as the app.
    std::mem::forget(tracker);
}

#[cfg(not(target_os = "macos"))]
pub fn track_companion(_app: &AppHandle) {}
