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

/// Who copied something: enough to name and picture them in the history.
///
/// The frontmost app at the moment the pasteboard changed. That is an
/// inference, not a fact the pasteboard carries — macOS records no owner — but
/// it is the same inference every clipboard manager makes, and it is right
/// except when you copy and switch apps inside the same half second.
pub struct FrontApp {
    pub pid: i32,
    /// Empty for the rare process without one, e.g. a bare executable.
    pub bundle_id: String,
    pub name: String,
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
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// That app's icon as PNG bytes, at roughly badge size.
///
/// Separate from [`frontmost_app`] because it is the expensive half — a decode
/// and a re-encode — and the caller only needs it the first time it sees a
/// given app.
pub fn app_icon_png(pid: i32) -> Option<Vec<u8>> {
    #[cfg(target_os = "macos")]
    {
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
    #[cfg(not(target_os = "macos"))]
    {
        let _ = pid;
        None
    }
}
