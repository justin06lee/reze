//! The X11 side of the Linux port: who has focus, handing it back, what that
//! app is called and what its icon looks like, and whether it is a terminal.
//!
//! macOS answers all of this through `NSWorkspace`. X11 answers it through the
//! properties a window manager keeps on its windows (EWMH: `_NET_ACTIVE_WINDOW`,
//! `_NET_WM_PID`, `_NET_WM_ICON`) — every desktop Reze targets maintains them,
//! GNOME and KDE and the small ones alike. Wayland exposes none of it, which is
//! part of why Reze does not support it.
//!
//! The unit of focus here is a *window*, not an app: X11 activates windows, and
//! handing focus back to "the app" would pick whichever of its windows the
//! window manager fancied.

use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt as _, EventMask, Window};
use x11rb::rust_connection::RustConnection;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_ACTIVE_WINDOW,
        _NET_WM_PID,
        _NET_WM_ICON,
    }
}

struct Display {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
}

/// One connection for every query, opened on first use. `None` when there is
/// no X server to talk to — a Wayland session without XWayland, or no display
/// at all — in which case every question below answers "don't know".
static DISPLAY: OnceLock<Option<Display>> = OnceLock::new();

fn display() -> Option<&'static Display> {
    DISPLAY
        .get_or_init(|| {
            let (conn, screen) = x11rb::connect(None).ok()?;
            let root = conn.setup().roots.get(screen)?.root;
            let atoms = Atoms::new(&conn).ok()?.reply().ok()?;
            Some(Display { conn, root, atoms })
        })
        .as_ref()
}

fn property32(window: Window, property: u32, kind: AtomEnum, len: u32) -> Option<Vec<u32>> {
    let d = display()?;
    let reply = d
        .conn
        .get_property(false, window, property, kind, 0, len)
        .ok()?
        .reply()
        .ok()?;
    let values = reply.value32()?.collect();
    Some(values)
}

/// The window the user is working in.
pub fn active_window() -> Option<Window> {
    let d = display()?;
    let w = *property32(d.root, d.atoms._NET_ACTIVE_WINDOW, AtomEnum::WINDOW, 1)?.first()?;
    (w != 0).then_some(w)
}

pub fn window_pid(window: Window) -> Option<u32> {
    let d = display()?;
    property32(window, d.atoms._NET_WM_PID, AtomEnum::CARDINAL, 1)?
        .first()
        .copied()
}

/// `WM_CLASS`: the instance and class names, e.g. `("gnome-terminal-server",
/// "Gnome-terminal")`. The closest thing X11 has to a bundle identifier.
pub fn window_class(window: Window) -> Option<(String, String)> {
    let d = display()?;
    let reply = d
        .conn
        .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::STRING, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    let mut parts = reply
        .value
        .split(|b| *b == 0)
        .map(|s| String::from_utf8_lossy(s).to_string());
    let instance = parts.next().unwrap_or_default();
    let class = parts.next().unwrap_or_default();
    (!instance.is_empty() || !class.is_empty()).then_some((instance, class))
}

/// Ask the window manager to focus `window`, and wait until it has.
///
/// Sent as a *pager* request (source indication 2). An ordinary application
/// asking for focus is exactly what focus-stealing prevention exists to refuse;
/// a pager is the user's own tool acting on their behalf, which is what this is,
/// and window managers honour it.
///
/// Asked more than once: a window that is still being mapped when the first
/// request arrives is ignored by the window manager, not queued.
pub fn activate(window: Window) -> bool {
    let Some(d) = display() else {
        return false;
    };
    for _ in 0..6 {
        let event =
            ClientMessageEvent::new(32, window, d.atoms._NET_ACTIVE_WINDOW, [2u32, 0, 0, 0, 0]);
        let sent = d.conn.send_event(
            false,
            d.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            event,
        );
        if sent.is_err() || d.conn.flush().is_err() {
            return false;
        }
        for _ in 0..5 {
            if active_window() == Some(window) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    false
}

/// Wait for the user to let go of the hotkey — every key of it.
///
/// Two reasons, both X11's. XTEST events combine with whatever is physically
/// held, so a Backspace sent while the expand hotkey's Control is still down
/// deletes a whole word (on macOS a synthesized keystroke carries its own
/// modifier flags). And a global hotkey is a passive grab that stays active
/// until its key comes back up: anything synthesized before then is delivered
/// to the hotkey's grab window rather than to the app being typed in, and
/// silently does nothing. Giving up after `limit` rather than waiting forever —
/// someone holding a key down that long is doing something else.
pub fn wait_for_keys_released(limit: Duration) {
    let Some(d) = display() else {
        return;
    };
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        let Some(reply) = d.conn.query_keymap().ok().and_then(|c| c.reply().ok()) else {
            return;
        };
        if reply.keys.iter().all(|byte| *byte == 0) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The window's own icon, as a PNG at least `want` pixels wide where it has one
/// that big, else the biggest it has.
///
/// `_NET_WM_ICON` is a flat list of `width, height, pixels…` entries in
/// non-premultiplied ARGB, one per size the app provides.
pub fn window_icon_png(window: Window, want: u32) -> Option<Vec<u8>> {
    let d = display()?;
    // Enough for a 1024² icon plus smaller ones; the server clamps to what is there.
    let data = property32(window, d.atoms._NET_WM_ICON, AtomEnum::CARDINAL, 4 << 20)?;

    let mut best: Option<(u32, u32, &[u32])> = None;
    let mut rest = data.as_slice();
    while rest.len() >= 2 {
        let (w, h) = (rest[0], rest[1]);
        let n = (w as usize).checked_mul(h as usize)?;
        if w == 0 || h == 0 || rest.len() < 2 + n {
            break;
        }
        let pixels = &rest[2..2 + n];
        rest = &rest[2 + n..];
        let better = match best {
            None => true,
            Some((bw, _, _)) => match (w >= want, bw >= want) {
                (true, true) => w < bw,
                (true, false) => true,
                (false, true) => false,
                (false, false) => w > bw,
            },
        };
        if better {
            best = Some((w, h, pixels));
        }
    }

    let (w, h, pixels) = best?;
    let rgba: Vec<u8> = pixels
        .iter()
        .flat_map(|p| {
            let [b, g, r, a] = p.to_le_bytes();
            [r, g, b, a]
        })
        .collect();
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgba).ok()?;
    }
    Some(out)
}

/// Whether `window` belongs to a terminal emulator.
///
/// Matters twice over on Linux, where it did not on macOS. A terminal pastes on
/// Ctrl+Shift+V — plain Ctrl+V is a literal ^V to the shell. And Ctrl+C is not
/// copy but SIGINT, so the select-and-copy fallback that reads text in front of
/// the caret would kill whatever is running in the foreground.
pub fn is_terminal(window: Window) -> bool {
    let Some((instance, class)) = window_class(window) else {
        return false;
    };
    [instance, class]
        .iter()
        .any(|name| names_terminal(&name.to_lowercase()))
}

/// Terminal emulators by `WM_CLASS`. Most say "term" somewhere in the name
/// (xterm, gnome-terminal, konsole's cousins, wezterm, xfce4-terminal…); the
/// list is the ones that don't.
fn names_terminal(name: &str) -> bool {
    const OTHERS: [&str; 17] = [
        "kitty", "alacritty", "konsole", "tilix", "foot", "st", "st-256color", "urxvt", "kgx",
        "org.gnome.console", "ptyxis", "org.gnome.ptyxis", "guake", "tilda", "yakuake",
        "com.mitchellh.ghostty", "ghostty",
    ];
    name.contains("term") || OTHERS.contains(&name) || name.contains("blackbox")
}

// ------------------------------------------------------------ app names

/// What a window's app calls itself, from its desktop entry: `("google-chrome",
/// "Google Chrome")` rather than the bare `WM_CLASS` of "Google-chrome".
///
/// The desktop entries are read once. An app installed while Reze is running
/// shows under its class name until the next launch, which is a cosmetic cost
/// against rescanning a few hundred files on every copy.
pub fn app_identity(instance: &str, class: &str) -> Option<(String, String)> {
    static ENTRIES: OnceLock<Vec<DesktopEntry>> = OnceLock::new();
    let entries = ENTRIES.get_or_init(desktop_entries);

    let eq = |a: &str, b: &str| !a.is_empty() && a.eq_ignore_ascii_case(b);
    let found = entries
        .iter()
        .find(|e| e.wm_class.as_deref().is_some_and(|c| eq(c, class) || eq(c, instance)))
        .or_else(|| entries.iter().find(|e| eq(&e.id, instance) || eq(&e.id, class)))
        .or_else(|| {
            // `org.gnome.Nautilus` for a class of `Nautilus`.
            entries.iter().find(|e| {
                let tail = e.id.rsplit('.').next().unwrap_or(&e.id);
                eq(tail, instance) || eq(tail, class)
            })
        })?;
    Some((found.id.clone(), found.name.clone()))
}

struct DesktopEntry {
    id: String,
    name: String,
    wm_class: Option<String>,
}

fn desktop_entries() -> Vec<DesktopEntry> {
    let home = dirs::home_dir().unwrap_or_default();
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    let data_dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());

    let mut dirs = vec![data_home.join("applications")];
    dirs.extend(data_dirs.split(':').map(|d| PathBuf::from(d).join("applications")));
    dirs.push(PathBuf::from("/var/lib/snapd/desktop/applications"));
    dirs.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
    dirs.push(home.join(".local/share/flatpak/exports/share/applications"));

    let mut out = Vec::new();
    for dir in dirs {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for file in read.flatten() {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            let Some(id) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else {
                continue;
            };
            let Ok(body) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(entry) = parse_desktop_entry(id, &body) {
                out.push(entry);
            }
        }
    }
    out
}

/// The unlocalised `Name` and the `StartupWMClass` of the `[Desktop Entry]`
/// group — not of the `[Desktop Action …]` groups that follow it.
fn parse_desktop_entry(id: String, body: &str) -> Option<DesktopEntry> {
    let mut in_main = false;
    let mut name = None;
    let mut wm_class = None;
    for line in body.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main {
            continue;
        }
        if let Some(v) = line.strip_prefix("Name=") {
            name.get_or_insert_with(|| v.to_string());
        } else if let Some(v) = line.strip_prefix("StartupWMClass=") {
            wm_class.get_or_insert_with(|| v.to_string());
        }
    }
    Some(DesktopEntry {
        id,
        name: name?,
        wm_class,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminals_are_recognised_by_class() {
        for name in ["gnome-terminal-server", "xterm", "kitty", "org.wezfurlong.wezterm", "kgx"] {
            assert!(names_terminal(name), "{name}");
        }
        for name in ["google-chrome", "code", "firefox", "slack", "nautilus"] {
            assert!(!names_terminal(name), "{name}");
        }
    }

    #[test]
    fn desktop_entries_read_the_main_group_only() {
        let body = "[Desktop Entry]\nName=Google Chrome\nStartupWMClass=Google-chrome\n\
                    [Desktop Action new-window]\nName=New Window\n";
        let e = parse_desktop_entry("google-chrome".into(), body).unwrap();
        assert_eq!(e.name, "Google Chrome");
        assert_eq!(e.wm_class.as_deref(), Some("Google-chrome"));
        assert!(parse_desktop_entry("x".into(), "[Desktop Entry]\nType=Application\n").is_none());
    }
}
