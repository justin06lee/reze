//! A short rolling record of what has just been typed. macOS only.
//!
//! Reading the text in front of the caret by selecting and copying it works in
//! ordinary text fields, but not in a terminal: Option+Left is an escape
//! sequence to the shell rather than a selection, and Cmd+C copies the
//! terminal's view selection, not the line being edited. TUIs like a CLI prompt
//! own their line editor entirely, so there is no text field to interrogate.
//!
//! So instead of asking the app what is there, we remember what we saw typed
//! and delete it with backspaces. That works anywhere keystrokes go.
//!
//! ## What is and is not captured
//!
//! The buffer is capped at [`MAX_CHARS`], lives only in memory, is never
//! written to disk or sent anywhere, and is cleared aggressively — on Enter,
//! Tab, Escape, any arrow or navigation key, any mouse click, any shortcut
//! with a modifier held, and whenever the frontmost application changes.
//! macOS also disables event taps entirely while secure input is active, so
//! password fields are never observed. It can be turned off with the
//! `trackTyping` setting, which falls back to selection-based reading.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Mutex;

/// Longer than any sensible trigger, short enough to be worthless to anyone.
const MAX_CHARS: usize = 160;

static BUFFER: Mutex<String> = Mutex::new(String::new());
static RUNNING: AtomicBool = AtomicBool::new(false);
static LAST_APP: AtomicI32 = AtomicI32::new(-1);

/// Text set aside by a shortcut keypress, recoverable only for a moment. Long
/// enough to survive the hotkey that triggered the expansion, far too short to
/// resurrect anything the user has moved on from.
static SHELVED: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);
const SHELF_LIFE: std::time::Duration = std::time::Duration::from_millis(400);

pub fn is_running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// What is currently in front of the caret, as best we know.
///
/// This also promotes a just-shelved buffer back into place. The hotkey that
/// asks for an expansion is itself a modified keypress, which arrives at the
/// tap microseconds earlier and would otherwise have thrown away the very text
/// we are being asked to expand.
pub fn snapshot() -> String {
    let mut buf = BUFFER.lock().unwrap();
    if buf.is_empty() {
        if let Some((text, when)) = SHELVED.lock().unwrap().take() {
            if when.elapsed() < SHELF_LIFE {
                *buf = text;
            }
        }
    }
    buf.clone()
}

pub fn clear() {
    BUFFER.lock().unwrap().clear();
    *SHELVED.lock().unwrap() = None;
}

/// Clear, but keep the text briefly recoverable.
///
/// Used only for shortcut keypresses. A shortcut usually means the buffer no
/// longer describes what is in front of the caret — but it might also be the
/// expand hotkey, in which case [`snapshot`] needs it back immediately.
fn shelve() {
    let mut buf = BUFFER.lock().unwrap();
    if !buf.is_empty() {
        *SHELVED.lock().unwrap() = Some((std::mem::take(&mut buf), std::time::Instant::now()));
    }
}

/// Forget the last `chars` characters, after we have backspaced over them.
pub fn drop_last(chars: usize) {
    let mut buf = BUFFER.lock().unwrap();
    let keep = buf.chars().count().saturating_sub(chars);
    let cut = buf
        .char_indices()
        .nth(keep)
        .map(|(i, _)| i)
        .unwrap_or(buf.len());
    buf.truncate(cut);
}

fn push(text: &str) {
    let mut buf = BUFFER.lock().unwrap();
    buf.push_str(text);
    let len = buf.chars().count();
    if len > MAX_CHARS {
        let cut = buf
            .char_indices()
            .nth(len - MAX_CHARS)
            .map(|(i, _)| i)
            .unwrap_or(0);
        buf.drain(..cut);
    }
}

fn backspace_one() {
    let mut buf = BUFFER.lock().unwrap();
    buf.pop();
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
        CallbackResult, EventField,
    };
    use foreign_types::ForeignType;
    use std::sync::OnceLock;

    // Virtual keycodes that mean "the caret moved or the line ended", after
    // which anything still in the buffer no longer sits in front of the caret.
    const RESET_KEYS: [i64; 13] = [
        36,  // Return
        76,  // Enter (keypad)
        48,  // Tab
        53,  // Escape
        123, // Left
        124, // Right
        125, // Down
        126, // Up
        115, // Home
        119, // End
        116, // Page Up
        121, // Page Down
        117, // Forward Delete
    ];
    const BACKSPACE: i64 = 51;

    struct Tap(CGEventTap<'static>);
    // Only ever touched from the main thread; the wrapper exists so the handle
    // can live in a static and be re-enabled if macOS disables the tap.
    unsafe impl Send for Tap {}
    unsafe impl Sync for Tap {}
    static TAP: OnceLock<Tap> = OnceLock::new();

    unsafe extern "C" {
        fn CGEventKeyboardGetUnicodeString(
            event: core_graphics::sys::CGEventRef,
            max_length: libc::c_ulong,
            actual_length: *mut libc::c_ulong,
            string: *mut u16,
        );
    }

    fn typed_text(event: &core_graphics::event::CGEvent) -> Option<String> {
        let mut buf = [0u16; 8];
        let mut len: libc::c_ulong = 0;
        unsafe {
            CGEventKeyboardGetUnicodeString(
                event.as_ptr(),
                buf.len() as libc::c_ulong,
                &mut len,
                buf.as_mut_ptr(),
            );
        }
        if len == 0 {
            return None;
        }
        let text = String::from_utf16_lossy(&buf[..len as usize]);
        // Control characters are events, not typing.
        if text.chars().any(|c| c.is_control()) {
            return None;
        }
        Some(text)
    }

    fn on_key(event: &core_graphics::event::CGEvent) {
        // A modifier held down means a shortcut, not typing. This also keeps
        // the expand hotkey itself out of the buffer, which would otherwise
        // make us backspace one character too many.
        let flags = event.get_flags();
        use core_graphics::event::CGEventFlags as F;
        if flags.intersects(F::CGEventFlagCommand | F::CGEventFlagControl | F::CGEventFlagAlternate)
        {
            // Shelved rather than dropped: this could be an editing shortcut,
            // after which the buffer is stale — or it could be the expand
            // hotkey, which needs the buffer intact a moment from now.
            super::shelve();
            return;
        }

        // Typing into a different app than last time starts a new context.
        let front = crate::focus::frontmost_pid().unwrap_or(-1);
        if LAST_APP.swap(front, Ordering::Relaxed) != front {
            super::clear();
        }

        let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
        if RESET_KEYS.contains(&code) {
            super::clear();
        } else if code == BACKSPACE {
            super::backspace_one();
        } else if let Some(text) = typed_text(event) {
            super::push(&text);
        }
    }

    /// Install the tap. Must be called on the main thread, whose run loop the
    /// tap is attached to.
    pub fn start() -> Result<(), String> {
        if RUNNING.load(Ordering::Relaxed) {
            return Ok(());
        }
        let tap = CGEventTap::new(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            // Listen only: we never modify or delay anyone's keystrokes.
            CGEventTapOptions::ListenOnly,
            vec![
                CGEventType::KeyDown,
                CGEventType::LeftMouseDown,
                CGEventType::RightMouseDown,
            ],
            |_proxy, kind, event| {
                match kind {
                    CGEventType::KeyDown => on_key(event),
                    CGEventType::LeftMouseDown | CGEventType::RightMouseDown => super::clear(),
                    // macOS switches a tap off if its callback ever runs long.
                    CGEventType::TapDisabledByTimeout
                    | CGEventType::TapDisabledByUserInput => {
                        if let Some(tap) = TAP.get() {
                            tap.0.enable();
                        }
                    }
                    _ => {}
                }
                CallbackResult::Keep
            },
        )
        .map_err(|_| "could not create the keyboard tap".to_string())?;

        let source = tap
            .mach_port()
            .create_runloop_source(0)
            .map_err(|_| "could not attach the keyboard tap to the run loop".to_string())?;
        unsafe { CFRunLoop::get_current().add_source(&source, kCFRunLoopCommonModes) };
        tap.enable();

        let _ = TAP.set(Tap(tap));
        RUNNING.store(true, Ordering::Relaxed);
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    /// No keystroke tap outside macOS yet; callers fall back to selecting and
    /// copying the text in front of the caret.
    pub fn start() -> Result<(), String> {
        Err("keystroke tracking is only implemented on macOS".into())
    }
}

pub use imp::start;
