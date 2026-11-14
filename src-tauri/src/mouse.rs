//! The mouse's back and forward buttons, on macOS.
//!
//! On Windows and Linux the web view hands buttons 3 and 4 to the page like
//! any other, and the page acts on them. WKWebView does not reliably pass them
//! on, so on macOS they are caught here, before the web view sees them, and sent
//! to the window as a `nav-button` event. The page drops one of the two when a
//! press arrives both ways.

/// Watches this app's own mouse events for the side buttons, for the life of
/// the app. A local monitor sees only events going to this app's windows, and
/// needs no accessibility permission.
#[cfg(target_os = "macos")]
pub fn watch_side_buttons(app: &tauri::AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask};
    use std::ptr::NonNull;
    use tauri::Emitter;

    let handle = app.clone();
    let block = RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit hands the monitor a live event for the length of
        // the call.
        let button = unsafe { event.as_ref() }.buttonNumber();
        if let Some(direction) = direction(button) {
            let _ = handle.emit("nav-button", direction);
        }
        // Passed on unchanged: the page may see it too, and that is fine.
        event.as_ptr()
    });
    // SAFETY: the block returns the event it was given, which is valid.
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::OtherMouseDown, &block)
    };
    // The monitor holds its own copy of the block. It is never removed: the
    // buttons are wanted for as long as the app runs.
    std::mem::forget(monitor);
}

#[cfg(not(target_os = "macos"))]
pub fn watch_side_buttons(_app: &tauri::AppHandle) {}

/// Which way a button number points, the way browsers number them: 3 is
/// back, 4 is forward.
pub fn direction(button: isize) -> Option<&'static str> {
    match button {
        3 => Some("back"),
        4 => Some("forward"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::direction;

    #[test]
    fn only_the_two_side_buttons_navigate() {
        assert_eq!(direction(3), Some("back"));
        assert_eq!(direction(4), Some("forward"));
        assert_eq!(direction(0), None);
        assert_eq!(direction(2), None);
    }
}
