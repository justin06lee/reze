//! Watched folders: files that become macros without anyone adding them.
//!
//! Point Reze at a folder of reference images and every file in it is a macro
//! you can trigger, named after the file. Add a file and it appears; delete one
//! and it goes. The folder is the source of truth for these, exactly as
//! `macros.json` is for the ones you wrote — which is why they are rebuilt from
//! scratch on every scan and never written back to the library file.
//!
//! Their files are referenced where they lie rather than copied. That folder is
//! the user's, and shadowing a hundred gigabytes of video inside `~/.reze`
//! would be indefensible.

use crate::assets;
use crate::store::Macro;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Stop after this many files across all folders.
///
/// Someone will eventually point this at their home directory, and the failure
/// mode should be "the first two thousand files" rather than a palette that
/// takes a minute to open.
const MAX_FILES: usize = 2000;

/// How far down to walk. Deep enough for any way a person organises media,
/// shallow enough that a symlink loop cannot outlive the scan.
const MAX_DEPTH: usize = 8;

/// The id a file gets, and the key its use count is stored under.
///
/// Derived from the path rather than random because the macro is rebuilt on
/// every scan: a fresh id each time would reset the count and make the palette
/// re-render every row on every file event.
pub fn id_for(path: &Path) -> String {
    format!("folder:{}", path.display())
}

/// Turn the files in `folders` into macros.
pub fn scan(folders: &[String], usage: &BTreeMap<String, u64>) -> Vec<Macro> {
    let mut out = Vec::new();
    for folder in folders {
        let root = PathBuf::from(shellexpand_home(folder));
        if !root.is_dir() {
            continue; // an unmounted drive, or a folder since deleted
        }
        walk(&root, &root, 0, &mut out, usage);
    }
    // Stable order regardless of how the filesystem hands entries back,
    // otherwise the palette list reshuffles on every rescan.
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    out
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<Macro>, usage: &BTreeMap<String, u64>) {
    if depth > MAX_DEPTH || out.len() >= MAX_FILES {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };

        // Dotfiles are configuration and metadata, never something you meant to
        // paste — `.DS_Store` in particular is in every folder macOS has opened.
        if name.starts_with('.') {
            continue;
        }

        // `file_type` rather than `metadata`: it does not follow symlinks, so a
        // link pointing at an ancestor is skipped instead of looping forever.
        let Ok(kind) = entry.file_type() else { continue };
        if kind.is_dir() {
            walk(root, &path, depth + 1, out, usage);
        } else if kind.is_file() {
            if let Some(m) = macro_for(root, &path, usage) {
                out.push(m);
            }
        }
    }
}

fn macro_for(root: &Path, path: &Path, usage: &BTreeMap<String, u64>) -> Option<Macro> {
    let bytes = std::fs::metadata(path).ok()?.len();
    let stem = path.file_stem()?.to_str()?.to_string();
    let relative = path.strip_prefix(root).ok()?;

    // Subfolders become tags, so a folder you have already organised is a
    // library you can already filter — `memes/cats/tabby.png` is `#memes #cats`.
    let tags: Vec<String> = relative
        .parent()
        .map(|p| {
            p.components()
                .filter_map(|c| c.as_os_str().to_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let id = id_for(path);
    Some(Macro {
        usage_count: usage.get(&id).copied().unwrap_or(0),
        id,
        name: stem,
        // Where it came from, which is the only way to tell two files with the
        // same name in different subfolders apart in the palette.
        description: relative.display().to_string(),
        tags,
        // Nothing to expand: the file is the whole payload.
        body: String::new(),
        attachments: vec![assets::referencing(path, bytes)],
        folder: Some(root.to_string_lossy().to_string()),
    })
}

/// Expand a leading `~`, since a hand-edited library will contain one.
fn shellexpand_home(path: &str) -> String {
    let Some(rest) = path.strip_prefix('~') else {
        return path.to_string();
    };
    let Some(home) = dirs::home_dir() else {
        return path.to_string();
    };
    format!("{}{}", home.display(), rest)
}

/// The folders that actually exist, expanded — what the watcher should watch.
pub fn existing(folders: &[String]) -> Vec<PathBuf> {
    folders
        .iter()
        .map(|f| PathBuf::from(shellexpand_home(f)))
        .filter(|p| p.is_dir())
        .collect()
}

/// Where a grab should be saved: the first watched folder, created if need be.
///
/// The first rather than a separate setting, so a download and the macro it
/// becomes are the same act — it lands in a watched folder and the scan picks
/// it up, with nothing else to configure.
pub fn download_dir(folders: &[String]) -> Result<PathBuf, String> {
    let first = folders
        .first()
        .ok_or("No folder to save into — add one under Settings → Folders.")?;
    let dir = PathBuf::from(shellexpand_home(first));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {dir:?}: {e}"))?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("reze-folders-{name}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn turns_files_into_macros_named_after_them() {
        let dir = scratch("basic");
        std::fs::write(dir.join("tabby.png"), b"x").unwrap();

        let found = scan(&[dir.to_string_lossy().to_string()], &Default::default());

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "tabby");
        assert_eq!(found[0].body, "");
        assert_eq!(found[0].attachments.len(), 1);
        assert_eq!(found[0].folder.as_deref(), Some(&*dir.to_string_lossy()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn subfolders_become_tags() {
        let dir = scratch("tags");
        std::fs::create_dir_all(dir.join("memes/cats")).unwrap();
        std::fs::write(dir.join("memes/cats/tabby.png"), b"x").unwrap();

        let found = scan(&[dir.to_string_lossy().to_string()], &Default::default());

        assert_eq!(found[0].tags, vec!["memes", "cats"]);
        assert_eq!(found[0].description, "memes/cats/tabby.png");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn skips_dotfiles() {
        let dir = scratch("dotfiles");
        std::fs::write(dir.join(".DS_Store"), b"x").unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/config"), b"x").unwrap();

        assert!(scan(&[dir.to_string_lossy().to_string()], &Default::default()).is_empty());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ids_are_stable_across_scans_so_use_counts_survive() {
        let dir = scratch("stable");
        std::fs::write(dir.join("a.png"), b"x").unwrap();
        let folders = [dir.to_string_lossy().to_string()];

        let first = scan(&folders, &Default::default());
        let counted: BTreeMap<_, _> = [(first[0].id.clone(), 7)].into_iter().collect();
        let second = scan(&folders, &counted);

        assert_eq!(first[0].id, second[0].id);
        assert_eq!(second[0].usage_count, 7, "a use count did not survive a rescan");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_folder_is_not_an_error() {
        // An external drive gets unplugged; the rest of the library must load.
        assert!(scan(&["/nonexistent/reze".into()], &Default::default()).is_empty());
    }
}
