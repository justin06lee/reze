//! Keeping one library on several machines.
//!
//! An import is something you do once, on purpose, looking at what will
//! change. Sync is the same merge done continuously and unattended, which
//! means it has to be right without anyone looking — and "right" comes down to
//! one property: **every machine, shown the same two libraries, picks the same
//! result.** Then it does not matter who syncs with whom, in what order, or how
//! often; they all end up in the same place and stay there.
//!
//! Three things make that true, and they are all of this module:
//!
//! - **Every macro carries when it last changed** ([`stamp`]). Where two
//!   machines both have a macro and disagree, the later change wins, whole —
//!   there is no merging inside a macro.
//! - **A deletion is remembered**, as the id and the time. A macro that is
//!   merely absent looks, to a machine that still has it, exactly like one
//!   this machine has not heard of yet, and would be handed straight back.
//!   Remembered, it is a change like any other: newer than the macro, it
//!   removes it; older than a later edit, the edit wins.
//! - **Ties are broken by what the macro says**, not by which side is asking
//!   ([`rank`]). Two machines that edited the same macro in the same
//!   millisecond still agree on a winner.
//!
//! None of it touches the network or the disk. [`crate::tailnet`] carries the
//! libraries about; this only decides what two of them make.

use crate::assets::Attachment;
use crate::merge::{alike, counterparts, portable};
use crate::store::{Library, Macro, Settings};
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap, HashSet};

/// How long a deletion is remembered.
///
/// Long enough for a laptop that spent the summer in a drawer to hear about
/// it; not forever, or the list only ever grows. A machine away for longer
/// than this brings its deleted macros back with it, once.
const REMEMBER_DELETIONS: u64 = 180 * 24 * 60 * 60 * 1000;

/// Now, in the unit the stamps are in: milliseconds since 1970.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Record what changed between two versions of this machine's own library.
///
/// `before` is the library as it was and `after` as it is about to be saved.
/// Every macro whose contents differ, and every new one, is marked as changed
/// `now`; every one that is gone is remembered as deleted `now`. The stamps on
/// `after` as it arrived are ignored — the editor sends back whatever it was
/// last shown, and what was on disk is the authority on when a thing changed.
///
/// Done on every save whether or not sync is on, so that switching it on
/// later finds a library that already knows its own history.
///
/// `now` is this machine's clock, and no two machines' clocks agree. So a
/// change is dated `now` or just after whatever it changes, whichever is
/// later: an edit made here is an edit *of* the version that was here, and
/// has to outrank it even when that version came from a machine whose clock
/// runs ahead.
pub fn stamp(before: &Library, after: &mut Library, now: u64) {
    let was: HashMap<&str, &Macro> = before.macros.iter().map(|m| (m.id.as_str(), m)).collect();
    // Folder macros are rebuilt from their folder and never saved or synced.
    for m in after.macros.iter_mut().filter(|m| m.folder.is_none()) {
        m.updated_at = match (was.get(m.id.as_str()), before.deleted.get(&m.id)) {
            (Some(old), _) if alike(old, m) => old.updated_at,
            (Some(old), _) => now.max(old.updated_at.saturating_add(1)),
            // Back after being deleted, which is a change to the deletion.
            (None, Some(deleted)) => now.max(deleted.saturating_add(1)),
            (None, None) => now,
        };
    }

    let here: HashSet<&str> = after.macros.iter().map(|m| m.id.as_str()).collect();
    let mut deleted = before.deleted.clone();
    for (id, at) in &after.deleted {
        let known = deleted.entry(id.clone()).or_insert(0);
        *known = (*known).max(*at);
    }
    for m in before.macros.iter().filter(|m| m.folder.is_none()) {
        if !here.contains(m.id.as_str()) {
            deleted.insert(m.id.clone(), now.max(m.updated_at));
        }
    }
    // Something that is back is not deleted: it was stamped `now` above, which
    // already outranks the deletion, so the record of it is only clutter.
    deleted.retain(|id, at| !here.contains(id.as_str()) && fresh(*at, now));
    after.deleted = deleted;
}

fn fresh(at: u64, now: u64) -> bool {
    at.saturating_add(REMEMBER_DELETIONS) > now
}

/// What the editor did since it last saved: these macros added or edited,
/// those deleted, and the settings if one of them was changed.
///
/// Not the library as the window has it. A window only knows what it was last
/// shown, and by the time it saves, another machine may have added a macro it
/// never saw — which, in a whole library sent back, would look exactly like a
/// macro the user had just deleted, and be deleted everywhere. Saying what was
/// *done* leaves nothing to infer.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Changes {
    #[serde(default)]
    pub macros: Vec<Macro>,
    #[serde(default)]
    pub removed: Vec<String>,
    #[serde(default)]
    pub settings: Option<Settings>,
}

/// The library on disk, with what the editor did done to it and dated.
///
/// Everything the editor did not touch stays as the disk has it, whatever the
/// window believed about it.
pub fn apply(before: &Library, changes: Changes, now: u64) -> Library {
    let Changes {
        macros,
        removed,
        settings,
    } = changes;
    let mut after = before.clone();
    after.macros.retain(|m| !removed.contains(&m.id));

    let mut added = Vec::new();
    // Folder macros are rebuilt from the folder on every scan. Writing one
    // here would persist a snapshot that goes stale the moment a file is
    // renamed, and that the next scan would then duplicate.
    for mut edited in macros.into_iter().filter(|m| m.folder.is_none()) {
        if removed.contains(&edited.id) {
            continue;
        }
        match after.macros.iter_mut().find(|m| m.id == edited.id) {
            Some(known) => {
                // The palette counts uses behind the editor's back.
                edited.usage_count = known.usage_count;
                *known = edited;
            }
            None => added.push(edited),
        }
    }
    // On top, which is where the editor shows a macro it has just made.
    added.append(&mut after.macros);
    after.macros = added;

    if let Some(settings) = settings {
        after.settings = Settings {
            // Set through a command of their own, and so never by a window
            // that may not have heard of the latest list.
            folders: before.settings.folders.clone(),
            ..settings
        };
    }
    stamp(before, &mut after, now);
    after
}

/// What a macro says, as a number that is the same on every machine and in
/// every build — which the standard library's hasher does not promise to be.
/// FNV-1a, with a separator between fields so that moving a character from
/// the end of one to the start of the next is still a different macro.
fn fingerprint(m: &Macro) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |text: &str| {
        for byte in text.bytes().chain(std::iter::once(0xff)) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    feed(&m.name);
    feed(&m.description);
    feed(&m.body);
    m.tags.iter().for_each(|tag| feed(tag));
    feed("");
    m.attachments.iter().for_each(|a| feed(&a.file));
    hash
}

/// What a library holds, as a few characters that are the same on every
/// machine holding the same thing — and, near enough to always, different on
/// any holding something else.
///
/// Two machines compare these before anything is fetched, which is what makes
/// asking every minute cheap. Everything [`reconcile`] goes by is in it and
/// nothing else is: not the order the macros are in, not the use counts, not
/// the settings.
pub fn revision(library: &Library) -> String {
    let mut macros: Vec<(&str, u64, u64)> = library
        .macros
        .iter()
        .filter(|m| m.folder.is_none())
        .map(|m| (m.id.as_str(), m.updated_at, fingerprint(m)))
        .collect();
    macros.sort_unstable();

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes.iter().chain(&[0xff]) {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for (id, at, says) in macros {
        feed(id.as_bytes());
        feed(&at.to_be_bytes());
        feed(&says.to_be_bytes());
    }
    feed(b"deleted");
    for (id, at) in &library.deleted {
        feed(id.as_bytes());
        feed(&at.to_be_bytes());
    }
    format!("{hash:016x}")
}

/// Where a version of a macro stands against another version of it. The
/// greater one wins, on every machine alike.
///
/// When it changed comes first. A macro from before stamps existed has none,
/// so it is placed by what it is instead: an untouched starter macro is the
/// oldest version there can be, and anything else unstamped is newer than
/// that and older than every real stamp. That is what lets a fresh install's
/// starter set meet a library someone has been editing for a year and lose.
///
/// After that, what it says, then its id — arbitrary, but the same arbitrary
/// answer everywhere, which is the only thing a tie-break is for.
fn rank(m: &Macro) -> (u64, u64, &str) {
    let when = match m.updated_at {
        0 if crate::store::is_seed(m) => 0,
        0 => 1,
        stamped => stamped,
    };
    (when, fingerprint(m), m.id.as_str())
}

/// What syncing with another library comes to.
#[derive(Debug)]
pub struct Reconciled {
    pub library: Library,
    /// Whether anything here is different for it. When not, nothing is
    /// written — which is also what stops two machines that already agree
    /// from waking each other for ever.
    pub changed: bool,
    /// Attachments of macros that came from the other side: the files that
    /// have to be here before the library that names them is saved.
    pub incoming: Vec<Attachment>,
}

/// Bring this machine's library and another's to the same thing.
///
/// Macros are paired the way an import pairs them — by id, then by trigger —
/// and for each pair the greater [`rank`] is kept. What only they have is
/// added and what only we have stays, unless a deletion newer than the macro
/// says otherwise. Use counts, settings and everything else that describes
/// one machine rather than the library are left alone.
pub fn reconcile(mine: &Library, theirs: &Library, now: u64) -> Reconciled {
    let arriving = portable(theirs);

    let mut deleted: BTreeMap<String, u64> = mine.deleted.clone();
    for (id, at) in &theirs.deleted {
        let known = deleted.entry(id.clone()).or_insert(0);
        *known = (*known).max(*at);
    }
    // Forgotten on the same day by both, or one would keep handing the other
    // back what it had just let go of.
    deleted.retain(|_, at| fresh(*at, now));
    let dead = |m: &Macro| deleted.get(&m.id).is_some_and(|at| *at >= m.updated_at);

    let mut theirs_of: Vec<Option<&Macro>> = vec![None; mine.macros.len()];
    let mut macros = Vec::new();
    let mut incoming = Vec::new();
    for (theirs, ours) in arriving.iter().zip(counterparts(&mine.macros, &arriving)) {
        match ours {
            Some(i) => theirs_of[i] = Some(theirs),
            // New here, and on top — where a macro made in the editor goes.
            None if !dead(theirs) => {
                incoming.extend(theirs.attachments.iter().cloned());
                macros.push(theirs.clone());
            }
            None => {}
        }
    }
    for (ours, theirs) in mine.macros.iter().zip(theirs_of) {
        let kept = match theirs {
            Some(theirs) if rank(theirs) > rank(ours) => {
                incoming.extend(theirs.attachments.iter().cloned());
                // Theirs, id and stamp included, so that both sides now mean
                // the same thing by it. The count is this machine's own.
                Macro {
                    usage_count: ours.usage_count,
                    ..theirs.clone()
                }
            }
            _ => ours.clone(),
        };
        if !dead(&kept) {
            macros.push(kept);
        }
    }

    let mut seen = HashSet::new();
    incoming.retain(|a| seen.insert(a.file.clone()));

    let changed = deleted != mine.deleted
        || macros.len() != mine.macros.len()
        || macros.iter().zip(&mine.macros).any(|(now, was)| {
            now.id != was.id || now.updated_at != was.updated_at || !alike(now, was)
        });

    Reconciled {
        library: Library {
            macros,
            deleted,
            ..mine.clone()
        },
        changed,
        incoming,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Settings, FORMAT_VERSION};

    const NOW: u64 = 1_800_000_000_000;

    fn mac(id: &str, name: &str, body: &str, at: u64) -> Macro {
        Macro {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            tags: Vec::new(),
            body: body.into(),
            attachments: Vec::new(),
            folder: None,
            usage_count: 0,
            updated_at: at,
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

    /// What a library holds, in a form two of them can be compared by.
    fn contents(library: &Library) -> Vec<(String, String, String, u64)> {
        let mut all: Vec<_> = library
            .macros
            .iter()
            .map(|m| (m.id.clone(), m.name.clone(), m.body.clone(), m.updated_at))
            .collect();
        all.sort();
        all
    }

    /// Sync both ways and insist the two machines now agree, and that syncing
    /// again changes nothing — the property everything else rests on.
    fn settle(a: &Library, b: &Library) -> Library {
        let at_a = reconcile(a, b, NOW).library;
        let at_b = reconcile(b, a, NOW).library;
        assert_eq!(contents(&at_a), contents(&at_b), "the two machines disagree");
        assert_eq!(at_a.deleted, at_b.deleted, "the two machines disagree on deletions");
        assert_eq!(revision(&at_a), revision(&at_b), "agreeing machines do not look it");
        assert!(!reconcile(&at_a, &at_b, NOW).changed, "agreeing machines kept changing");
        assert!(!reconcile(&at_b, &at_a, NOW).changed, "agreeing machines kept changing");
        at_a
    }

    #[test]
    fn what_either_machine_added_ends_up_on_both() {
        let a = lib(vec![mac("1", "from a", "x", 10)]);
        let b = lib(vec![mac("2", "from b", "y", 20)]);

        let both = settle(&a, &b);

        assert_eq!(both.macros.len(), 2);
    }

    #[test]
    fn the_later_edit_of_a_macro_wins_on_both() {
        let a = lib(vec![mac("1", "shared", "edited first", 100)]);
        let b = lib(vec![mac("1", "shared", "edited last", 200)]);

        let both = settle(&a, &b);

        assert_eq!(both.macros[0].body, "edited last");
    }

    #[test]
    fn a_deletion_reaches_the_machine_that_still_has_the_macro() {
        let mut a = lib(vec![]);
        a.deleted.insert("1".into(), NOW - 700);
        let b = lib(vec![mac("1", "deleted on a", "x", NOW - 900)]);

        let both = settle(&a, &b);

        assert!(both.macros.is_empty(), "a deleted macro was handed back");
    }

    #[test]
    fn an_edit_made_after_a_deletion_brings_the_macro_back() {
        let mut a = lib(vec![]);
        a.deleted.insert("1".into(), NOW - 700);
        let b = lib(vec![mac("1", "kept after all", "edited since", NOW - 600)]);

        let both = settle(&a, &b);

        assert_eq!(both.macros.len(), 1);
    }

    #[test]
    fn a_fresh_installs_starter_macros_lose_to_ones_someone_has_edited() {
        // Neither side has a stamp: both were written before stamps existed.
        // Ids differ, because every install makes up its own.
        let seed = crate::store::Library::default().macros[0].clone();
        assert!(crate::store::is_seed(&seed));
        let mut edited = seed.clone();
        edited.id = "the-laptops-own-id".into();
        edited.body = "a year of tuning".into();

        let both = settle(&lib(vec![seed]), &lib(vec![edited]));

        assert_eq!(both.macros.len(), 1, "the two met as two macros");
        assert_eq!(both.macros[0].body, "a year of tuning");
    }

    #[test]
    fn a_tie_goes_the_same_way_on_both_machines() {
        // Same macro, edited on both in the same millisecond.
        let a = lib(vec![mac("1", "shared", "one way", 500)]);
        let b = lib(vec![mac("1", "shared", "the other", 500)]);

        settle(&a, &b);
        settle(&b, &a);
    }

    #[test]
    fn use_counts_are_not_synced() {
        let mut ours = mac("1", "shared", "old", 100);
        ours.usage_count = 40;
        let mut theirs = mac("1", "shared", "new", 200);
        theirs.usage_count = 2;

        let out = reconcile(&lib(vec![ours]), &lib(vec![theirs]), NOW);

        assert_eq!(out.library.macros[0].body, "new");
        assert_eq!(out.library.macros[0].usage_count, 40);
    }

    #[test]
    fn nothing_is_written_when_nothing_differs() {
        let a = lib(vec![mac("1", "same", "x", 100)]);
        let mut b = a.clone();
        b.macros[0].usage_count = 9; // theirs to keep, not ours to copy

        assert!(!reconcile(&a, &b, NOW).changed);
    }

    #[test]
    fn deletions_are_forgotten_by_both_sides_at_once() {
        let mut a = lib(vec![]);
        a.deleted.insert("long-gone".into(), NOW - REMEMBER_DELETIONS - 1);
        a.deleted.insert("recent".into(), NOW - 1000);

        let both = settle(&a, &lib(vec![]));

        assert_eq!(both.deleted.keys().collect::<Vec<_>>(), ["recent"]);
    }

    #[test]
    fn saving_stamps_what_changed_and_only_that() {
        let before = lib(vec![
            mac("kept", "kept", "same", 100),
            mac("edited", "edited", "old", 100),
            mac("removed", "removed", "x", 100),
        ]);
        // As the editor would send it: stale or missing stamps, one macro
        // changed, one gone, one new.
        let mut after = lib(vec![
            mac("kept", "kept", "same", 0),
            mac("edited", "edited", "new", 100),
            mac("added", "added", "y", 0),
        ]);

        stamp(&before, &mut after, NOW);

        let at = |id: &str| after.macros.iter().find(|m| m.id == id).unwrap().updated_at;
        assert_eq!(at("kept"), 100, "an untouched macro was marked as changed");
        assert_eq!(at("edited"), NOW);
        assert_eq!(at("added"), NOW);
        assert_eq!(after.deleted.get("removed"), Some(&NOW));
    }

    #[test]
    fn a_change_outranks_what_it_changed_whatever_the_clocks_say() {
        // Both arrived from a machine whose clock is an hour fast.
        let ahead = NOW + 3_600_000;
        let mut before = lib(vec![
            mac("edited", "edited", "old", ahead),
            mac("removed", "removed", "x", ahead),
        ]);
        before.deleted.insert("restored".into(), ahead);
        let mut after = lib(vec![
            mac("edited", "edited", "new", ahead),
            mac("restored", "restored", "y", 0),
        ]);

        stamp(&before, &mut after, NOW);

        // Each of them, met by the library it was made from, wins.
        let settled = settle(&after, &before);
        let bodies: Vec<_> = settled.macros.iter().map(|m| m.body.as_str()).collect();
        assert!(bodies.contains(&"new"), "an edit lost to what it edited");
        assert!(bodies.contains(&"y"), "a macro put back stayed deleted");
        assert!(!bodies.contains(&"x"), "a deleted macro came back");
    }

    #[test]
    fn a_macro_that_comes_back_is_no_longer_deleted() {
        let mut before = lib(vec![]);
        before.deleted.insert("back".into(), NOW - 1000);
        let mut after = lib(vec![mac("back", "restored", "x", NOW - 2000)]);

        stamp(&before, &mut after, NOW);

        assert!(after.deleted.is_empty());
        // And it outranks the deletion on any machine that still remembers it.
        let mut elsewhere = lib(vec![]);
        elsewhere.deleted.insert("back".into(), NOW - 1000);
        assert_eq!(settle(&after, &elsewhere).macros.len(), 1);
    }

    #[test]
    fn reads_a_save_the_way_the_editor_sends_it() {
        // Word for word what the window puts on the wire: see `Changes` in
        // types.ts. An edit and a deletion, then a setting on its own.
        let edit = r#"{"macros":[{"id":"1","name":"first","description":"","tags":[],
            "body":"one, edited","attachments":[],"usageCount":0,"updatedAt":100}],
            "removed":["3"],"settings":null}"#;
        let changes: Changes = serde_json::from_str(edit).unwrap();
        assert_eq!(changes.macros[0].body, "one, edited");
        assert_eq!(changes.removed, ["3"]);
        assert!(changes.settings.is_none());

        let setting = r#"{"macros":[],"removed":[],"settings":{
            "hotkey":"CmdOrCtrl+Shift+Space","expandHotkey":"Alt+Space","folders":[],
            "trackTyping":true,"pasteMode":"paste","restoreClipboard":true,
            "clipboardHotkey":"CmdOrCtrl+Shift+V","clipboardHistory":true,
            "clipboardLimit":200,"shareOverTailscale":true,"sync":true}}"#;
        let changes: Changes = serde_json::from_str(setting).unwrap();
        assert!(changes.settings.unwrap().syncs());
    }

    #[test]
    fn a_save_changes_only_what_the_editor_touched() {
        // The disk has a macro the window never saw: it arrived from another
        // machine after the window was last shown the library.
        let mut used = mac("edited", "edited", "old", 100);
        used.usage_count = 7;
        let disk = lib(vec![
            mac("arrived", "from elsewhere", "x", 100),
            used,
            mac("removed", "removed", "y", 100),
        ]);

        let after = apply(
            &disk,
            Changes {
                macros: vec![mac("edited", "edited", "new", 0), mac("made", "made here", "z", 0)],
                removed: vec!["removed".into()],
                settings: None,
            },
            NOW,
        );

        let ids: Vec<&str> = after.macros.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["made", "arrived", "edited"], "a new macro goes on top");
        let of = |id: &str| after.macros.iter().find(|m| m.id == id).unwrap();
        assert_eq!(of("arrived").updated_at, 100, "a macro nobody touched was dated");
        assert_eq!((of("edited").body.as_str(), of("edited").updated_at), ("new", NOW));
        assert_eq!(of("edited").usage_count, 7, "the count came from the window");
        assert_eq!(of("made").updated_at, NOW);
        assert_eq!(after.deleted.keys().collect::<Vec<_>>(), ["removed"]);
    }

    #[test]
    fn a_save_cannot_write_a_folder_macro_or_move_the_folders() {
        let mut disk = lib(vec![]);
        disk.settings.folders = vec!["/as/set".into()];
        let mut derived = mac("folder:/as/set/a.png", "a", "", 0);
        derived.folder = Some("/as/set".into());
        let mut stale = disk.settings.clone();
        stale.folders = vec!["/from/before".into()];
        stale.paste_mode = "copy".into();

        let after = apply(
            &disk,
            Changes {
                macros: vec![derived],
                removed: vec![],
                settings: Some(stale),
            },
            NOW,
        );

        assert!(after.macros.is_empty());
        assert_eq!(after.settings.paste_mode, "copy");
        assert_eq!(after.settings.folders, ["/as/set"]);
    }

    #[test]
    fn deleting_something_already_gone_leaves_no_trace() {
        // Made and deleted between two saves, or deleted on another machine
        // first: either way there is nothing here to remember deleting.
        let disk = lib(vec![mac("kept", "kept", "x", 100)]);

        let after = apply(
            &disk,
            Changes {
                removed: vec!["never-saved".into()],
                ..Changes::default()
            },
            NOW,
        );

        assert!(after.deleted.is_empty());
        assert_eq!(revision(&after), revision(&disk));
    }

    #[test]
    fn the_revision_is_what_the_library_holds_and_nothing_else() {
        let a = lib(vec![mac("1", "one", "x", 100), mac("2", "two", "y", 200)]);

        // Order, use counts and settings are each machine's own.
        let mut same = lib(vec![mac("2", "two", "y", 200), mac("1", "one", "x", 100)]);
        same.macros[0].usage_count = 12;
        same.settings.paste_mode = "copy".into();
        assert_eq!(revision(&a), revision(&same));

        let mut edited = a.clone();
        edited.macros[0].body = "changed".into();
        assert_ne!(revision(&a), revision(&edited));

        let mut redated = a.clone();
        redated.macros[0].updated_at = 101;
        assert_ne!(revision(&a), revision(&redated));

        let mut deleted = a.clone();
        deleted.deleted.insert("3".into(), 300);
        assert_ne!(revision(&a), revision(&deleted));
    }
}
