//! Keeping track of who had focus, and knowing when a copy actually happened.
//!
//! Showing the palette deactivates whatever app the user was typing in. macOS
//! usually reactivates it when we hide again, but not reliably in time — and a
//! `Cmd+V` synthesized while nothing is frontmost is simply swallowed, which
//! looks exactly like the paste silently failing. So we remember the app
//! ourselves and put it back deliberately before pressing anything.

/// Whatever identifies "where the user was": a process on macOS, where apps
/// are activated as a whole, and a top-level window on X11, where focus
/// belongs to windows — see [`crate::x11`].
#[cfg(target_os = "macos")]
pub type Target = i32;
#[cfg(not(target_os = "macos"))]
pub type Target = u32;

/// Where the user is working right now, if that is somewhere other than Reze.
///
/// Our own windows are skipped: opening the palette twice in a row must not
/// record the palette as the place to paste into.
pub fn frontmost_elsewhere() -> Option<Target> {
    #[cfg(target_os = "macos")]
    {
        let ours = std::process::id() as i32;
        frontmost_pid().filter(|pid| *pid != ours)
    }
    #[cfg(target_os = "linux")]
    {
        crate::x11::active_window()
            .filter(|w| crate::x11::window_pid(*w) != Some(std::process::id()))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Where the user is working right now, even if that is one of our windows.
pub fn frontmost() -> Option<Target> {
    #[cfg(target_os = "macos")]
    {
        frontmost_pid()
    }
    #[cfg(target_os = "linux")]
    {
        crate::x11::active_window()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Bring `target` back to the front. Returns whether that was accepted.
pub fn activate(target: Target) -> bool {
    #[cfg(target_os = "macos")]
    {
        activate_pid(target)
    }
    #[cfg(target_os = "linux")]
    {
        crate::x11::activate(target)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = target;
        false
    }
}

/// Whether the focused window is a terminal emulator.
///
/// Always `false` on macOS, where it makes no difference: ⌘V pastes and ⌘C
/// copies in a terminal like anywhere else. On Linux both differ — see
/// [`crate::x11::is_terminal`].
pub fn frontmost_is_terminal() -> bool {
    #[cfg(target_os = "linux")]
    {
        crate::x11::active_window().is_some_and(crate::x11::is_terminal)
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

/// Let go of the hotkey before any keystrokes are synthesized. Only X11 needs
/// this — see [`crate::x11::wait_for_keys_released`].
pub fn wait_for_hotkey_release() {
    #[cfg(target_os = "linux")]
    crate::x11::wait_for_keys_released(std::time::Duration::from_secs(1));
}

/// PID of the frontmost application, or `None` if that cannot be determined.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
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
    // X11 has no counter, but it does announce every change of clipboard
    // owner, and [`crate::clip`] counts those.
    #[cfg(target_os = "linux")]
    {
        crate::clip::change_count()
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// Who copied something: enough to name and picture them in the history.
///
/// The frontmost app at the moment the pasteboard changed. That is an
/// inference, not a fact the pasteboard carries — macOS records no owner — but
/// it is the same inference every clipboard manager makes, and it is right
/// except when you copy and switch apps inside the same half second.
pub struct FrontApp {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub pid: i32,
    /// Empty for the rare process without one, e.g. a bare executable. On
    /// Linux, the app's desktop entry id, or failing that its `WM_CLASS`.
    pub bundle_id: String,
    pub name: String,
    /// The X11 window it was read from, which is where its icon lives. Zero on
    /// macOS, where the icon belongs to the process.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub window: u32,
}

pub fn frontmost_app() -> Option<FrontApp> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;
        let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
        Some(FrontApp {
            pid: app.processIdentifier(),
            bundle_id: app.bundleIdentifier().map(|s| s.to_string()).unwrap_or_default(),
            name: app.localizedName().map(|s| s.to_string()).unwrap_or_default(),
            window: 0,
        })
    }
    #[cfg(target_os = "linux")]
    {
        let window = crate::x11::active_window()?;
        let (instance, class) = crate::x11::window_class(window).unwrap_or_default();
        let (bundle_id, name) = crate::x11::app_identity(&instance, &class).unwrap_or_else(|| {
            let id = if instance.is_empty() { class.clone() } else { instance.clone() };
            let name = if class.is_empty() { instance.clone() } else { class.clone() };
            (id, name)
        });
        Some(FrontApp {
            pid: crate::x11::window_pid(window).map(|p| p as i32).unwrap_or(0),
            bundle_id,
            name,
            window,
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        None
    }
}

/// That app's icon as PNG bytes, at roughly badge size.
///
/// Separate from [`frontmost_app`] because it is the expensive half — a decode
/// and a re-encode — and the caller only needs it the first time it sees a
/// given app.
pub fn app_icon_png(front: &FrontApp) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
        let pid = front.pid;
        use objc2::rc::Retained;
        use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSRunningApplication};
        use objc2_foundation::NSDictionary;

        /// Smallest icon worth caching. The badge is drawn at 16pt, so this
        /// covers a 2× display with room to spare — and an app icon carries a
        /// 1024px representation whose PNG runs to well over a megabyte, which
        /// is an absurd thing to keep one of per app for a 16-pixel badge.
        const BADGE_PIXELS: isize = 64;

        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid)?;
        let icon = app.icon()?;
        // NSImage holds representations, not bytes. TIFF is the one format it
        // will always hand back, and it hands back *all* of them — which is
        // what makes it possible to choose a small one.
        let tiff = icon.TIFFRepresentation()?;

        let mut chosen: Option<Retained<NSBitmapImageRep>> = None;
        {
            let reps = NSBitmapImageRep::imageRepsWithData(&tiff);
            for rep in reps.iter() {
                let Some(bitmap) = rep.downcast_ref::<NSBitmapImageRep>() else {
                    continue;
                };
                let wide = bitmap.pixelsWide();
                // The smallest that is still big enough; failing that, the
                // biggest available, for an icon with nothing above 64px.
                let better = match &chosen {
                    None => true,
                    Some(best) => match (wide >= BADGE_PIXELS, best.pixelsWide() >= BADGE_PIXELS) {
                        (true, true) => wide < best.pixelsWide(),
                        (true, false) => true,
                        (false, true) => false,
                        (false, false) => wide > best.pixelsWide(),
                    },
                };
                if better {
                    chosen = Some(Retained::from(bitmap));
                }
            }
        }
        // A TIFF that would not decompose into representations still encodes
        // whole, so fall back to that rather than losing the icon.
        let rep = match chosen {
            Some(rep) => rep,
            None => NSBitmapImageRep::imageRepWithData(&tiff)?,
        };

        let png = unsafe {
            rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
        }?;
        Some(png.to_vec())
    }
    // The window's own icon property. The badge is drawn at 16pt, so 64px
    // covers a 2× display the same as on macOS.
    #[cfg(target_os = "linux")]
    {
        crate::x11::window_icon_png(front.window, 64)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = front;
        None
    }
}
