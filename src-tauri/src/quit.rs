//! Quitting goes through the window, so edits that are not on disk yet are
//! saved, or asked about, before the app ends.
//!
//! The page says two things: that it is listening (`quit_ready`), and whether
//! it holds edits that are not on disk (`quit_guard`). Only when both are true
//! is a quit handed to it, as `quit-requested`; otherwise the app just goes,
//! so a blank window, a page still loading or nothing to save never stands in
//! the way. A page that does not acknowledge within [`ANSWER_WITHIN`] is taken
//! to be gone, and the app goes then.
//!
//! On macOS three things quit: the menu's Quit (⌘Q), the Dock's Quit, and
//! logging out or shutting down. The menu item is ours (built in
//! `lib.rs`); the other two arrive as `applicationShouldTerminate:`, which
//! tao's delegate does not implement, so [`watch_terminate`] adds it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager};

/// How long the page has to say it heard a quit before the app goes anyway.
///
/// A quit is only ever handed over while the page holds unsaved edits, so
/// this is generous: a page busy drawing a large diff answers late, and
/// leaving after three seconds lost what it was holding. It is still a hard
/// ceiling. A page that has said nothing by then is hung or gone, and the app
/// must never be one that cannot be quit.
pub const ANSWER_WITHIN: Duration = Duration::from_secs(10);

/// The event the page answers.
pub const EVENT: &str = "quit-requested";

static READY: AtomicBool = AtomicBool::new(false);
static GUARD: AtomicBool = AtomicBool::new(false);
static EXITING: AtomicBool = AtomicBool::new(false);
static ASKED: AtomicU64 = AtomicU64::new(0);
static HEARD: AtomicU64 = AtomicU64::new(0);

/// The page is listening for quits.
pub fn set_ready(ready: bool) {
    READY.store(ready, Ordering::SeqCst);
}

/// The page holds edits that are not on disk, or not.
pub fn set_guard(needed: bool) {
    GUARD.store(needed, Ordering::SeqCst);
}

/// The page heard quit request `ticket`.
pub fn heard(ticket: u64) {
    HEARD.fetch_max(ticket, Ordering::SeqCst);
}

/// Whether a quit has to go through the page: it is listening, has edits to
/// look after, the window is there, and the app is not already on its way out.
pub fn must_ask(window_open: bool) -> bool {
    !EXITING.load(Ordering::SeqCst)
        && READY.load(Ordering::SeqCst)
        && GUARD.load(Ordering::SeqCst)
        && window_open
}

/// Hands a quit to the page. False when the app should simply go now.
///
/// Every request gets a ticket and a deadline: one the page has not
/// acknowledged by then ends the app, so a hung or blank page can never keep
/// it from quitting. A request the page did acknowledge waits for its answer,
/// which the page gives by closing the window (go) or not (stay).
pub fn ask(app: &AppHandle) -> bool {
    if !must_ask(app.get_webview_window("main").is_some()) {
        return false;
    }
    let ticket = ASKED.fetch_add(1, Ordering::SeqCst) + 1;
    if app.emit(EVENT, ticket).is_err() {
        return false;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(ANSWER_WITHIN);
        if HEARD.load(Ordering::SeqCst) < ticket {
            exit_now(&app);
        }
    });
    true
}

/// Ends the app without asking anything again.
pub fn exit_now(app: &AppHandle) {
    EXITING.store(true, Ordering::SeqCst);
    app.exit(0);
}

/// The menu's Quit and the Dock's: the page decides if it must, else go.
pub fn requested(app: &AppHandle) {
    if !ask(app) {
        exit_now(app);
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::sync::OnceLock;

    use objc2::ffi::{class_addMethod, class_getInstanceMethod, object_getClass};
    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{class, msg_send, sel};
    use tauri::AppHandle;

    static APP: OnceLock<AppHandle> = OnceLock::new();

    const TERMINATE_CANCEL: usize = 0;
    const TERMINATE_NOW: usize = 1;

    /// `-applicationShouldTerminate:`. The Dock's Quit, logout and shutdown.
    /// Cancelled only while the page has edits to look after and is
    /// listening; the page then closes the window, which ends the app. With
    /// nothing unsaved it answers at once, so a logout goes on as usual.
    extern "C-unwind" fn should_terminate(
        _this: *mut AnyObject,
        _cmd: Sel,
        _sender: *mut AnyObject,
    ) -> usize {
        let Some(app) = APP.get() else {
            return TERMINATE_NOW;
        };
        if super::ask(app) {
            TERMINATE_CANCEL
        } else {
            TERMINATE_NOW
        }
    }

    /// Adds `applicationShouldTerminate:` to the app delegate's class, unless
    /// it has one: nothing tao does is replaced.
    pub fn watch_terminate(app: &AppHandle) {
        let _ = APP.set(app.clone());
        // SAFETY: called from setup, on the main thread, after tao has made
        // the application and its delegate. The method is only added where
        // the class has none, with the signature AppKit calls it with:
        // NSApplicationTerminateReply (NSUInteger) from (id, SEL, id).
        unsafe {
            let shared: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            if shared.is_null() {
                return;
            }
            let delegate: *mut AnyObject = msg_send![shared, delegate];
            if delegate.is_null() {
                return;
            }
            let class = object_getClass(delegate) as *mut AnyClass;
            let selector = sel!(applicationShouldTerminate:);
            if class.is_null() || !class_getInstanceMethod(class, selector).is_null() {
                return;
            }
            let imp: Imp = std::mem::transmute::<
                extern "C-unwind" fn(*mut AnyObject, Sel, *mut AnyObject) -> usize,
                Imp,
            >(should_terminate);
            class_addMethod(class, selector, imp, c"Q@:@".as_ptr());
        }
    }
}

#[cfg(target_os = "macos")]
pub use mac::watch_terminate;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_listening_page_with_unsaved_edits_is_asked() {
        set_ready(false);
        set_guard(true);
        assert!(!must_ask(true), "a page not listening yet");
        set_ready(true);
        assert!(must_ask(true));
        assert!(!must_ask(false), "no window");
        set_guard(false);
        assert!(!must_ask(true), "nothing unsaved");
        set_guard(true);
        EXITING.store(true, Ordering::SeqCst);
        assert!(!must_ask(true), "already going");
        EXITING.store(false, Ordering::SeqCst);
        set_ready(false);
        set_guard(false);
    }
}
