//! Macro persistence.
//!
//! The JSON file at `~/.reze/macros.json` is the source of truth, not the GUI.
//! It is watched for external edits so hand-editing in a text editor and
//! editing in the app both work, and neither clobbers the other.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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
    /// "paste" simulates Cmd+V into the previous app; "copy" only fills the clipboard.
    pub paste_mode: String,
    pub restore_clipboard: bool,
}

fn default_expand_hotkey() -> String {
    "Alt+Space".into()
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: "CmdOrCtrl+Shift+Space".into(),
            expand_hotkey: default_expand_hotkey(),
            track_typing: true,
            paste_mode: "paste".into(),
            restore_clipboard: true,
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
    /// PID of whatever was frontmost when the palette opened, so focus can be
    /// handed back deliberately rather than hoped for.
    pub previous_app: Mutex<Option<i32>>,
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

fn m(name: &str, description: &str, tags: &[&str], body: &str) -> Macro {
    Macro {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.into(),
        description: description.into(),
        tags: tags.iter().map(|s| s.to_string()).collect(),
        body: body.trim().into(),
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
