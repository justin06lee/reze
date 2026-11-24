//! Clipboard history: everything you copy, and which app you copied it from.
//!
//! macOS will not tell us the pasteboard changed — there is no notification, no
//! callback, nothing to subscribe to. The only signal is `changeCount`, a
//! number that goes up. So this polls, the way every clipboard manager on the
//! platform does, and reads the pasteboard whenever that number moves. X11
//! does announce changes, and [`crate::clip`] turns those announcements into
//! the same kind of number, so one loop serves both.
//!
//! Two things follow from that, and they are most of this module:
//!
//! - **Our own pasteboard traffic must not come back as history.** Reze writes
//!   to the pasteboard constantly — every paste, every restore, and the
//!   synthesized ⌘C that expand-in-place uses to read the words at the caret.
//!   Left alone, using the app would fill the history with the app. Hence
//!   [`suspend`], which every one of those paths holds for its duration.
//! - **The source app is an inference.** Nothing on the pasteboard says who
//!   wrote it, so we take whoever was frontmost when the count moved. That is
//!   right unless you copy and switch apps within one poll interval.
//!
//! Storage is deliberately separate from `macros.json`: that file is watched for
//! external edits and rebroadcast, and a history that rewrote it on every copy
//! would have both windows repainting all day.

use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::clip::Snapshot;

/// How often to ask whether the pasteboard changed.
///
/// Fast enough that the entry is there before you can reach the hotkey, slow
/// enough to be free: the poll itself is one integer read, and the pasteboard
/// is only actually *read* on the rare tick where that integer moved.
const POLL: Duration = Duration::from_millis(400);

/// The longest text worth keeping.
///
/// Not a limit on what you can copy — only on what is written to disk. The
/// index is rewritten whole on every copy, and one pathological paste of a
/// whole log file would make every subsequent copy pay for it.
const MAX_TEXT: usize = 1_000_000;

/// The largest image worth keeping, for the same reason.
const MAX_IMAGE: usize = 32 * 1024 * 1024;

/// How much of a text entry the windows are given.
///
/// The full text stays here and is only resolved when something is actually
/// pasted, so the palette can list two hundred entries without carrying two
/// hundred documents across the IPC bridge.
const PREVIEW_CHARS: usize = 800;

// ------------------------------------------------------------------ state

/// Whether to record at all. Mirrored from settings rather than read from disk,
/// because the poll loop must not touch the filesystem on every tick.
static ENABLED: AtomicBool = AtomicBool::new(true);

/// How many entries to keep.
static LIMIT: AtomicU32 = AtomicU32::new(default_limit());

/// The change count we have already accounted for.
static SEEN: AtomicIsize = AtomicIsize::new(0);

/// Nesting depth of [`suspend`]. Nested rather than a flag because the paste
/// path and the expand path can both be holding one.
static SUSPENDED: AtomicUsize = AtomicUsize::new(0);

pub const fn default_limit() -> u32 {
    200
}

/// A promise that nothing written to the pasteboard while this is alive will be
/// recorded — held across every write Reze makes, and across the synthesized
/// ⌘C that reads a selection back out of another app.
///
/// On drop it re-baselines rather than simply unblocking: whatever the count is
/// by then is ours, and the next tick must not treat the accumulated change as
/// something the user did.
pub struct Suspend;

pub fn suspend() -> Suspend {
    SUSPENDED.fetch_add(1, Ordering::SeqCst);
    Suspend
}

impl Drop for Suspend {
    fn drop(&mut self) {
        // Baseline first, then unblock, or a tick landing between the two would
        // see our own write as new.
        SEEN.store(change_count(), Ordering::SeqCst);
        SUSPENDED.fetch_sub(1, Ordering::SeqCst);
    }
}

fn change_count() -> isize {
    crate::focus::pasteboard_change_count().unwrap_or(0)
}

// ------------------------------------------------------------------- data

/// One thing you copied.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    /// `text`, `image` or `file` — which of the fields below carries it.
    pub kind: String,
    /// The whole thing, for a text entry. Never sent to the windows; see [`View`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Bare name of the blob inside `~/.reze/clipboard`, for an image.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub file: String,
    /// Absolute paths, for files copied in Finder. Referenced where they lie —
    /// copying a Finder selection into a shadow directory is not what anyone
    /// means by remembering that they copied it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    pub mime: String,
    pub bytes: u64,
    /// Hash of the content, so copying the same thing twice moves one entry up
    /// rather than making two.
    pub digest: String,
    /// Unix milliseconds.
    pub copied_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

/// The app it was copied from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    /// Empty for a process without one.
    #[serde(default)]
    pub bundle_id: String,
    pub name: String,
    /// Bare name of its icon inside `~/.reze/clipboard/icons`, written once per
    /// app and shared by every entry from it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
}

/// An entry as the windows see it: everything except the payload.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: String,
    pub kind: String,
    /// The first [`PREVIEW_CHARS`] of a text entry, or the file names of a file
    /// one. Enough to recognise it by and to search; not the thing itself.
    pub preview: String,
    /// Length of the whole text, so the preview can say what it left out.
    pub chars: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    pub mime: String,
    pub bytes: u64,
    pub copied_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<Source>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct History {
    #[serde(default)]
    entries: Vec<Entry>,
}

pub fn clip_dir() -> PathBuf {
    crate::store::config_dir().join("clipboard")
}

fn icons_dir() -> PathBuf {
    clip_dir().join("icons")
}

fn index_path() -> PathBuf {
    crate::store::config_dir().join("clipboard.json")
}

static STORE: OnceLock<Mutex<History>> = OnceLock::new();

fn store() -> &'static Mutex<History> {
    STORE.get_or_init(|| {
        let history = std::fs::read_to_string(index_path())
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Mutex::new(history)
    })
}

/// Write the index out.
///
/// A failure here is not worth surfacing anywhere: the history is a
/// convenience, and an unwritable one should not interrupt a copy.
fn persist(history: &History) {
    let dir = crate::store::config_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let Ok(body) = serde_json::to_string(history) else {
        return;
    };
    let path = index_path();
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, &body).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

// ------------------------------------------------------------- public API

/// Point the recorder at the current settings.
pub fn configure(settings: &crate::store::Settings) {
    ENABLED.store(settings.clipboard_history, Ordering::SeqCst);
    LIMIT.store(settings.clipboard_limit.max(1), Ordering::SeqCst);
    let mut history = store().lock().unwrap();
    if prune(&mut history) {
        persist(&history);
    }
}

/// Everything remembered, newest first.
pub fn views() -> Vec<View> {
    store().lock().unwrap().entries.iter().map(view).collect()
}

fn view(entry: &Entry) -> View {
    let preview = match entry.kind.as_str() {
        "text" => entry.text.chars().take(PREVIEW_CHARS).collect(),
        "file" => entry
            .paths
            .iter()
            .map(|p| {
                std::path::Path::new(p)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| p.clone())
            })
            .collect::<Vec<_>>()
            .join(", "),
        _ => String::new(),
    };
    View {
        id: entry.id.clone(),
        kind: entry.kind.clone(),
        preview,
        chars: entry.text.chars().count(),
        file: entry.file.clone(),
        paths: entry.paths.clone(),
        mime: entry.mime.clone(),
        bytes: entry.bytes,
        copied_at: entry.copied_at,
        source: entry.source.clone(),
    }
}

/// What it would take to put entry `id` back on the pasteboard.
pub fn payload_for(id: &str) -> Result<crate::paste::Payload, String> {
    let history = store().lock().unwrap();
    let entry = history
        .entries
        .iter()
        .find(|e| e.id == id)
        .ok_or("that clipboard entry is gone")?;

    match entry.kind.as_str() {
        "text" => Ok(crate::paste::Payload::Bundle {
            text: entry.text.clone(),
            files: Vec::new(),
        }),
        "image" => {
            let path = crate::assets::bare_name(&clip_dir(), &entry.file)?;
            if !path.exists() {
                return Err("that image is no longer on disk".into());
            }
            Ok(crate::paste::Payload::Image {
                mime: entry.mime.clone(),
                path,
            })
        }
        _ => {
            let files: Vec<PathBuf> = entry
                .paths
                .iter()
                .map(PathBuf::from)
                .filter(|p| p.exists())
                .collect();
            if files.is_empty() {
                return Err("those files have been moved or deleted".into());
            }
            Ok(crate::paste::Payload::Bundle {
                text: String::new(),
                files,
            })
        }
    }
}

pub fn remove(id: &str) {
    let mut history = store().lock().unwrap();
    if let Some(pos) = history.entries.iter().position(|e| e.id == id) {
        let gone = history.entries.remove(pos);
        discard_blob(&gone);
        persist(&history);
    }
}

/// Forget everything. Icons stay — they belong to apps, not to entries, and
/// the next copy from the same app would only fetch them again.
pub fn clear() {
    let mut history = store().lock().unwrap();
    for entry in &history.entries {
        discard_blob(entry);
    }
    history.entries.clear();
    persist(&history);
}

/// The newest thing copied, whole — what a machine sharing its clipboard
/// hands to the others.
pub fn latest() -> Option<Entry> {
    store().lock().unwrap().entries.first().cloned()
}

/// Something copied on another of this user's machines.
pub struct Arrival {
    /// `text` or `image`.
    pub kind: String,
    pub text: String,
    /// For an image: the bare name it was fetched under, already in
    /// [`clip_dir`].
    pub file: String,
    pub mime: String,
    /// The machine it was copied on, by its tailnet name.
    pub from: String,
}

/// Put what another machine copied on this one's clipboard, and remember it
/// like anything copied here — with the machine it came from where the app
/// would be.
///
/// Says whether anything changed: the same thing arriving twice, or arriving
/// back at the machine that copied it, is not a change.
pub fn receive(arrival: Arrival) -> Result<bool, String> {
    if !ENABLED.load(Ordering::SeqCst) {
        return Ok(false);
    }
    let blob = match arrival.kind.as_str() {
        "image" => Some(crate::assets::bare_name(&clip_dir(), &arrival.file)?),
        _ => None,
    };
    let (digest, bytes) = match &blob {
        Some(path) => {
            let image = std::fs::read(path).map_err(|e| format!("reading {path:?}: {e}"))?;
            (hash(&image), image.len() as u64)
        }
        None => (hash(arrival.text.as_bytes()), arrival.text.len() as u64),
    };

    let mut history = store().lock().unwrap();
    let known = history.entries.iter().position(|e| e.digest == digest);
    if known == Some(0) {
        // Already the newest thing here, which is to say already on the
        // clipboard. The copy just fetched is one too many.
        if let Some(path) = &blob {
            let _ = std::fs::remove_file(path);
        }
        return Ok(false);
    }

    {
        // Ours to write, and not something anyone copied *here*: without this
        // it would be recorded a second time, and announced straight back.
        let _quiet = suspend();
        match &blob {
            Some(path) => crate::paste::copy_payload(&crate::paste::Payload::Image {
                mime: arrival.mime.clone(),
                path: path.clone(),
            })?,
            None => crate::paste::write_clipboard(&arrival.text)?,
        }
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    match known {
        Some(pos) => {
            // Copied here before, and now there: one entry, moved up.
            let mut moved = history.entries.remove(pos);
            moved.copied_at = now;
            history.entries.insert(0, moved);
            if let Some(path) = &blob {
                let _ = std::fs::remove_file(path);
            }
        }
        None => {
            history.entries.insert(
                0,
                Entry {
                    id: uuid::Uuid::new_v4().to_string(),
                    kind: arrival.kind,
                    text: arrival.text,
                    file: arrival.file,
                    paths: Vec::new(),
                    mime: arrival.mime,
                    bytes,
                    digest,
                    copied_at: now,
                    source: Some(Source {
                        bundle_id: String::new(),
                        name: arrival.from,
                        icon: String::new(),
                    }),
                },
            );
            prune(&mut history);
        }
    }
    persist(&history);
    drop(history);

    if let Some(app) = APP.get() {
        use tauri::Emitter;
        let _ = app.emit("clipboard-changed", views());
    }
    Ok(true)
}

/// For telling the windows about a copy that arrived from another machine,
/// which happens on a thread nobody handed an app to.
static APP: OnceLock<tauri::AppHandle> = OnceLock::new();

/// Start watching the pasteboard. Must be called on the main thread, where
/// Linux's change counter hooks into GTK. Does nothing useful where there is no
/// change count to watch.
pub fn start(app: tauri::AppHandle) {
    let _ = APP.set(app.clone());
    let _ = std::fs::create_dir_all(icons_dir());
    crate::clip::watch();
    SEEN.store(change_count(), Ordering::SeqCst);

    std::thread::spawn(move || loop {
        std::thread::sleep(POLL);

        // Switched off: keep the baseline moving so turning it back on does not
        // immediately record whatever happened to be copied while it was off.
        if !ENABLED.load(Ordering::SeqCst) {
            SEEN.store(change_count(), Ordering::SeqCst);
            continue;
        }
        if SUSPENDED.load(Ordering::SeqCst) != 0 {
            continue;
        }
        let count = change_count();
        if count == SEEN.load(Ordering::SeqCst) {
            continue;
        }
        SEEN.store(count, Ordering::SeqCst);

        let Some(snapshot) = crate::clip::read_snapshot() else {
            continue;
        };
        if record(snapshot).is_some() {
            use tauri::Emitter;
            let _ = app.emit("clipboard-changed", views());
            // And the other machines, if the clipboard is shared with them.
            crate::tailnet::copied();
        }
    });
}

// ------------------------------------------------------------- recording

/// Turn a pasteboard snapshot into an entry, if it is worth keeping.
///
/// Returns `None` when nothing changed for the windows to hear about, including
/// the ordinary case of re-copying the item already at the top.
fn record(snapshot: Snapshot) -> Option<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let (kind, digest, mime, bytes, text, paths, blob) = match snapshot {
        Snapshot::Text(text) => {
            if text.trim().is_empty() || text.len() > MAX_TEXT {
                return None;
            }
            let digest = hash(text.as_bytes());
            let bytes = text.len() as u64;
            (
                "text",
                digest,
                "text/plain".to_string(),
                bytes,
                text,
                Vec::new(),
                None,
            )
        }
        Snapshot::Files(files) => {
            let paths: Vec<String> = files
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            if paths.is_empty() {
                return None;
            }
            let bytes = files
                .iter()
                .filter_map(|p| std::fs::metadata(p).ok())
                .map(|m| m.len())
                .sum();
            let digest = hash(paths.join("\n").as_bytes());
            (
                "file",
                digest,
                "application/octet-stream".to_string(),
                bytes,
                String::new(),
                paths,
                None,
            )
        }
        Snapshot::Image { mime, bytes } => {
            if bytes.is_empty() || bytes.len() > MAX_IMAGE {
                return None;
            }
            let digest = hash(&bytes);
            let len = bytes.len() as u64;
            (
                "image",
                digest,
                mime,
                len,
                String::new(),
                Vec::new(),
                Some(bytes),
            )
        }
    };

    let mut history = store().lock().unwrap();

    // Already have it: move it up and re-stamp it rather than storing it twice.
    // The source is left as it was — the first app to produce this text is the
    // more interesting answer to "where did this come from".
    if let Some(pos) = history.entries.iter().position(|e| e.digest == digest) {
        if pos == 0 {
            history.entries[0].copied_at = now;
            persist(&history);
            return None;
        }
        let mut moved = history.entries.remove(pos);
        moved.copied_at = now;
        history.entries.insert(0, moved);
        persist(&history);
        return Some(());
    }

    // Only now is the blob worth writing: a re-copied image would have matched
    // above and made this a wasted few megabytes.
    let file = match blob {
        Some(bytes) => match write_blob(&mime, &bytes) {
            Some(name) => name,
            None => return None,
        },
        None => String::new(),
    };

    history.entries.insert(
        0,
        Entry {
            id: uuid::Uuid::new_v4().to_string(),
            kind: kind.to_string(),
            text,
            file,
            paths,
            mime,
            bytes,
            digest,
            copied_at: now,
            source: source_now(),
        },
    );
    prune(&mut history);
    persist(&history);
    Some(())
}

/// Drop the oldest entries past the limit. Returns whether anything went.
fn prune(history: &mut History) -> bool {
    let limit = LIMIT.load(Ordering::SeqCst) as usize;
    if history.entries.len() <= limit {
        return false;
    }
    for gone in history.entries.drain(limit..) {
        discard_blob(&gone);
    }
    true
}

fn discard_blob(entry: &Entry) {
    if entry.file.is_empty() {
        return;
    }
    if let Ok(path) = crate::assets::bare_name(&clip_dir(), &entry.file) {
        let _ = std::fs::remove_file(path);
    }
}

fn write_blob(mime: &str, bytes: &[u8]) -> Option<String> {
    let dir = clip_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let name = format!(
        "{}.{}",
        uuid::Uuid::new_v4(),
        crate::assets::ext_for_mime(mime)
    );
    std::fs::write(dir.join(&name), bytes).ok()?;
    Some(name)
}

fn hash(bytes: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Who is frontmost, with their icon cached to disk on first sight.
fn source_now() -> Option<Source> {
    let app = crate::focus::frontmost_app()?;
    let key = icon_key(&app);
    let icon = match key {
        Some(key) => {
            let name = format!("{key}.png");
            let path = icons_dir().join(&name);
            if !path.exists() {
                if let Some(png) = crate::focus::app_icon_png(&app) {
                    let _ = std::fs::create_dir_all(icons_dir());
                    let _ = std::fs::write(&path, png);
                }
            }
            match path.exists() {
                true => name,
                false => String::new(),
            }
        }
        None => String::new(),
    };
    Some(Source {
        bundle_id: app.bundle_id,
        name: app.name,
        icon,
    })
}

/// A file name for this app's icon: its bundle id with anything that is not
/// safe in a path flattened out, so `com.apple.Safari` stays legible.
fn icon_key(app: &crate::focus::FrontApp) -> Option<String> {
    let raw = match app.bundle_id.is_empty() {
        false => &app.bundle_id,
        true => &app.name,
    };
    let key: String = raw
        .chars()
        .map(|c| match c.is_ascii_alphanumeric() || c == '.' || c == '-' {
            true => c,
            false => '_',
        })
        .collect();
    let key = key.trim_matches('.').to_string();
    match key.is_empty() {
        true => None,
        false => Some(key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_entry(text: &str, at: u64) -> Entry {
        Entry {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "text".into(),
            text: text.into(),
            file: String::new(),
            paths: Vec::new(),
            mime: "text/plain".into(),
            bytes: text.len() as u64,
            digest: hash(text.as_bytes()),
            copied_at: at,
            source: None,
        }
    }

    #[test]
    fn a_view_carries_the_head_of_the_text_and_the_whole_length() {
        let long = "x".repeat(PREVIEW_CHARS * 2);
        let v = view(&text_entry(&long, 0));
        assert_eq!(v.preview.chars().count(), PREVIEW_CHARS);
        assert_eq!(v.chars, PREVIEW_CHARS * 2);
    }

    #[test]
    fn a_file_entry_previews_as_its_file_names() {
        let mut entry = text_entry("", 0);
        entry.kind = "file".into();
        entry.paths = vec!["/tmp/one.png".into(), "/tmp/deep/two.mov".into()];
        assert_eq!(view(&entry).preview, "one.png, two.mov");
    }

    #[test]
    fn pruning_keeps_the_newest() {
        LIMIT.store(2, Ordering::SeqCst);
        let mut history = History {
            entries: vec![text_entry("a", 3), text_entry("b", 2), text_entry("c", 1)],
        };
        assert!(prune(&mut history));
        assert_eq!(
            history.entries.iter().map(|e| e.text.as_str()).collect::<Vec<_>>(),
            ["a", "b"],
        );
        LIMIT.store(default_limit(), Ordering::SeqCst);
    }

    #[test]
    fn icon_keys_are_plain_file_names() {
        let app = |bundle: &str, name: &str| crate::focus::FrontApp {
            pid: 0,
            bundle_id: bundle.into(),
            name: name.into(),
            window: 0,
        };
        assert_eq!(icon_key(&app("com.apple.Safari", "Safari")).unwrap(), "com.apple.Safari");
        // No bundle id: the name has to do, sanitised.
        assert_eq!(icon_key(&app("", "My App!")).unwrap(), "My_App_");
        // Nothing that could climb out of the icons directory survives.
        assert_eq!(icon_key(&app("../../etc/passwd", "")).unwrap(), "_.._etc_passwd");
        assert!(icon_key(&app("", "")).is_none());
    }

    #[test]
    fn identical_content_hashes_identically() {
        assert_eq!(hash(b"hello"), hash(b"hello"));
        assert_ne!(hash(b"hello"), hash(b"hellp"));
    }
}
