//! A short rolling record of what has just been typed. macOS and X11.
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
//! password fields are never observed there. X11 has no such thing: any
//! client may watch the keyboard, and a password typed into a field passes
//! through the buffer like anything else — held in memory only, and gone at
//! the Enter that submits it. It can be turned off with the `trackTyping`
//! setting, which falls back to selection-based reading.

#[cfg(target_os = "macos")]
use std::sync::atomic::AtomicI32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Longer than any sensible trigger, short enough to be worthless to anyone.
const MAX_CHARS: usize = 160;

static BUFFER: Mutex<String> = Mutex::new(String::new());
static RUNNING: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
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

/// XInput2 raw events: every key the X server sees, whichever window it is
/// headed for — the X11 counterpart of a listen-only event tap. Raw events
/// carry keycodes rather than text, so the character is worked out here from
/// the keyboard mapping and whichever modifiers are seen held.
///
/// Only the first layout group is read. Someone switching between layouts will
/// find the buffer describes the first one, and expansion falls back to
/// selecting the text instead.
#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;
    use x11rb::connection::Connection;
    use x11rb::protocol::xinput::{self, ConnectionExt as _};
    use x11rb::protocol::xproto::{self, ConnectionExt as _};
    use x11rb::protocol::Event;
    use x11rb::rust_connection::RustConnection;

    const BACKSPACE: u32 = 0xff08;

    /// Keysyms after which anything still in the buffer no longer sits in
    /// front of the caret — the same set as the macOS keycodes above.
    fn resets(sym: u32) -> bool {
        matches!(
            sym,
            0xff0d | 0xff8d // Return, keypad Enter
            | 0xff09 | 0xfe20 // Tab, Shift+Tab
            | 0xff1b // Escape
            | 0xff50..=0xff57 // Home, arrows, Page Up, Page Down, End
            | 0xff95..=0xff9f // the keypad's navigation keys
            | 0xffff // Delete
        )
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Modifier {
        Shift,
        /// Control, Alt, Super and friends: a shortcut, not typing.
        Command,
        /// AltGr — a second layer of characters, not a shortcut.
        Level3,
        CapsLock,
        NumLock,
    }

    fn modifier(sym: u32) -> Option<Modifier> {
        Some(match sym {
            0xffe1 | 0xffe2 => Modifier::Shift,
            0xffe3 | 0xffe4 | 0xffe7..=0xffee => Modifier::Command,
            0xfe03 | 0xff7e => Modifier::Level3,
            0xffe5 => Modifier::CapsLock,
            0xff7f => Modifier::NumLock,
            _ => return None,
        })
    }

    struct Keymap {
        min: u8,
        per: usize,
        syms: Vec<u32>,
    }

    impl Keymap {
        fn load(conn: &RustConnection) -> Option<Self> {
            let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
            let reply = conn
                .get_keyboard_mapping(min, max - min + 1)
                .ok()?
                .reply()
                .ok()?;
            Some(Self {
                min,
                per: reply.keysyms_per_keycode as usize,
                syms: reply.keysyms,
            })
        }

        fn sym(&self, code: u8, index: usize) -> u32 {
            if code < self.min || index >= self.per {
                return 0;
            }
            let at = (code - self.min) as usize * self.per + index;
            self.syms.get(at).copied().unwrap_or(0)
        }
    }

    struct Tap {
        conn: RustConnection,
        active_window: xproto::Atom,
        keymap: Keymap,
        /// Modifier keys down right now, by keycode — so letting go of one
        /// Shift while holding the other still reads as shifted.
        held: Vec<(u8, Modifier)>,
        caps: bool,
        num: bool,
    }

    impl Tap {
        fn open() -> Result<Self, String> {
            let (conn, screen) =
                x11rb::connect(None).map_err(|e| format!("no X display ({e})"))?;
            let root = conn.setup().roots[screen].root;
            let no_xi2 = |_| "the X server has no XInput2".to_string();

            conn.xinput_xi_query_version(2, 2)
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(no_xi2)?;
            let mask = xinput::XIEventMask::RAW_KEY_PRESS
                | xinput::XIEventMask::RAW_KEY_RELEASE
                | xinput::XIEventMask::RAW_BUTTON_PRESS;
            conn.xinput_xi_select_events(
                root,
                &[xinput::EventMask {
                    deviceid: xinput::Device::ALL_MASTER.into(),
                    mask: vec![mask],
                }],
            )
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| format!("could not watch the keyboard ({e})"))?;

            // The focused window changing starts a new context, the way
            // switching apps does on macOS.
            conn.change_window_attributes(
                root,
                &xproto::ChangeWindowAttributesAux::new()
                    .event_mask(xproto::EventMask::PROPERTY_CHANGE),
            )
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| e.to_string())?;
            let active_window = conn
                .intern_atom(false, b"_NET_ACTIVE_WINDOW")
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?
                .atom;

            let keymap = Keymap::load(&conn).ok_or("could not read the keyboard mapping")?;
            // Caps Lock and Num Lock latch, so read where they stand now and
            // follow their presses from here on.
            let locks = conn
                .query_pointer(root)
                .ok()
                .and_then(|c| c.reply().ok())
                .map(|r| u16::from(r.mask))
                .unwrap_or(0);

            Ok(Self {
                conn,
                active_window,
                keymap,
                held: Vec::new(),
                caps: locks & 0x0002 != 0,
                num: locks & 0x0010 != 0,
            })
        }

        fn run(mut self) {
            while let Ok(event) = self.conn.wait_for_event() {
                match event {
                    Event::XinputRawKeyPress(e) => self.key(e.detail as u8, true),
                    Event::XinputRawKeyRelease(e) => self.key(e.detail as u8, false),
                    // 4–7 are the scroll wheel, which moves no caret.
                    Event::XinputRawButtonPress(e) if !(4..=7).contains(&e.detail) => {
                        super::clear()
                    }
                    Event::PropertyNotify(e) if e.atom == self.active_window => super::clear(),
                    Event::MappingNotify(_) => {
                        if let Some(keymap) = Keymap::load(&self.conn) {
                            self.keymap = keymap;
                        }
                    }
                    _ => {}
                }
            }
        }

        fn holding(&self, which: Modifier) -> bool {
            self.held.iter().any(|(_, m)| *m == which)
        }

        fn key(&mut self, code: u8, pressed: bool) {
            if let Some(m) = modifier(self.keymap.sym(code, 0)) {
                match (m, pressed) {
                    (Modifier::CapsLock, true) => self.caps = !self.caps,
                    (Modifier::NumLock, true) => self.num = !self.num,
                    (Modifier::CapsLock | Modifier::NumLock, false) => {}
                    (_, true) => {
                        if !self.held.iter().any(|(c, _)| *c == code) {
                            self.held.push((code, m));
                        }
                    }
                    (_, false) => self.held.retain(|(c, _)| *c != code),
                }
                return;
            }
            if !pressed {
                return;
            }

            // Shelved rather than dropped, exactly as on macOS: this is either
            // an editing shortcut or the expand hotkey itself.
            if self.holding(Modifier::Command) {
                super::shelve();
                return;
            }

            let (sym, upper) = self.resolve(code);
            if resets(sym) {
                super::clear();
            } else if sym == BACKSPACE {
                super::backspace_one();
            } else if let Some(c) = xkeysym::Keysym::from(sym).key_char() {
                if c.is_control() {
                    return;
                }
                match upper {
                    true => super::push(&c.to_uppercase().to_string()),
                    false => super::push(&c.to_string()),
                }
            }
        }

        /// The keysym `code` produces with the modifiers held now, and whether
        /// it still needs uppercasing — the core mapping lists only one
        /// symbol for a letter key on some layouts, leaving case to the reader.
        fn resolve(&self, code: u8) -> (u32, bool) {
            let k = &self.keymap;
            let shift = self.holding(Modifier::Shift);
            let (lower, upper) = (k.sym(code, 0), k.sym(code, 1));

            // The keypad: Num Lock swaps its navigation keys for digits, and
            // Shift swaps them back.
            if matches!(upper, 0xffb0..=0xffb9 | 0xffac | 0xffae) {
                return (if self.num != shift { upper } else { lower }, false);
            }

            // AltGr's two levels sit at 4 and 5 in the core mapping.
            let (lo, hi) = match self.holding(Modifier::Level3) {
                true => (k.sym(code, 4), k.sym(code, 5)),
                false => (lower, upper),
            };
            let letter = xkeysym::Keysym::from(lo)
                .key_char()
                .is_some_and(|c| c.is_lowercase());
            let shifted = shift != (self.caps && letter);
            match (shifted, hi) {
                (false, _) => (lo, false),
                (true, 0) => (lo, true),
                (true, hi) => (hi, false),
            }
        }
    }

    /// Start watching. Runs on a thread of its own with its own connection, so
    /// it can be called from anywhere; it only returns once the watcher is
    /// either listening or known not to be able to.
    pub fn start() -> Result<(), String> {
        if RUNNING.load(Ordering::Relaxed) {
            return Ok(());
        }
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("reze-typing".into())
            .spawn(move || match Tap::open() {
                Ok(tap) => {
                    RUNNING.store(true, Ordering::Relaxed);
                    let _ = tx.send(Ok(()));
                    tap.run();
                    // The X connection went away; say so, so the next
                    // expansion tries again rather than trusting a dead buffer.
                    RUNNING.store(false, Ordering::Relaxed);
                    super::clear();
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            })
            .map_err(|e| e.to_string())?;
        rx.recv_timeout(Duration::from_secs(3))
            .map_err(|_| "the keyboard watcher did not start".to_string())?
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod imp {
    /// No keystroke tap here; callers fall back to selecting and copying the
    /// text in front of the caret.
    pub fn start() -> Result<(), String> {
        Err("keystroke tracking is only implemented on macOS and X11".into())
    }
}
pub use imp::start;
