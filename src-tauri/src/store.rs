//! Macro persistence.
//!
//! The JSON file at `~/.reze/macros.json` is the source of truth, not the GUI.
//! It is watched for external edits so hand-editing in a text editor and
//! editing in the app both work, and neither clobbers the other.

use crate::assets::Attachment;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Macro {
    pub id: String,
    /// What you type in the palette, e.g. "full analysis".
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The expansion. May contain `{{vars}}` and `{{> includes}}`.
    pub body: String,
    /// Files pasted along with the body — images, video, anything. Defaulted so
    /// libraries written before attachments existed still load.
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// The watched folder this macro was derived from, if any.
    ///
    /// Derived macros are rebuilt from the folder on every scan and never
    /// written back to `macros.json` — the folder is their source of truth, the
    /// way this file is for the ones you wrote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(default)]
    pub usage_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub hotkey: String,
    /// Expands the trigger already typed at the caret, without showing a window.
    /// Defaulted so libraries written before this existed still load.
    #[serde(default = "default_expand_hotkey")]
    pub expand_hotkey: String,
    /// Watch what is typed so in-place expansion works in terminals and TUIs,
    /// where the text in front of the caret cannot be selected and read back.
    #[serde(default = "default_true")]
    pub track_typing: bool,
    /// Folders whose files become macros on their own. See [`crate::folders`].
    /// A link pasted into the palette is downloaded into the first one.
    #[serde(default = "default_folders")]
    pub folders: Vec<String>,
    /// "paste" simulates Cmd+V into the previous app; "copy" only fills the clipboard.
    pub paste_mode: String,
    pub restore_clipboard: bool,
    /// Opens the clipboard history. See [`crate::history`].
    #[serde(default = "default_clipboard_hotkey")]
    pub clipboard_hotkey: String,
    /// Whether to remember what you copy at all. On by default — a clipboard
    /// history that has to be switched on before it can catch the thing you
    /// already lost is not one.
    #[serde(default = "default_true")]
    pub clipboard_history: bool,
    /// How many copies to keep.
    #[serde(default = "crate::history::default_limit")]
    pub clipboard_limit: u32,
    /// Whether this user's other machines may fetch this library over
    /// Tailscale. See [`crate::tailnet`]. On by default: the whole point is
    /// that the second machine finds the first without a trip back to it, and
    /// nothing answers to anyone but devices signed in as the same user.
    #[serde(default = "default_true")]
    pub share_over_tailscale: bool,
}

/// Seeded rather than left empty so grabbing works the moment it is pressed,
/// with somewhere obvious for the results to land. Point it at your own folder
/// instead and this one stops being used.
fn default_folders() -> Vec<String> {
    vec![config_dir().join("media").to_string_lossy().to_string()]
}

/// ⌥Space on macOS. Not Alt+Space on Linux, where GNOME, KDE and most window
/// managers already hold it for the window menu, and a key another client has
/// grabbed cannot be grabbed again.
fn default_expand_hotkey() -> String {
    match cfg!(target_os = "macos") {
        true => "Alt+Space".into(),
        false => "Control+Alt+Space".into(),
    }
}

/// ⌘⇧V, which is what every clipboard manager on macOS uses. Not Ctrl+Shift+V
/// on Linux: that is paste in every terminal, and grabbing it globally would
/// take paste away from all of them. Super+Shift+V is what GNOME's clipboard
/// extensions settled on instead.
fn default_clipboard_hotkey() -> String {
    match cfg!(target_os = "macos") {
        true => "CmdOrCtrl+Shift+V".into(),
        false => "Super+Shift+V".into(),
    }
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "CmdOrCtrl+Shift+Space".into(),
            expand_hotkey: default_expand_hotkey(),
            folders: default_folders(),
            track_typing: true,
            paste_mode: "paste".into(),
            restore_clipboard: true,
            clipboard_hotkey: default_clipboard_hotkey(),
            clipboard_history: true,
            clipboard_limit: crate::history::default_limit(),
            share_over_tailscale: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub macros: Vec<Macro>,
    /// Use counts for folder-derived macros, keyed by their id.
    ///
    /// They cannot live on the macro: it is rebuilt from the folder every scan,
    /// so anything written on it is discarded. Kept here so a reference image
    /// you reach for daily still climbs the palette.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub folder_usage: std::collections::BTreeMap<String, u64>,
}

fn default_version() -> u32 {
    FORMAT_VERSION
}

impl Default for Library {
    fn default() -> Self {
        Self {
            version: FORMAT_VERSION,
            settings: Settings::default(),
            macros: seed_macros(),
            folder_usage: Default::default(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".reze")
}

pub fn library_path() -> PathBuf {
    config_dir().join("macros.json")
}

/// Tracks the exact bytes we last wrote so the file watcher can tell our own
/// saves apart from a real external edit (otherwise every save round-trips).
pub struct AppState {
    pub last_written: Mutex<String>,
    /// Whatever was frontmost when the palette opened — a PID on macOS, a
    /// window on X11 — so focus can be handed back deliberately rather than
    /// hoped for.
    pub previous_app: Mutex<Option<crate::focus::Target>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            last_written: Mutex::new(String::new()),
            previous_app: Mutex::new(None),
        }
    }
}

pub fn load() -> Result<Library, String> {
    let path = library_path();
    if !path.exists() {
        let lib = Library::default();
        save(&lib)?;
        return Ok(lib);
    }
    let raw = std::fs::read_to_string(&path).map_err(|e| format!("reading {path:?}: {e}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("parsing {path:?}: {e}"))
}

pub fn serialize(lib: &Library) -> Result<String, String> {
    serde_json::to_string_pretty(lib).map_err(|e| e.to_string())
}

pub fn save(lib: &Library) -> Result<String, String> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {dir:?}: {e}"))?;
    let body = serialize(lib)?;
    let path = library_path();
    // Write-then-rename so a crash mid-write can never truncate the library.
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &body).map_err(|e| format!("writing {tmp:?}: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("renaming into {path:?}: {e}"))?;
    Ok(body)
}

// ---------------------------------------------------------------- backups

/// How many set-aside libraries to keep.
///
/// Enough to walk back through an afternoon of importing from the wrong
/// machine; few enough that the attachments they keep alive — see
/// [`crate::assets::sweep`] — are eventually let go.
const BACKUPS: usize = 5;

pub fn backups_dir() -> PathBuf {
    config_dir().join("backups")
}

/// Set the library aside before something replaces it wholesale.
///
/// An import is the one thing in this app that rewrites macros you did not have
/// open in front of you, so it is also the one thing that leaves a copy behind:
/// the file as it is on disk, byte for byte, hand edits and all.
pub fn backup() -> Result<PathBuf, String> {
    backup_into(&library_path(), &backups_dir())
}

fn backup_into(library: &Path, dir: &Path) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {dir:?}: {e}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Zero-padded so that sorting the names sorts the backups.
    let dest = dir.join(format!("macros-{stamp:014}.json"));
    std::fs::copy(library, &dest).map_err(|e| format!("backing up to {dest:?}: {e}"))?;
    for old in backups_in(dir).into_iter().skip(BACKUPS) {
        let _ = std::fs::remove_file(old);
    }
    Ok(dest)
}

/// Every backup, newest first.
pub fn backups() -> Vec<PathBuf> {
    backups_in(&backups_dir())
}

fn backups_in(dir: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    found.retain(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("macros-") && n.ends_with(".json"))
    });
    found.sort();
    found.reverse();
    found
}

/// Put the newest backup back as the library, and return what it holds.
///
/// The backup is used up by this: undoing twice walks back two imports rather
/// than restoring the same one again.
pub fn restore_backup() -> Result<(Library, String), String> {
    let newest = backups().into_iter().next().ok_or("There is nothing to undo.")?;
    let raw = std::fs::read_to_string(&newest).map_err(|e| format!("reading {newest:?}: {e}"))?;
    // Parsed before it replaces anything: a backup that will not load is not
    // an improvement on whatever is there now.
    let library: Library =
        serde_json::from_str(&raw).map_err(|e| format!("parsing {newest:?}: {e}"))?;
    let path = library_path();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &raw).map_err(|e| format!("writing {tmp:?}: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("renaming into {path:?}: {e}"))?;
    let _ = std::fs::remove_file(&newest);
    Ok((library, raw))
}

fn m(name: &str, description: &str, tags: &[&str], body: &str) -> Macro {
    Macro {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        description: description.into(),
        tags: tags.iter().map(|s| s.to_string()).collect(),
        body: body.trim().into(),
        attachments: Vec::new(),
        folder: None,
        usage_count: 0,
    }
}

/// Starter set, written on first launch. Meant to be edited, not treated as gospel.
fn seed_macros() -> Vec<Macro> {
    vec![
        m(
            "rigor",
            "Shared preamble — include with {{> rigor}}",
            &["snippet"],
            "Be rigorous and concrete. Cite every claim with a `file_path:line` reference. \
             If you are uncertain about something, say so explicitly rather than guessing. \
             Do not summarize what you did not actually read.",
        ),
        m(
            "full analysis",
            "Deep end-to-end read of a codebase",
            &["review", "analysis"],
            "{{> rigor}}\n\nFully and thoroughly analyze {{target|this codebase}}.\n\n\
             Work through it in this order:\n\
             1. Map the overall architecture — entry points, module boundaries, and how data flows between them.\n\
             2. Identify the core abstractions and whether they actually hold, or leak.\n\
             3. Call out correctness risks: unhandled errors, race conditions, unchecked assumptions, silent failure paths.\n\
             4. Note performance characteristics and anywhere the complexity is worse than it looks.\n\
             5. Assess test coverage — not the percentage, but whether the tests would actually catch a regression.\n\
             6. Flag anything that would surprise a new contributor.\n\n\
             Prioritize findings by real impact. Skip anything cosmetic unless it causes genuine confusion.",
        ),
        m(
            "security audit",
            "Threat-model style security pass",
            &["review", "security"],
            "{{> rigor}}\n\nPerform a security review of {{target|the changes on this branch}}.\n\n\
             Cover: input validation and injection surfaces, authentication and authorization gaps, \
             secret and credential handling, unsafe deserialization, path traversal, SSRF, \
             dependency risk, and anything that crosses a trust boundary.\n\n\
             For each finding give: the concrete attack path, the severity, and the minimal fix. \
             Do not report theoretical issues that are not reachable in this code.",
        ),
        m(
            "explain like i'm new",
            "Onboarding-grade explanation",
            &["explain"],
            "Explain {{topic}} to me as if I am a competent engineer who is brand new to this specific system.\n\n\
             Start with the one-paragraph mental model I need before any detail makes sense. \
             Then work outward from there. Use concrete examples from the actual code rather than \
             generic illustrations. Name the things that commonly trip people up.",
        ),
        m(
            "refactor plan",
            "Plan a refactor without writing it yet",
            &["planning"],
            "{{> rigor}}\n\nPropose a refactor plan for {{target}}.\n\n\
             Do not write the code yet. Give me:\n\
             - What is wrong with the current structure, specifically.\n\
             - The target structure, and why it is better on dimensions that matter.\n\
             - An ordered sequence of steps where the code builds and tests pass after every step.\n\
             - What could break, and how we would know.\n\
             - What you would deliberately leave alone, and why.",
        ),
        m(
            "debug this",
            "Systematic debugging pass",
            &["debug"],
            "{{> rigor}}\n\nHelp me debug this: {{symptom}}\n\n\
             Do not guess at a fix. Instead:\n\
             1. State what the code is actually supposed to do at the failure point.\n\
             2. List the candidate causes, ordered by likelihood given the evidence.\n\
             3. For each, tell me the cheapest observation that would confirm or eliminate it.\n\
             4. Only once the cause is identified, propose the fix — and the test that would have caught it.",
        ),
        m(
            "code review",
            "Reviewer-mindset pass over a diff",
            &["review"],
            "{{> rigor}}\n\nReview {{target|the current diff}} as a demanding but fair senior reviewer.\n\n\
             Focus on correctness first, then clarity, then everything else. \
             Distinguish clearly between things that are actually broken, things that are risky, \
             and things that are merely stylistic preference — and label each.\n\n\
             If something is genuinely fine, say so rather than inventing a nitpick to seem thorough.",
        ),
        m(
            "write tests",
            "Meaningful test coverage, not coverage theater",
            &["testing"],
            "Write tests for {{target}}.\n\n\
             Prioritize the cases that would actually catch a regression: boundary conditions, \
             error paths, empty and malformed input, concurrency if relevant. \
             Skip tests that only restate the implementation.\n\n\
             Match the existing test style and helpers in this repo rather than introducing a new pattern.",
        ),
        m(
            "commit message",
            "Conventional commit for staged changes",
            &["git"],
            "Write a Conventional Commits message for the currently staged changes.\n\n\
             Read the actual staged diff first. The subject is imperative mood and under 72 characters. \
             The body explains why the change was made and any trade-offs — not a restatement of the diff. \
             Omit the body only if the change is genuinely trivial.",
        ),
        m(
            "from clipboard",
            "Wraps whatever you last copied",
            &["utility"],
            "Analyze the following and tell me what is wrong with it:\n\n```\n{{clipboard}}\n```",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reze-store-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_library_written_before_sharing_existed_still_loads_and_shares() {
        let old = r#"{"version":1,"settings":{"hotkey":"CmdOrCtrl+Shift+Space",
            "pasteMode":"paste","restoreClipboard":true},"macros":[]}"#;
        let library: Library = serde_json::from_str(old).unwrap();
        assert!(library.settings.share_over_tailscale);
    }

    #[test]
    fn a_backup_is_the_file_as_it_was() {
        let dir = scratch("backup");
        let library = dir.join("macros.json");
        std::fs::write(&library, "exactly this").unwrap();

        let copy = backup_into(&library, &dir.join("backups")).unwrap();

        assert_eq!(std::fs::read_to_string(copy).unwrap(), "exactly this");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn only_the_newest_backups_are_kept_and_they_list_newest_first() {
        let dir = scratch("prune");
        let library = dir.join("macros.json");
        let backups = dir.join("backups");
        let mut made = Vec::new();
        for n in 0..BACKUPS + 2 {
            std::fs::write(&library, n.to_string()).unwrap();
            made.push(backup_into(&library, &backups).unwrap());
            // The names are millisecond stamps; two in one would be one file.
            std::thread::sleep(std::time::Duration::from_millis(3));
        }

        let kept = backups_in(&backups);

        assert_eq!(kept.len(), BACKUPS);
        assert_eq!(kept.first(), made.last(), "the newest is not listed first");
        assert!(!made[0].exists() && !made[1].exists(), "the oldest were not pruned");
        std::fs::remove_dir_all(&dir).ok();
    }
}
