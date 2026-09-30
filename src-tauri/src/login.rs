//! Launching at login.
//!
//! macOS 13 replaced every older way of doing this — the deprecated
//! `SMLoginItemSetEnabled`, AppleScript against System Events, and hand-written
//! `~/Library/LaunchAgents` plists — with `SMAppService`. It is the only one
//! that needs no second permission prompt (AppleScript would ask for Automation
//! access, on top of the Accessibility grant this app already needs) and the
//! only one that appears where a user would look for it: System Settings →
//! General → Login Items → **Open at Login**.
//!
//! The system owns this setting, not `macros.json`. A checkbox that remembered
//! its own answer would start lying the moment the user unticked it in System
//! Settings, so the state is always read back from the OS — the same way the
//! Accessibility banner works.

/// Whether Reze starts at login, as the system sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LoginItem {
    Enabled,
    Disabled,
    /// Registered, but switched off by the user in System Settings. Only they
    /// can undo that, so the UI has to say so rather than quietly re-register.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    RequiresApproval,
    /// Nothing here can do it: an unsupported OS, older than macOS 13, or a dev
    /// build rather than an installed app.
    Unsupported,
}

#[cfg(target_os = "macos")]
mod imp {
    use super::LoginItem;
    use objc2::runtime::AnyClass;
    use objc2_service_management::{SMAppService, SMAppServiceStatus};

    /// `SMAppService` arrived in macOS 13 and the bundle still declares 10.15 as
    /// its minimum, so the class can genuinely be missing at runtime. objc2
    /// panics on a class it cannot find, so this is checked rather than caught.
    fn available() -> bool {
        AnyClass::get(c"SMAppService").is_some()
    }

    /// `SMAppService` registers a *bundle*. Under `tauri dev` the executable is
    /// a bare binary with no bundle around it, and the error macOS returns for
    /// that reads like a permission problem — so rule it out by name instead.
    fn is_bundled() -> bool {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.ends_with("Contents/MacOS")))
            .unwrap_or(false)
    }

    pub fn status() -> LoginItem {
        if !available() || !is_bundled() {
            return LoginItem::Unsupported;
        }
        // SAFETY: the class exists, and neither selector takes an argument.
        match unsafe { SMAppService::mainAppService().status() } {
            SMAppServiceStatus::Enabled => LoginItem::Enabled,
            SMAppServiceStatus::RequiresApproval => LoginItem::RequiresApproval,
            // NotRegistered, plus NotFound — a registration that outlived the
            // bundle it pointed at, which is what a reinstall leaves behind.
            _ => LoginItem::Disabled,
        }
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        if !available() {
            return Err("Launching at login needs macOS 13 or later.".into());
        }
        if !is_bundled() {
            return Err(
                "Only an installed Reze.app can register itself to launch at login — \
                 a dev build has no bundle for macOS to start. Run `make` first."
                    .into(),
            );
        }

        // Asking for the state it is already in is not an error worth showing,
        // and macOS does report one (`kSMErrorAlreadyRegistered`) for it.
        match (enabled, status()) {
            (true, LoginItem::Enabled) | (false, LoginItem::Disabled) => return Ok(()),
            (true, LoginItem::RequiresApproval) => {
                return Err(
                    "Reze is registered, but login items are switched off for it in \
                     System Settings. Only you can turn that back on."
                        .into(),
                )
            }
            _ => {}
        }

        let service = unsafe { SMAppService::mainAppService() };
        let outcome = if enabled {
            unsafe { service.registerAndReturnError() }
        } else {
            unsafe { service.unregisterAndReturnError() }
        };

        outcome.map_err(|e| {
            let verb = if enabled { "register" } else { "unregister" };
            format!("Could not {verb} Reze as a login item: {}", e.localizedDescription())
        })
    }

    /// Opens System Settings straight to Login Items — Apple ships this exact
    /// call for the case where an app needs the user to flip the switch itself.
    pub fn open_settings() {
        if available() {
            unsafe { SMAppService::openSystemSettingsLoginItems() };
        }
    }
}

/// An XDG autostart entry: a desktop file in `~/.config/autostart`, which
/// GNOME, KDE and every other freedesktop session launches at login. The file
/// is the whole of the state — it exists or it does not — so, as on macOS,
/// the answer is read back from where the session reads it, and a user who
/// deletes it or switches it off in GNOME Tweaks is believed.
#[cfg(target_os = "linux")]
mod imp {
    use super::LoginItem;
    use std::path::PathBuf;

    fn entry_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap_or_default().join(".config"))
            .join("autostart")
            .join("reze.desktop")
    }

    /// The binary to start. A debug build is refused for the same reason macOS
    /// refuses one without a bundle: it lives in `target/`, and the next
    /// `cargo clean` would leave a login entry pointing at nothing.
    fn installed_exe() -> Option<PathBuf> {
        if cfg!(debug_assertions) {
            return None;
        }
        std::env::current_exe().ok()
    }

    /// Switched off without being removed: `Hidden=true` is the spec's way,
    /// and GNOME writes its own key instead.
    fn switched_off(body: &str) -> bool {
        body.lines().map(str::trim).any(|l| {
            l.eq_ignore_ascii_case("Hidden=true")
                || l.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
        })
    }

    pub fn status() -> LoginItem {
        if installed_exe().is_none() {
            return LoginItem::Unsupported;
        }
        match std::fs::read_to_string(entry_path()) {
            Ok(body) if !switched_off(&body) => LoginItem::Enabled,
            _ => LoginItem::Disabled,
        }
    }

    pub fn set(enabled: bool) -> Result<(), String> {
        let path = entry_path();
        if !enabled {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    Err(format!("Could not remove {}: {e}", path.display()))
                }
                _ => Ok(()),
            };
        }
        let exe = installed_exe().ok_or(
            "Only an installed Reze can start itself at login — a dev build lives in \
             target/ and would be gone after the next clean. Run `make` first.",
        )?;
        // Quoted per the desktop entry spec, which reserves these four inside quotes.
        let mut quoted = String::new();
        for c in exe.to_string_lossy().chars() {
            if matches!(c, '"' | '`' | '$' | '\\') {
                quoted.push('\\');
            }
            quoted.push(c);
        }
        let body = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name=Reze\n\
             Comment=Turn short triggers into full prompts\n\
             Exec=\"{quoted}\"\n\
             Icon=reze\n\
             Terminal=false\n\
             X-GNOME-Autostart-enabled=true\n"
        );
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
        }
        std::fs::write(&path, body).map_err(|e| format!("Could not write {}: {e}", path.display()))
    }

    /// No settings pane to open: GNOME keeps login apps in Tweaks, which may
    /// not be installed, and the switch in Reze is the same file anyway.
    pub fn open_settings() {}

    #[cfg(test)]
    mod tests {
        #[test]
        fn a_switched_off_entry_reads_as_off() {
            assert!(super::switched_off("[Desktop Entry]\nHidden=true\n"));
            assert!(super::switched_off("X-GNOME-Autostart-enabled=false"));
            assert!(!super::switched_off("[Desktop Entry]\nX-GNOME-Autostart-enabled=true\n"));
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    use super::LoginItem;

    /// Nothing implemented here. Reporting `Unsupported` keeps the UI honest,
    /// rather than showing a switch that does nothing.
    pub fn status() -> LoginItem {
        LoginItem::Unsupported
    }

    pub fn set(_enabled: bool) -> Result<(), String> {
        Err("Launching at login is only implemented on macOS and Linux.".into())
    }

    pub fn open_settings() {}
}

pub use imp::{open_settings, set, status};
