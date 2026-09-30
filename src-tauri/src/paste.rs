//! Getting text into the app the user was actually using.
//!
//! macOS gives no supported way to inject text directly into a foreign app, so
//! the universal approach is: put the text on the clipboard, synthesize Cmd+V,
//! then put the user's clipboard back. That requires Accessibility permission.

use std::path::PathBuf;
use std::time::Duration;

/// Whether this app is allowed to synthesize keystrokes.
/// Without it, the Cmd+V is silently swallowed by the OS — no error, nothing
/// pasted — which is why we check up front rather than letting it fail quietly.
pub fn has_accessibility() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_accessibility_client::accessibility::application_is_trusted()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

/// Same check, but asks the OS to show the "grant access" prompt.
pub fn request_accessibility() -> bool {
    #[cfg(target_os = "macos")]
    {
        macos_accessibility_client::accessibility::application_is_trusted_with_prompt()
    }
    #[cfg(not(target_os = "macos"))]
    {
        true
    }
}

pub fn read_clipboard() -> Result<String, String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.get_text().or_else(|_| Ok(String::new()))
}

pub fn write_clipboard(text: &str) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    cb.set_text(text.to_string()).map_err(|e| e.to_string())
}

/// A value nothing else would ever put on the clipboard.
///
/// Used where no pasteboard change counter is available: write this, send the
/// copy, and if it is still there afterwards then nothing was selected. The
/// caller restores the user's clipboard either way.
pub const SENTINEL: &str = "\u{200b}reze-nothing-was-copied\u{200b}";

/// What goes on the pasteboard.
///
/// Two shapes rather than one because they arrive differently. A macro is text
/// and files together; a clipboard-history image is bytes that were never a
/// file to the app that copied them, and putting it back as a file attachment
/// named after a UUID would not be the thing that was copied.
pub enum Payload {
    /// Text, then the files — see [`paste_payload`] for why in that order.
    Bundle { text: String, files: Vec<PathBuf> },
    /// Raw image bytes, read from `path` at the moment of pasting.
    Image { mime: String, path: PathBuf },
}

/// Put `payload` on the clipboard and paste it into the frontmost app.
///
/// A pasteboard holds one payload, so a macro carrying both text and files
/// takes two passes: **text first, then the files**. That order leaves the text
/// landing exactly where the caret already was — unchanged from a text-only
/// macro, including the surrounding characters an in-place expansion puts back
/// — and lets the attachments follow it. In a chat composer they drop into the
/// attachment tray; in a document they land after the paragraph. Both are what
/// you would have done by hand, in that order.
///
/// The caller is responsible for having already hidden our window — macOS
/// restores focus to the previous app on hide, and the paste lands wherever
/// focus ended up.
///
/// `run_on_main` must execute the given function on the main thread and block
/// until it finishes. Synthesizing the keystroke has to happen there: mapping
/// a character to a layout-specific keycode goes through the Text Services
/// Manager, which calls `dispatch_assert_queue(main)` internally and raises
/// SIGTRAP — an uncatchable hard crash, not an `Err` — anywhere else. The
/// clipboard work and the sleeps deliberately stay off the main thread so the
/// UI is never blocked for the half second this takes.
pub fn paste_payload(
    payload: &Payload,
    restore_clipboard: bool,
    run_on_main: impl Fn(fn() -> Result<(), String>) -> Result<(), String>,
) -> Result<(), String> {
    if !has_accessibility() {
        return Err("accessibility-denied".into());
    }

    // Only text is worth putting back. If the user had an image copied, the
    // best we could restore is the empty string `get_text` gives for it — which
    // would destroy the thing we were trying to preserve.
    let previous = match restore_clipboard {
        true => read_clipboard().ok().filter(|s| !s.is_empty()),
        false => None,
    };

    match payload {
        Payload::Bundle { text, files } => {
            if !text.is_empty() {
                write_clipboard(text)?;
                // Let the clipboard server and the focus change settle first.
                std::thread::sleep(Duration::from_millis(120));
                run_on_main(send_paste_keystroke)?;
            }

            if !files.is_empty() {
                // The first paste has to have been read by the target app
                // before the pasteboard is overwritten, or the second one
                // arrives instead of it.
                if !text.is_empty() {
                    std::thread::sleep(Duration::from_millis(280));
                }
                crate::clip::write_files(files)?;
                std::thread::sleep(Duration::from_millis(140));
                run_on_main(send_paste_keystroke)?;
            }
        }
        Payload::Image { mime, path } => {
            let bytes = std::fs::read(path).map_err(|e| format!("reading {path:?}: {e}"))?;
            crate::clip::write_image(mime, &bytes)?;
            std::thread::sleep(Duration::from_millis(140));
            run_on_main(send_paste_keystroke)?;
        }
    }

    if let Some(prev) = previous {
        // Long enough that the target app has read the pasteboard already.
        std::thread::sleep(Duration::from_millis(400));
        let _ = write_clipboard(&prev);
    }
    Ok(())
}

/// Put the payload on the clipboard without pasting it — the ⌘↵ path.
///
/// One pasteboard, one payload: the text if there is any, otherwise the files.
/// A macro carrying both cannot be copied as both, and silently dropping the
/// text a user could actually see in the preview would be the worse half to lose.
pub fn copy_payload(payload: &Payload) -> Result<(), String> {
    match payload {
        Payload::Bundle { text, files } if text.is_empty() && !files.is_empty() => {
            crate::clip::write_files(files)
        }
        Payload::Bundle { text, .. } => write_clipboard(text),
        Payload::Image { mime, path } => {
            let bytes = std::fs::read(path).map_err(|e| format!("reading {path:?}: {e}"))?;
            crate::clip::write_image(mime, &bytes)
        }
    }
}

/// The key combinations we synthesize. Named rather than exposing `enigo::Key`
/// so the rest of the app never has to reason about keycodes.
#[derive(Clone, Copy)]
pub enum Chord {
    /// Extend the selection one word to the left.
    ExtendWordLeft,
    /// Drop the selection, leaving the caret at its right-hand end.
    CollapseRight,
    /// Delete one character to the left of the caret.
    Backspace,
    Copy,
    Paste,
}

/// Send `chord` `times` times. **Main thread only** — see [`paste_payload`].
pub fn send(chord: Chord, times: usize) -> Result<(), String> {
    use enigo::Key;

    #[cfg(target_os = "macos")]
    let command = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let command = Key::Control;

    // Option+Arrow is the by-word motion on macOS; Control+Arrow elsewhere.
    #[cfg(target_os = "macos")]
    let word = Key::Alt;
    #[cfg(not(target_os = "macos"))]
    let word = Key::Control;

    let (modifiers, key): (&[Key], Key) = match chord {
        Chord::ExtendWordLeft => (&[Key::Shift, word], Key::LeftArrow),
        Chord::CollapseRight => (&[], Key::RightArrow),
        Chord::Backspace => (&[], Key::Backspace),
        Chord::Copy => (&[command], Key::Unicode('c')),
        Chord::Paste => (&[command], Key::Unicode('v')),
    };
    tap(modifiers, key, times)
}

fn tap(modifiers: &[enigo::Key], key: enigo::Key, times: usize) -> Result<(), String> {
    use enigo::{Direction, Enigo, Keyboard, Settings};

    if times == 0 {
        return Ok(());
    }
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;

    for m in modifiers {
        enigo.key(*m, Direction::Press).map_err(|e| e.to_string())?;
    }

    let mut result = Ok(());
    for _ in 0..times {
        if let Err(e) = enigo.key(key, Direction::Click) {
            result = Err(e.to_string());
            break;
        }
    }

    // Release unconditionally and in reverse order. A modifier left stuck down
    // would break every subsequent keystroke on the machine, which is a far
    // worse outcome than a failed paste.
    for m in modifiers.iter().rev() {
        let _ = enigo.key(*m, Direction::Release);
    }
    result
}

/// Main thread only — see [`paste_payload`].
fn send_paste_keystroke() -> Result<(), String> {
    send(Chord::Paste, 1)
}
