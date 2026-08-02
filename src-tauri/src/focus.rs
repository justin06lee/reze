//! Keeping track of who had focus, and knowing when a copy actually happened.
//!
//! Showing the palette deactivates whatever app the user was typing in. macOS
//! usually reactivates it when we hide again, but not reliably in time — and a
//! `Cmd+V` synthesized while nothing is frontmost is simply swallowed, which
//! looks exactly like the paste silently failing. So we remember the app
//! ourselves and put it back deliberately before pressing anything.

/// PID of the frontmost application, or `None` if that cannot be determined.
pub fn frontmost_pid() -> Option<i32> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        Some(app.processIdentifier())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Bring `pid` back to the front. Returns whether the activation was accepted.
pub fn activate_pid(pid: i32) -> bool {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
        let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) else {
            return false;
        };
        #[allow(deprecated)]
        app.activateWithOptions(NSApplicationActivationOptions::ActivateAllWindows)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        false
    }
}

/// Monotonic counter that macOS bumps on every pasteboard write, or `None`
/// where no such counter exists.
///
/// Comparing it before and after a synthesized copy is the only honest way to
/// tell "the user had nothing selected" from "the copy worked and happened to
/// produce the same text" — reading the clipboard alone cannot distinguish
/// them, and a stale read would match against whatever was copied hours ago.
///
/// Callers that get `None` should fall back to [`crate::paste::write_sentinel`]:
/// park a value on the clipboard that nothing else could produce, and treat it
/// still being there as proof that nothing was copied.
pub fn pasteboard_change_count() -> Option<isize> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSPasteboard;
        Some(NSPasteboard::generalPasteboard().changeCount())
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}
