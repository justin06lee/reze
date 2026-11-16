//! The files a macro pastes: images, video, audio, anything else.
//!
//! Text lives inside `macros.json`; files cannot. So they are copied into
//! `~/.reze/assets/` under a generated name and referenced from the library by
//! that name alone. Copying rather than linking is deliberate — a macro must
//! keep working after the screenshot it was made from is thrown away, and the
//! whole `~/.reze` directory has to stay portable between machines.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long a freshly imported file is protected from [`sweep`].
///
/// The editor imports a file the moment it is dropped, but only writes the
/// macro that references it once the 400 ms autosave debounce elapses. Between
/// those two moments the file looks like garbage and is not. Sixty seconds is
/// far longer than that gap and far shorter than any session.
const SWEEP_GRACE: Duration = Duration::from_secs(60);

/// A file attached to a macro.
///
/// It lives in one of two places. Files you attach by hand are **copied** into
/// the assets directory and named by `file`, so the macro survives the original
/// being deleted. Files in a [watched folder](crate::folders) are **referenced
/// where they lie** by `path` — that folder is yours, and copying a hundred
/// gigabytes of video to shadow it would be absurd.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    /// Generated name inside `~/.reze/assets`, e.g. `a1b2….png`. Empty when
    /// `path` is set.
    #[serde(default)]
    pub file: String,
    /// Absolute path to a file outside the library. Set only for watched
    /// folders; takes precedence over `file`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// What the file was called when it came in, for display only.
    pub name: String,
    /// Which preview the UI should draw: `image`, `video`, `audio` or `file`.
    pub kind: String,
    pub mime: String,
    pub bytes: u64,
}

pub fn assets_dir() -> PathBuf {
    crate::store::config_dir().join("assets")
}

/// Where this attachment's bytes actually are.
pub fn resolve(attachment: &Attachment) -> Result<PathBuf, String> {
    match &attachment.path {
        // A folder attachment names an absolute path we put there ourselves
        // during the scan. Relative would mean relative to the working
        // directory, which for a launched app is `/` — never what was meant.
        Some(path) if Path::new(path).is_absolute() => Ok(PathBuf::from(path)),
        Some(path) => Err(format!("{path:?} is not an absolute path")),
        None => path_for(&attachment.file),
    }
}

/// Resolve a stored `file` name to a real path.
///
/// The library file is user-editable by design, so this is the boundary where a
/// hand-written `"file": "../../.ssh/id_rsa"` has to stop. Anything that is not
/// a plain file name is refused outright rather than normalised, because there
/// is no legitimate reason for one to appear.
pub fn path_for(file: &str) -> Result<PathBuf, String> {
    bare_name(&assets_dir(), file)
}

/// The same check, against any directory we own — the clipboard history keeps
/// its images the same way, and its index is just as hand-editable.
pub fn bare_name(dir: &Path, file: &str) -> Result<PathBuf, String> {
    let bare = Path::new(file);
    let mut parts = bare.components();
    let valid = matches!(parts.next(), Some(std::path::Component::Normal(_))) && parts.next().is_none();
    if !valid || file.is_empty() {
        return Err(format!("{file:?} is not a file name"));
    }
    Ok(dir.join(file))
}

/// Copy a file on disk into the assets directory.
pub fn import_path(src: &Path) -> Result<Attachment, String> {
    let meta = std::fs::metadata(src).map_err(|e| format!("reading {src:?}: {e}"))?;
    if meta.is_dir() {
        return Err(format!(
            "{} is a folder — attach the files inside it instead",
            display_name(src)
        ));
    }

    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let dest = fresh_path(&ext)?;
    std::fs::copy(src, &dest).map_err(|e| format!("copying into {dest:?}: {e}"))?;

    Ok(describe(&dest, &display_name(src), &ext, meta.len()))
}

/// Write bytes handed to us — a screenshot off the pasteboard, say — into the
/// assets directory under `name`.
pub fn import_bytes(name: &str, ext: &str, data: &[u8]) -> Result<Attachment, String> {
    let ext = ext.trim_start_matches('.').to_lowercase();
    let dest = fresh_path(&ext)?;
    std::fs::write(&dest, data).map_err(|e| format!("writing {dest:?}: {e}"))?;
    Ok(describe(&dest, name, &ext, data.len() as u64))
}

fn fresh_path(ext: &str) -> Result<PathBuf, String> {
    let dir = assets_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {dir:?}: {e}"))?;
    let stem = uuid::Uuid::new_v4().to_string();
    Ok(dir.join(if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }))
}

fn describe(dest: &Path, name: &str, ext: &str, bytes: u64) -> Attachment {
    let mime = mime_for(ext);
    Attachment {
        id: uuid::Uuid::new_v4().to_string(),
        file: dest
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string(),
        path: None,
        name: name.to_string(),
        kind: kind_for(&mime).to_string(),
        mime,
        bytes,
    }
}

/// An attachment for a file that stays where it is — see [`Attachment`].
pub fn referencing(path: &Path, bytes: u64) -> Attachment {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let mime = mime_for(&ext);
    Attachment {
        // Derived from the path, not random: a folder is rescanned constantly,
        // and a fresh id every time would make React rebuild every preview.
        id: format!("folder:{}", path.display()),
        file: String::new(),
        path: Some(path.to_string_lossy().to_string()),
        name: display_name(path),
        kind: kind_for(&mime).to_string(),
        mime,
        bytes,
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file")
        .to_string()
}

/// Which preview to draw. Derived from the MIME type rather than stored
/// separately so a library written by hand only has to get the extension right.
fn kind_for(mime: &str) -> &'static str {
    match mime.split('/').next().unwrap_or("") {
        "image" => "image",
        "video" => "video",
        "audio" => "audio",
        _ => "file",
    }
}

/// Extension to MIME, covering what a person actually pastes.
///
/// A table rather than a crate: the list is short, the fallback is harmless,
/// and the only consumers are an `<img>`/`<video>` tag and the pasteboard —
/// neither of which trusts us over sniffing the bytes themselves.
fn mime_for(ext: &str) -> String {
    let mime = match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "heic" | "heif" => "image/heic",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "ico" => "image/vnd.microsoft.icon",

        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",

        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",

        "pdf" => "application/pdf",
        "json" => "application/json",
        "zip" => "application/zip",
        "txt" | "md" => "text/plain",
        "csv" => "text/csv",
        "html" => "text/html",

        _ => "application/octet-stream",
    };
    mime.to_string()
}

/// The extension conventionally used for a pasteboard type, for data that
/// arrives with no filename attached.
pub fn ext_for_mime(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/tiff" => "tiff",
        "application/pdf" => "pdf",
        _ => "bin",
    }
}

/// Delete asset files no macro refers to any more.
///
/// Runs after an in-app save and nowhere else. That is the only moment the
/// reference set provably shrank — an *external* edit to `macros.json` reaches
/// us through the file watcher, where a text editor mid-write can momentarily
/// look like a library with no macros in it at all. Sweeping on that would
/// delete every attachment the user has over a transient empty read, so
/// orphans from hand-editing simply wait for the next save in the app.
///
/// What a [backup](crate::store::backup) names counts as referenced too. A
/// library set aside before an import is only worth having if the files it
/// points at are still there when it is put back.
pub fn sweep(library: &crate::store::Library) {
    let backups: Vec<crate::store::Library> = crate::store::backups()
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .filter_map(|raw| serde_json::from_str(&raw).ok())
        .collect();
    let referenced: std::collections::HashSet<&str> = std::iter::once(library)
        .chain(&backups)
        .flat_map(|l| l.macros.iter())
        .flat_map(|m| m.attachments.iter())
        .map(|a| a.file.as_str())
        .collect();
    sweep_dir(&assets_dir(), &referenced);
}

/// The sweep itself, against an explicit directory so it can be tested without
/// pointing at the real asset library.
fn sweep_dir(dir: &Path, referenced: &std::collections::HashSet<&str>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // nothing imported yet
    };

    let now = SystemTime::now();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if referenced.contains(name.as_str()) {
            continue;
        }
        let fresh = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_none_or(|age| age < SWEEP_GRACE);
        if fresh {
            continue;
        }
        let _ = std::fs::remove_file(entry.path());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_plain_file_name() {
        assert_eq!(path_for("a.png").unwrap(), assets_dir().join("a.png"));
    }

    #[test]
    fn refuses_anything_that_could_escape_the_directory() {
        for hostile in ["../secret", "a/b.png", "/etc/passwd", "..", ".", ""] {
            assert!(path_for(hostile).is_err(), "{hostile:?} was allowed through");
        }
    }

    /// A directory of files with controlled ages, in a fresh temp location.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reze-sweep-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Backdate a file so the grace period no longer protects it.
    fn age(path: &Path) {
        let old = SystemTime::now() - SWEEP_GRACE * 2;
        std::fs::File::open(path)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }

    #[test]
    fn sweeps_orphans_and_keeps_what_is_referenced() {
        let dir = scratch("orphans");
        for name in ["kept.png", "orphan.png"] {
            std::fs::write(dir.join(name), b"x").unwrap();
            age(&dir.join(name));
        }

        sweep_dir(&dir, &["kept.png"].into_iter().collect());

        assert!(dir.join("kept.png").exists(), "a referenced file was deleted");
        assert!(!dir.join("orphan.png").exists(), "an orphan survived");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn spares_a_file_that_was_only_just_imported() {
        // The editor writes the file the moment it is dropped and the macro
        // that references it 400 ms later. In between it looks like an orphan.
        let dir = scratch("grace");
        std::fs::write(dir.join("just-dropped.png"), b"x").unwrap();

        sweep_dir(&dir, &Default::default());

        assert!(
            dir.join("just-dropped.png").exists(),
            "a file imported seconds ago was swept before its macro could be saved",
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn kinds_follow_the_mime_family() {
        assert_eq!(kind_for(&mime_for("png")), "image");
        assert_eq!(kind_for(&mime_for("mov")), "video");
        assert_eq!(kind_for(&mime_for("m4a")), "audio");
        assert_eq!(kind_for(&mime_for("pdf")), "file");
        // Unknown extensions must still produce a usable attachment.
        assert_eq!(kind_for(&mime_for("sketch")), "file");
    }
}
