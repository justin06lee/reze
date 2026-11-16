//! Bringing another machine's library into this one.
//!
//! Getting the other library here is [`crate::tailnet`]'s business; this is
//! what happens once it has arrived, and none of it touches the network or the
//! disk — two libraries in, one out — so all of it can be tested.
//!
//! A macro on that machine and a macro on this one are **the same macro** if
//! they share an id, or failing that a trigger. The second rule is the one that
//! matters in practice: every install seeds the same starter macros under ids
//! of its own, and two machines' `full analysis` should meet as one macro
//! rather than sit side by side as two.
//!
//! Nothing here decides *whether* to import. The caller shows a [`Preview`]
//! first, and the two ways to go through with it differ only in what happens
//! to the macros this machine alone has: a merge keeps them, a replace does not.

use crate::assets::Attachment;
use crate::store::{Library, Macro, Settings, FORMAT_VERSION};
use serde::Serialize;

/// What importing would do, worked out before anything is touched.
///
/// Names rather than counts wherever something will change, because "2 macros
/// will be overwritten" is not a thing anyone can agree to without knowing
/// which two.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preview {
    /// Triggers that machine has and this one does not.
    pub added: Vec<String>,
    /// Triggers both have, with different contents — theirs would win.
    pub changed: Vec<String>,
    /// How many are identical on both. A count: there is nothing to decide.
    pub same: usize,
    /// Triggers only this machine has. Kept by a merge, removed by a replace.
    pub only_here: Vec<String>,
    /// Files that would have to be fetched, and how much that is.
    pub attachments: usize,
    pub bytes: u64,
    /// Whether the hotkeys could come along — see [`Choices::hotkeys`].
    pub hotkeys: bool,
}

/// How to go through with it.
#[derive(Debug, Clone, Copy, Default)]
pub struct Choices {
    /// Make this library a copy of theirs, dropping what only this one has.
    pub replace: bool,
    /// Take their settings as well as their macros.
    pub settings: bool,
    /// Take their hotkeys with the settings. Only ever true between two
    /// machines on the same OS: the defaults differ on purpose — `Alt+Space`
    /// is the window menu on Linux — and a Mac's bindings would land on a
    /// Linux desktop as keys that are already spoken for.
    pub hotkeys: bool,
}

/// The library an import produces, and what it took to make it.
#[derive(Debug)]
pub struct Merged {
    pub library: Library,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    /// The attachments of every macro that came from the other machine: the
    /// files that have to be here before the library that names them is saved.
    pub incoming: Vec<Attachment>,
}

/// How one of their macros relates to ours. The index is into ours.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Fate {
    New,
    Changed(usize),
    Same(usize),
}

/// Triggers are matched the way expand-in-place matches them: case ignored.
pub(crate) fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Whether two macros would do the same thing when triggered.
///
/// Use counts are not part of it — they say how often a macro was reached for
/// on one machine, not what it is — and neither is the id.
pub(crate) fn alike(a: &Macro, b: &Macro) -> bool {
    a.name == b.name
        && a.description == b.description
        && a.tags == b.tags
        && a.body == b.body
        && a.attachments.len() == b.attachments.len()
        && a.attachments.iter().zip(&b.attachments).all(|(x, y)| x.file == y.file)
}

/// Their macros, reduced to what can exist on another machine.
///
/// A library is hand-editable and this one arrived over a network, so it is
/// held to the same rules a local one is, plus one: an attachment may only name
/// a file in the asset library. A `path` is an absolute path on *that* machine,
/// which here is either nothing or — worse — something else entirely.
pub(crate) fn portable(theirs: &Library) -> Vec<Macro> {
    theirs
        .macros
        .iter()
        // Folder macros are never written to the file, so one being there
        // means someone put it there by hand. It is made of a folder this
        // machine does not have.
        .filter(|m| m.folder.is_none())
        .cloned()
        .map(|mut m| {
            m.attachments
                .retain(|a| a.path.is_none() && crate::assets::path_for(&a.file).is_ok());
            m
        })
        .collect()
}

/// Find, for each of their macros, the index of the one of ours it is.
pub(crate) fn counterparts(mine: &[Macro], theirs: &[Macro]) -> Vec<Option<usize>> {
    let mut taken = vec![false; mine.len()];
    let mut matched: Vec<Option<usize>> = vec![None; theirs.len()];

    // Ids first, all of them, before any trigger is considered: a macro
    // renamed on one machine must still find itself, and must not lose its
    // counterpart to some other macro that happens to have its old name.
    for (t, theirs) in theirs.iter().enumerate() {
        if let Some(i) = mine.iter().position(|m| m.id == theirs.id) {
            if !taken[i] {
                taken[i] = true;
                matched[t] = Some(i);
            }
        }
    }
    for (t, theirs) in theirs.iter().enumerate() {
        if matched[t].is_some() {
            continue;
        }
        let wanted = key(&theirs.name);
        if let Some(i) = (0..mine.len()).find(|&i| !taken[i] && key(&mine[i].name) == wanted) {
            taken[i] = true;
            matched[t] = Some(i);
        }
    }
    matched
}

/// Decide, for each of their macros, which of ours it is and how it differs.
fn pair(mine: &[Macro], theirs: &[Macro]) -> Vec<Fate> {
    theirs
        .iter()
        .zip(counterparts(mine, theirs))
        .map(|(theirs, at)| match at {
            None => Fate::New,
            Some(i) if alike(&mine[i], theirs) => Fate::Same(i),
            Some(i) => Fate::Changed(i),
        })
        .collect()
}

/// Whether `theirs` was written by a Reze this one can read.
///
/// Unknown fields are skipped when a library is parsed, so a newer format
/// would load without complaint and be saved back with pieces missing. Better
/// to say so than to import half of something.
pub fn check(theirs: &Library) -> Result<(), String> {
    if theirs.version > FORMAT_VERSION {
        return Err(
            "That machine runs a newer Reze than this one. Update this one, then import.".into(),
        );
    }
    Ok(())
}

/// Work out what an import would do.
///
/// `have` answers whether an attachment file is already in the asset library,
/// so that only what would actually be fetched is counted — and so that this
/// stays a function of its arguments.
pub fn preview(
    mine: &Library,
    theirs: &Library,
    hotkeys: bool,
    have: impl Fn(&str) -> bool,
) -> Preview {
    let incoming = portable(theirs);
    let fates = pair(&mine.macros, &incoming);
    let mut out = Preview {
        hotkeys,
        ..Preview::default()
    };

    let mut claimed = vec![false; mine.macros.len()];
    let mut files = std::collections::BTreeSet::new();
    for (theirs, fate) in incoming.iter().zip(&fates) {
        match *fate {
            Fate::New => out.added.push(theirs.name.clone()),
            Fate::Changed(i) => {
                claimed[i] = true;
                out.changed.push(theirs.name.clone());
            }
            Fate::Same(i) => {
                claimed[i] = true;
                out.same += 1;
            }
        }
        if !matches!(fate, Fate::Same(_)) {
            for a in &theirs.attachments {
                if !have(&a.file) && files.insert(a.file.clone()) {
                    out.attachments += 1;
                    out.bytes += a.bytes;
                }
            }
        }
    }
    out.only_here = mine
        .macros
        .iter()
        .zip(claimed)
        .filter(|(_, claimed)| !claimed)
        .map(|(m, _)| m.name.clone())
        .collect();
    out
}

/// Produce the library an import leaves behind.
pub fn merge(mine: &Library, theirs: &Library, choices: Choices) -> Merged {
    let incoming = portable(theirs);
    let fates = pair(&mine.macros, &incoming);

    let mut added = Vec::new();
    // What each of ours becomes, where theirs replaces it.
    let mut replaced: Vec<Option<Macro>> = vec![None; mine.macros.len()];
    let mut claimed = vec![false; mine.macros.len()];
    let mut fetch = Vec::new();
    let mut theirs_in_order = Vec::new();

    for (theirs, fate) in incoming.into_iter().zip(fates) {
        let arrived = match fate {
            Fate::New => {
                fetch.extend(theirs.attachments.iter().cloned());
                added.push(theirs.clone());
                theirs
            }
            Fate::Changed(i) => {
                claimed[i] = true;
                fetch.extend(theirs.attachments.iter().cloned());
                // Theirs in every respect but the count, which is this
                // machine's own record of what gets reached for here. Their id
                // too, so the two libraries agree from now on about which
                // macro this is, whatever either side renames it to.
                let adopted = Macro {
                    usage_count: mine.macros[i].usage_count,
                    ..theirs
                };
                replaced[i] = Some(adopted.clone());
                adopted
            }
            Fate::Same(i) => {
                claimed[i] = true;
                // Already here, and identical: ours stays exactly as it is.
                mine.macros[i].clone()
            }
        };
        theirs_in_order.push(arrived);
    }

    let updated = replaced.iter().flatten().count();
    let only_here = claimed.iter().filter(|c| !**c).count();

    let macros = if choices.replace {
        // Their library, in their order.
        theirs_in_order
    } else {
        // Ours, in our order, with the new ones on top — where a macro made
        // in the editor goes, and where something that just arrived is found.
        let mut out = added.clone();
        out.extend(
            mine.macros
                .iter()
                .zip(replaced)
                .map(|(ours, theirs)| theirs.unwrap_or_else(|| ours.clone())),
        );
        out
    };

    // The same file can be attached to several macros; it is fetched once.
    let mut seen = std::collections::HashSet::new();
    fetch.retain(|a| seen.insert(a.file.clone()));

    Merged {
        library: Library {
            version: FORMAT_VERSION,
            settings: match choices.settings {
                true => adopt(&mine.settings, &theirs.settings, choices.hotkeys),
                false => mine.settings.clone(),
            },
            macros,
            // Keyed by paths in this machine's folders; theirs mean nothing here.
            folder_usage: mine.folder_usage.clone(),
            deleted: mine.deleted.clone(),
        },
        added: added.len(),
        updated,
        removed: if choices.replace { only_here } else { 0 },
        incoming: fetch,
    }
}

/// Our settings, with the ones that mean the same thing on any machine taken
/// from theirs.
///
/// What stays behind is whatever describes *this* machine rather than a
/// preference: the watched folders, which are paths on it, and whether it
/// shares its library — importing from a machine must not be able to switch
/// that on.
fn adopt(mine: &Settings, theirs: &Settings, hotkeys: bool) -> Settings {
    let mut out = mine.clone();
    out.paste_mode = theirs.paste_mode.clone();
    out.restore_clipboard = theirs.restore_clipboard;
    out.track_typing = theirs.track_typing;
    out.clipboard_history = theirs.clipboard_history;
    out.clipboard_limit = theirs.clipboard_limit.clamp(1, 1000);
    if hotkeys {
        out.hotkey = theirs.hotkey.clone();
        out.expand_hotkey = theirs.expand_hotkey.clone();
        out.clipboard_hotkey = theirs.clipboard_hotkey.clone();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(id: &str, name: &str, body: &str) -> Macro {
        Macro {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            tags: Vec::new(),
            body: body.into(),
            attachments: Vec::new(),
            folder: None,
            usage_count: 0,
            updated_at: 0,
        }
    }

    fn file(name: &str) -> Attachment {
        Attachment {
            id: format!("id-{name}"),
            file: name.into(),
            path: None,
            name: name.into(),
            kind: "image".into(),
            mime: "image/png".into(),
            bytes: 10,
        }
    }

    fn lib(macros: Vec<Macro>) -> Library {
        Library {
            version: FORMAT_VERSION,
            settings: Settings::default(),
            macros,
            folder_usage: Default::default(),
            deleted: Default::default(),
        }
    }

    fn names(library: &Library) -> Vec<&str> {
        library.macros.iter().map(|m| m.name.as_str()).collect()
    }

    const MERGE: Choices = Choices {
        replace: false,
        settings: false,
        hotkeys: false,
    };
    const REPLACE: Choices = Choices {
        replace: true,
        ..MERGE
    };

    #[test]
    fn a_merge_adds_what_is_new_and_keeps_what_is_only_here() {
        let mine = lib(vec![mac("1", "mine alone", "x")]);
        let theirs = lib(vec![mac("2", "theirs alone", "y")]);

        let out = merge(&mine, &theirs, MERGE);

        assert_eq!(names(&out.library), ["theirs alone", "mine alone"]);
        assert_eq!((out.added, out.updated, out.removed), (1, 0, 0));
    }

    #[test]
    fn a_replace_leaves_exactly_their_library() {
        let mine = lib(vec![mac("1", "mine alone", "x"), mac("2", "shared", "old")]);
        let theirs = lib(vec![mac("2", "shared", "new"), mac("3", "theirs alone", "y")]);

        let out = merge(&mine, &theirs, REPLACE);

        assert_eq!(names(&out.library), ["shared", "theirs alone"]);
        assert_eq!(out.library.macros[0].body, "new");
        assert_eq!((out.added, out.updated, out.removed), (1, 1, 1));
    }

    #[test]
    fn the_same_trigger_under_another_id_is_the_same_macro() {
        // Every install seeds its starter macros under ids of its own.
        let mine = lib(vec![mac("seed-here", "Full Analysis", "ours")]);
        let theirs = lib(vec![mac("seed-there", "full analysis", "theirs")]);

        let out = merge(&mine, &theirs, MERGE);

        assert_eq!(out.library.macros.len(), 1, "the two met as two macros");
        assert_eq!(out.library.macros[0].body, "theirs");
        // Their id, so the next import pairs them without going by name.
        assert_eq!(out.library.macros[0].id, "seed-there");
        assert_eq!((out.added, out.updated), (0, 1));
    }

    #[test]
    fn an_id_match_outranks_a_trigger_match() {
        // "b" was renamed to "a" over there, and a different "a" exists here.
        // The renamed macro must find itself, not the one with its new name.
        let mine = lib(vec![mac("1", "a", "the other a"), mac("2", "b", "old")]);
        let theirs = lib(vec![mac("2", "a", "renamed")]);

        let out = merge(&mine, &theirs, MERGE);

        assert_eq!(out.library.macros.len(), 2);
        assert_eq!(out.library.macros[0].body, "the other a", "the wrong macro was replaced");
        assert_eq!(out.library.macros[1].body, "renamed");
    }

    #[test]
    fn an_identical_macro_is_left_exactly_as_it_is() {
        let mut ours = mac("here", "same", "body");
        ours.usage_count = 9;
        let mine = lib(vec![ours]);
        let theirs = lib(vec![mac("there", "same", "body")]);

        let out = merge(&mine, &theirs, MERGE);

        assert_eq!(out.library.macros[0].id, "here");
        assert_eq!(out.library.macros[0].usage_count, 9);
        assert_eq!((out.added, out.updated), (0, 0));
        assert!(out.incoming.is_empty());
    }

    #[test]
    fn use_counts_stay_this_machines_own() {
        let mut ours = mac("1", "shared", "old");
        ours.usage_count = 40;
        let mut their_shared = mac("1", "shared", "new");
        their_shared.usage_count = 2;
        let mut their_new = mac("2", "new", "y");
        their_new.usage_count = 7;

        for choices in [MERGE, REPLACE] {
            let out = merge(
                &lib(vec![ours.clone()]),
                &lib(vec![their_shared.clone(), their_new.clone()]),
                choices,
            );
            let count = |name: &str| {
                out.library.macros.iter().find(|m| m.name == name).unwrap().usage_count
            };
            assert_eq!(count("shared"), 40, "a count kept here was overwritten");
            // Nothing here to keep, so theirs is a better start than zero.
            assert_eq!(count("new"), 7);
        }
    }

    #[test]
    fn only_files_of_macros_that_actually_came_over_are_fetched() {
        let mut same = mac("1", "same", "x");
        same.attachments = vec![file("already-here.png")];
        let mut new = mac("2", "new", "y");
        new.attachments = vec![file("a.png"), file("b.png")];
        let mut also_new = mac("3", "also new", "z");
        also_new.attachments = vec![file("a.png")]; // the same file, attached twice

        let out = merge(&lib(vec![same.clone()]), &lib(vec![same, new, also_new]), MERGE);

        let fetched: Vec<&str> = out.incoming.iter().map(|a| a.file.as_str()).collect();
        assert_eq!(fetched, ["a.png", "b.png"]);
    }

    #[test]
    fn nothing_that_points_outside_the_asset_library_comes_over() {
        let mut hostile = mac("1", "hostile", "x");
        hostile.attachments = vec![
            file("fine.png"),
            file("../../.ssh/id_rsa"),
            Attachment {
                path: Some("/etc/passwd".into()),
                ..file("")
            },
        ];
        let mut derived = mac("folder:/there/cat.png", "cat", "");
        derived.folder = Some("/there".into());

        let out = merge(&lib(vec![]), &lib(vec![hostile, derived]), MERGE);

        assert_eq!(names(&out.library), ["hostile"], "a folder macro was imported");
        let kept: Vec<&str> =
            out.library.macros[0].attachments.iter().map(|a| a.file.as_str()).collect();
        assert_eq!(kept, ["fine.png"]);
        assert_eq!(out.incoming.len(), 1);
    }

    #[test]
    fn settings_stay_put_unless_asked_for() {
        let mine = lib(vec![]);
        let mut theirs = lib(vec![]);
        theirs.settings.paste_mode = "copy".into();

        assert_eq!(merge(&mine, &theirs, MERGE).library.settings.paste_mode, "paste");
        assert_eq!(merge(&mine, &theirs, REPLACE).library.settings.paste_mode, "paste");
    }

    #[test]
    fn what_describes_the_other_machine_never_comes_with_its_settings() {
        let mut mine = lib(vec![]);
        mine.settings.folders = vec!["/here/refs".into()];
        mine.settings.share_over_tailscale = false;
        mine.settings.hotkey = "Control+Shift+Space".into();
        let mut theirs = lib(vec![]);
        theirs.settings.folders = vec!["/Users/someone/refs".into()];
        theirs.settings.share_over_tailscale = true;
        theirs.settings.hotkey = "CmdOrCtrl+Shift+Space".into();
        theirs.settings.paste_mode = "copy".into();
        theirs.settings.clipboard_limit = 50_000;

        let across = Choices {
            settings: true,
            ..MERGE
        };
        let out = merge(&mine, &theirs, across).library.settings;

        assert_eq!(out.paste_mode, "copy", "a portable setting did not come over");
        assert_eq!(out.clipboard_limit, 1000, "a limit past the editor's own was taken");
        assert_eq!(out.folders, ["/here/refs"], "another machine's paths were adopted");
        assert!(!out.share_over_tailscale, "an import switched sharing on");
        assert_eq!(out.hotkey, "Control+Shift+Space", "hotkeys crossed between systems");

        let same_os = Choices {
            hotkeys: true,
            ..across
        };
        let out = merge(&mine, &theirs, same_os).library.settings;
        assert_eq!(out.hotkey, "CmdOrCtrl+Shift+Space");
    }

    #[test]
    fn the_preview_says_what_the_merge_then_does() {
        let mut changed_here = mac("1", "changed", "old");
        changed_here.attachments = vec![file("old.png")];
        let mut changed_there = mac("1", "changed", "new");
        changed_there.attachments = vec![file("have.png"), file("need.png")];
        let mine = lib(vec![changed_here, mac("2", "same", "x"), mac("3", "only here", "x")]);
        let theirs = lib(vec![changed_there, mac("2", "same", "x"), mac("4", "new", "x")]);

        let seen = preview(&mine, &theirs, true, |f| f == "have.png");

        assert_eq!(seen.added, ["new"]);
        assert_eq!(seen.changed, ["changed"]);
        assert_eq!(seen.same, 1);
        assert_eq!(seen.only_here, ["only here"]);
        assert_eq!((seen.attachments, seen.bytes), (1, 10));

        let done = merge(&mine, &theirs, REPLACE);
        assert_eq!(
            (done.added, done.updated, done.removed),
            (seen.added.len(), seen.changed.len(), seen.only_here.len()),
        );
    }

    #[test]
    fn a_library_from_a_newer_reze_is_refused() {
        let mut theirs = lib(vec![]);
        assert!(check(&theirs).is_ok());
        theirs.version = FORMAT_VERSION + 1;
        assert!(check(&theirs).is_err());
    }
}
