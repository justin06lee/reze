//! Getting text into the app the user was actually using.
//!
//! macOS gives no supported way to inject text directly into a foreign app, so
//! the universal approach is: put the text on the clipboard, synthesize Cmd+V,
//! then put the user's clipboard back. That requires Accessibility permission.

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

/// Copy `text` to the clipboard and paste it into the frontmost app.
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
pub fn paste(
    text: &str,
    restore_clipboard: bool,
    run_on_main: impl FnOnce(fn() -> Result<(), String>) -> Result<(), String>,
) -> Result<(), String> {
    if !has_accessibility() {
        return Err("accessibility-denied".into());
    }

    let previous = if restore_clipboard {
        read_clipboard().ok()
    } else {
        None
    };

    write_clipboard(text)?;
    // Let the clipboard server and the focus change settle before the keystroke.
    std::thread::sleep(Duration::from_millis(120));

    run_on_main(send_paste_keystroke)?;

    if let Some(prev) = previous {
        // Long enough that the target app has read the pasteboard already.
        std::thread::sleep(Duration::from_millis(400));
        let _ = write_clipboard(&prev);
    }
    Ok(())
}

/// Main thread only — see [`paste`].
fn send_paste_keystroke() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};

    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;

    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    enigo
        .key(modifier, Direction::Press)
        .map_err(|e| e.to_string())?;
    let result = enigo.key(Key::Unicode('v'), Direction::Click);
    // Release the modifier even if the keypress failed — a stuck Cmd key would
    // leave the user's machine in a genuinely broken state.
    let release = enigo.key(modifier, Direction::Release);
    result.map_err(|e| e.to_string())?;
    release.map_err(|e| e.to_string())?;
    Ok(())
}
