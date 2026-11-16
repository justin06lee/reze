mod assets;
mod clip;
mod expand;
mod focus;
mod folders;
mod grab;
mod history;
mod login;
mod merge;
mod paste;
mod store;
mod sync;
mod tailnet;
mod typed;
#[cfg(target_os = "linux")]
mod x11;

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::sync::mpsc::channel;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use store::{AppState, Library};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

const PALETTE: &str = "palette";
const EDITOR: &str = "editor";

/// Fraction of the screen height the palette sits below the top edge.
/// Slightly above centre reads as "overlay", not "dialog".
const PALETTE_TOP_RATIO: f64 = 0.22;

/// Must match `--palette-radius` in the stylesheet, or the blur layer and the
/// HTML surface will disagree at the corners.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const PALETTE_CORNER_RADIUS: f64 = 14.0;

// ---------------------------------------------------------------- commands

/// The library as the windows see it: the macros you wrote, followed by the
/// ones the [watched folders](folders) contain.
///
/// Authored first so that where a trigger name is claimed twice, the one you
/// wrote deliberately wins over a file that happens to share its name —
/// expand-in-place takes the first match.
fn merged() -> Result<Library, String> {
    let mut library = store::load()?;
    let derived = folders::scan(&library.settings.folders, &library.folder_usage);
    library.macros.extend(derived);
    Ok(library)
}

#[tauri::command]
fn get_library() -> Result<Library, String> {
    merged()
}

/// Tell both windows the library is different now.
///
/// The merged view, not what was written — the windows must not lose their
/// folder macros just because something saved a change to another one.
pub(crate) fn library_changed(app: &AppHandle) {
    if let Ok(library) = merged() {
        let _ = app.emit("library-changed", &library);
    }
}

/// Save what the editor did — see [`sync::Changes`] for why it is that, and
/// not the library as the editor has it.
#[tauri::command]
fn save_library(app: AppHandle, changes: sync::Changes) -> Result<(), String> {
    let library = {
        let _writing = store::lock();
        // Read rather than assumed, and a file that will not read is not
        // written over: it is the source of truth, and somebody is probably
        // halfway through editing it by hand.
        let before = store::load()?;
        let library = sync::apply(&before, changes, sync::now());
        // Switching sync on lets other machines change this library with
        // nobody watching, starting with a merge of everything they have. So
        // the library as it is goes aside first, as it does for an import.
        if library.settings.syncs() && !before.settings.syncs() {
            store::backup()?;
        }
        store::save(&library)?;
        library
    };
    // Saving is the only moment the set of referenced files can shrink, so it is
    // the only moment worth looking for orphans.
    assets::sweep(&library);

    history::configure(&library.settings);
    tailnet::configure(&library.settings);
    tailnet::sync_soon();

    library_changed(&app);
    Ok(())
}

/// Increment the use counter so the palette can rank by what you actually reach for.
#[tauri::command]
fn bump_usage(id: String) -> Result<(), String> {
    let _writing = store::lock();
    let mut lib = store::load()?;
    if id.starts_with("folder:") {
        // A folder macro is not in this file and cannot carry its own count —
        // see `Library::folder_usage`.
        *lib.folder_usage.entry(id).or_insert(0) += 1;
    } else if let Some(m) = lib.macros.iter_mut().find(|m| m.id == id) {
        m.usage_count += 1;
    } else {
        return Ok(()); // gone since the palette listed it; nothing to record
    }
    store::save(&lib)?;
    Ok(())
}

#[tauri::command]
fn read_clipboard() -> Result<String, String> {
    paste::read_clipboard()
}

#[tauri::command]
fn accessibility_status() -> bool {
    paste::has_accessibility()
}

#[tauri::command]
fn request_accessibility() -> bool {
    paste::request_accessibility()
}

/// Whether Reze starts itself at login. Owned by the OS, not the library file
/// — see [`login`] — so it is read back rather than remembered.
#[tauri::command]
fn login_item_status() -> login::LoginItem {
    login::status()
}

#[tauri::command]
fn set_login_item(enabled: bool) -> Result<(), String> {
    login::set(enabled)
}

#[tauri::command]
fn open_login_items_settings() {
    login::open_settings();
}

#[tauri::command]
fn open_accessibility_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}

// ------------------------------------------------------------ attachments

/// Where attachment files live, so the frontend can build `asset://` URLs for
/// its previews. A directory rather than a URL per file: the stored name is
/// already all the library carries, and joining it here would mean a round trip
/// per thumbnail.
#[tauri::command]
fn assets_dir() -> String {
    assets::assets_dir().to_string_lossy().to_string()
}

/// Copy files at `paths` into the asset library. Used by the file picker and by
/// files dropped onto the editor window.
///
/// Partial success is the useful behaviour: dropping ten files of which one is a
/// folder should attach the nine, not refuse all of them. Anything skipped comes
/// back in `errors` so the editor can say what happened rather than silently
/// attaching fewer things than were dropped.
#[tauri::command]
fn import_files(paths: Vec<String>) -> Imported {
    let mut out = Imported::default();
    for path in paths {
        match assets::import_path(std::path::Path::new(&path)) {
            Ok(a) => out.attachments.push(a),
            Err(e) => out.errors.push(e),
        }
    }
    out
}

/// Turn whatever media is on the clipboard into attachments.
///
/// Read natively rather than from the webview's `paste` event: a file copied in
/// Finder reaches the real pasteboard as a URL and the webview as nothing at
/// all, and a screenshot arrives as data the webview may or may not hand over.
#[tauri::command]
fn import_clipboard() -> Imported {
    let mut out = Imported::default();
    for item in clip::read_media() {
        let result = match item {
            clip::Incoming::File(path) => assets::import_path(&path),
            clip::Incoming::Data { mime, bytes } => {
                let ext = assets::ext_for_mime(&mime);
                // Nothing on the pasteboard carries a filename, so give it one
                // that says where it came from.
                assets::import_bytes(&format!("Clipboard.{ext}"), ext, &bytes)
            }
        };
        match result {
            Ok(a) => out.attachments.push(a),
            Err(e) => out.errors.push(e),
        }
    }
    out
}

/// The outcome of an import: what made it in, and why anything else did not.
#[derive(Default, serde::Serialize)]
struct Imported {
    attachments: Vec<assets::Attachment>,
    errors: Vec<String>,
}

/// Resolve attachments to real paths, dropping any that no longer exist.
///
/// A file deleted out from under the library must not fail the whole paste —
/// the text half is still worth delivering — and a watched folder on an
/// unplugged drive is exactly that case.
fn attachment_paths(attachments: &[assets::Attachment]) -> Vec<PathBuf> {
    attachments
        .iter()
        .filter_map(|a| assets::resolve(a).ok())
        .filter(|p| p.exists())
        .collect()
}

// ---------------------------------------------------------------- folders

/// Replace the watched folder list, then rescan.
#[tauri::command]
fn set_folders(app: AppHandle, folders: Vec<String>) -> Result<(), String> {
    let library = {
        let _writing = store::lock();
        let mut library = store::load()?;
        library.settings.folders = folders;
        store::save(&library)?;
        library
    };

    watch_folders(&app, &library.settings.folders);
    library_changed(&app);
    Ok(())
}

/// Show a file or folder in Finder. Used for both the library and the watched
/// folders, which is why it takes a path rather than knowing one.
#[tauri::command]
fn reveal_path(path: String) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(&path)
        .spawn();

    #[cfg(target_os = "linux")]
    reveal_on_linux(std::path::Path::new(&path));

    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn();
}

// ------------------------------------------------------------------ grab

/// Which download tools are installed, so Settings can say what is missing.
#[tauri::command]
fn grab_tools() -> grab::Tools {
    grab::tools()
}

/// One grab at a time.
///
/// Not just politeness: `gallery-dl` is identified by which files appeared in
/// the folder while it ran, and two overlapping downloads would each claim the
/// other's. It doubles as the "do not hide the palette" flag — a window that
/// vanished mid-download would take the naming step with it.
#[derive(Default)]
pub struct Grabbing(AtomicBool);

/// A finished download, ready to be previewed and named.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Grabbed {
    /// Where it landed. Handed back so the rename knows what to move.
    path: String,
    /// What to put in the naming field: the filename without its extension.
    suggested_name: String,
    /// So the palette can preview it with the same component as everything else.
    attachment: assets::Attachment,
}

/// Download whatever `url` points at into the first watched folder.
///
/// Awaited by the palette rather than fired and forgotten, because the whole
/// point of doing this from the palette is that there is somewhere to show
/// progress and somewhere to ask what to call the result.
#[tauri::command]
async fn grab_url(app: AppHandle, url: String) -> Result<Grabbed, String> {
    if app.state::<Grabbing>().0.swap(true, Ordering::SeqCst) {
        return Err("Already downloading something.".into());
    }
    // A guard rather than clearing at each exit: `?` below must not leave the
    // flag set, or grabbing would be dead for the rest of the session.
    struct Release(AppHandle);
    impl Drop for Release {
        fn drop(&mut self) {
            self.0.state::<Grabbing>().0.store(false, Ordering::SeqCst);
        }
    }
    let _release = Release(app.clone());

    let library = store::load()?;
    let dir = folders::download_dir(&library.settings.folders)?;
    // `download_dir` creates the folder if it was missing, and a folder that did
    // not exist at launch is neither watched nor allowed to be previewed. This is
    // the one moment that set can change by our own doing, so re-point both.
    watch_folders(&app, &library.settings.folders);

    let result = tauri::async_runtime::spawn_blocking(move || grab::grab(&url, &dir))
        .await
        .map_err(|e| e.to_string())??;

    let bytes = std::fs::metadata(&result.path).map(|m| m.len()).unwrap_or(0);
    let grabbed = Grabbed {
        suggested_name: result
            .path
            .file_stem()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default(),
        attachment: assets::referencing(&result.path, bytes),
        path: result.path.to_string_lossy().to_string(),
    };

    // It is already a macro — the folder watcher will say so too, but not for
    // another few hundred milliseconds, and the palette is showing it now.
    if let Ok(library) = merged() {
        let _ = app.emit("library-changed", &library);
    }
    Ok(grabbed)
}

/// Rename a just-grabbed file, which is what renames the macro made from it.
#[tauri::command]
fn rename_grab(app: AppHandle, path: String, name: String) -> Result<String, String> {
    let from = PathBuf::from(&path);
    let parent = from.parent().ok_or("that file has nowhere to live")?;

    // The path came from us a moment ago, but it arrives back over IPC, so it
    // is checked rather than trusted: renaming is only ever allowed inside a
    // folder the user asked us to watch.
    let watched = folders::existing(&store::load()?.settings.folders);
    if !watched.iter().any(|dir| parent.starts_with(dir)) {
        return Err("that file is not in a watched folder".into());
    }

    let extension = from
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let to = grab::unique_named(parent, &name, &extension);
    if to != from {
        std::fs::rename(&from, &to).map_err(|e| format!("renaming: {e}"))?;
    }

    if let Ok(library) = merged() {
        let _ = app.emit("library-changed", &library);
    }
    Ok(to.to_string_lossy().to_string())
}

// ------------------------------------------------------- clipboard history

/// Everything remembered, newest first — previews only; see [`history::View`].
#[tauri::command]
fn clipboard_history() -> Vec<history::View> {
    history::views()
}

/// Where remembered images and app icons live, for the same reason
/// [`assets_dir`] exists: the entries carry bare names, and the previews are
/// drawn straight off disk.
#[tauri::command]
fn clipboard_dir() -> String {
    history::clip_dir().to_string_lossy().to_string()
}

#[tauri::command]
fn forget_clip(app: AppHandle, id: String) {
    history::remove(&id);
    let _ = app.emit("clipboard-changed", history::views());
}

#[tauri::command]
fn clear_clips(app: AppHandle) {
    history::clear();
    let _ = app.emit("clipboard-changed", history::views());
}

// ------------------------------------------------------------ other devices

/// Whether Tailscale is up, and which of the user's other machines run Reze.
///
/// Async because it waits on every one of them at once — see
/// [`tailnet::status`] — and a sleeping laptop takes seconds to not answer.
#[tauri::command]
async fn tailnet_status() -> tailnet::Tailnet {
    tauri::async_runtime::spawn_blocking(tailnet::status)
        .await
        .unwrap_or(tailnet::Tailnet::Stopped)
}

/// What importing from the machine at `address` would do, without doing it.
#[tauri::command]
async fn import_preview(address: String) -> Result<merge::Preview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let theirs = tailnet::library_of(&address)?;
        let mine = store::load()?;
        Ok(merge::preview(
            &mine,
            &theirs.library,
            theirs.hello.os == std::env::consts::OS,
            |file| assets::path_for(file).is_ok_and(|path| path.exists()),
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What an import did, for the line Settings shows once it is over.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct ImportOutcome {
    /// The machine it came from, by its tailnet name.
    from: String,
    added: usize,
    updated: usize,
    removed: usize,
    /// Attachment files that had to be fetched.
    attachments: usize,
    /// Attachments that machine's library names but could not hand over.
    missing: Vec<String>,
    /// Anything asked for that was deliberately not done.
    warnings: Vec<String>,
}

/// Bring the library of the machine at `address` into this one.
///
/// `replace` makes this library a copy of that one; without it, what only this
/// machine has is kept. See [`merge`] for what "the same macro" means.
#[tauri::command]
async fn import_library(
    app: AppHandle,
    address: String,
    replace: bool,
    settings: bool,
) -> Result<ImportOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || run_import(&app, &address, replace, settings))
        .await
        .map_err(|e| e.to_string())?
}

fn run_import(
    app: &AppHandle,
    address: &str,
    replace: bool,
    settings: bool,
) -> Result<ImportOutcome, String> {
    // Fetched again rather than carried over from the preview: what gets
    // merged should be what is there now, not what was there when asked.
    let theirs = tailnet::library_of(address)?;
    let choices = merge::Choices {
        replace,
        settings,
        hotkeys: settings && theirs.hello.os == std::env::consts::OS,
    };

    // Files first, library last — see `collect` for why the order matters.
    // They are fetched for the merge as it would come out now; the merge that
    // is kept is made again below, once nothing else can write, because a
    // fetch takes long enough for the editor or a sync to save in the middle.
    let planned = merge::merge(&store::load()?, &theirs.library, choices);
    let collected = theirs.collect(&planned.incoming, &assets::assets_dir())?;

    let writing = store::lock();
    let mine = store::load()?;
    let mut result = merge::merge(&mine, &theirs.library, choices);
    let unfetched = |a: &assets::Attachment| {
        !collected.gone.contains(&a.file) && !assets::path_for(&a.file).is_ok_and(|p| p.exists())
    };
    if result.incoming.iter().any(unfetched) {
        return Err(
            "The library here changed while the import was fetching files. Nothing was \
             changed — import again."
                .into(),
        );
    }
    collected.strip(&mut result.library);

    // An import is a change like any other, and has to be dated like one for
    // the other machines to hear of it.
    sync::stamp(&mine, &mut result.library, sync::now());

    // The one write in this app that replaces macros nobody had open in front
    // of them, so the library as it is goes aside first — see `undo_import`.
    store::backup()?;

    // Hotkeys that came along are registered before they are saved, the same
    // rule as rebinding one by hand: a combination another app already holds
    // must not end up written down as this machine's way in.
    let mut warnings = Vec::new();
    let keys = |s: &store::Settings| {
        [s.hotkey.clone(), s.expand_hotkey.clone(), s.clipboard_hotkey.clone()]
    };
    let (before, after) = (keys(&mine.settings), keys(&result.library.settings));
    let rebind = |[palette, expand, clipboard]: &[String; 3]| {
        register_hotkeys(app, palette, expand, clipboard)
    };
    if before != after {
        if let Err(e) = rebind(&after) {
            let _ = rebind(&before);
            let s = &mut result.library.settings;
            [s.hotkey, s.expand_hotkey, s.clipboard_hotkey] = before.clone();
            warnings.push(format!("The hotkeys were left as they were ({e})."));
        }
    }

    if let Err(e) = store::save(&result.library) {
        let _ = rebind(&before);
        return Err(e);
    }
    drop(writing);
    history::configure(&result.library.settings);
    tailnet::sync_soon();

    let outcome = ImportOutcome {
        from: theirs.name,
        added: result.added,
        updated: result.updated,
        removed: result.removed,
        attachments: collected.fetched,
        missing: collected.missing,
        warnings,
    };
    library_changed(app);
    Ok(outcome)
}

/// Put the library back the way it was before the last import.
///
/// Exactly as it was, dates and all — which is why it is not offered while
/// sync is on. Put back like this, the macros an import brought are merely
/// absent, and the other machines hand them straight back; and the way to
/// make their absence stick is to record them as deleted, which would delete
/// them from the machine they were imported from.
#[tauri::command]
fn undo_import(app: AppHandle) -> Result<(), String> {
    let (library, folders_before) = {
        let _writing = store::lock();
        let folders_before = store::load().map(|l| l.settings.folders).unwrap_or_default();
        (store::restore_backup()?, folders_before)
    };

    // Everything a save would have re-pointed, since the settings may have
    // gone back with the macros.
    let s = &library.settings;
    if let Err(e) = register_hotkeys(&app, &s.hotkey, &s.expand_hotkey, &s.clipboard_hotkey) {
        eprintln!("reze: {e}");
    }
    if folders_before != s.folders {
        watch_folders(&app, &s.folders);
    }
    history::configure(s);
    tailnet::configure(s);
    tailnet::sync_soon();

    library_changed(&app);
    Ok(())
}

/// Hide the palette, then deliver `text` and `attachments` to whatever app
/// regained focus.
#[tauri::command]
async fn deliver(
    app: AppHandle,
    text: String,
    copy_only: bool,
    attachments: Vec<assets::Attachment>,
) -> Result<(), String> {
    let files = attachment_paths(&attachments);
    deliver_payload(app, paste::Payload::Bundle { text, files }, copy_only).await
}

/// Put a remembered clipboard entry back — the same delivery, from the other
/// source. See [`history`].
#[tauri::command]
async fn deliver_clip(app: AppHandle, id: String, copy_only: bool) -> Result<(), String> {
    // Resolved before the window goes: an entry whose file has been deleted
    // since it was copied should say so while there is still something on
    // screen to say it on.
    let payload = history::payload_for(&id)?;
    deliver_payload(app, payload, copy_only).await
}

/// Hide the palette, hand focus back, and paste.
async fn deliver_payload(
    app: AppHandle,
    payload: paste::Payload,
    copy_only: bool,
) -> Result<(), String> {
    // Checked before hiding, not inside the worker: an error raised after the
    // window is gone is an error the user never sees. Missing Accessibility is
    // the single most likely failure — and it silently recurs after every
    // update, since a rebuilt binary is a new app as far as macOS is concerned.
    if !copy_only && !paste::has_accessibility() {
        return Err("accessibility-denied".into());
    }

    if let Some(win) = app.get_webview_window(PALETTE) {
        let _ = win.hide();
    }

    // Hiding is not enough on its own. macOS does usually hand focus back, but
    // not always before the keystroke lands, and a Cmd+V sent while nothing is
    // frontmost goes nowhere at all — no error, no paste. So put the app that
    // had focus back deliberately, then give it a moment to actually take it.
    if !copy_only {
        let previous = app.state::<AppState>().previous_app.lock().unwrap().take();
        if let Some(target) = previous {
            let _ = on_main(&app, move || focus::activate(target));
            std::thread::sleep(Duration::from_millis(60));
        }
    }

    let restore = store::load()
        .map(|l| l.settings.restore_clipboard)
        .unwrap_or(true);

    let worker = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        // Everything below writes to the pasteboard. Without this the history
        // would fill up with Reze pasting on the user's behalf.
        let _quiet = history::suspend();
        if copy_only {
            paste::copy_payload(&payload)
        } else {
            paste::paste_payload(&payload, restore, |f| {
                on_main(&worker, f).and_then(|inner| inner)
            })
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    // Anything that fails after the hide would otherwise vanish with the
    // window, so bring it back to carry the message.
    if result.is_err() {
        if let Some(win) = app.get_webview_window(PALETTE) {
            let _ = win.show();
            let _ = win.set_focus();
        }
    }
    result
}

/// Run `f` on the main thread and wait for its result.
///
/// Every synthesized keystroke goes through here. The timeout is a backstop
/// only: if the event loop were ever wedged, a missing paste is a far better
/// outcome than a worker thread parked forever.
fn on_main<T, F>(app: &AppHandle, f: F) -> Result<T, String>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = channel();
    app.run_on_main_thread(move || {
        let _ = tx.send(f());
    })
    .map_err(|e| format!("could not reach the main thread: {e}"))?;

    rx.recv_timeout(Duration::from_secs(5))
        .map_err(|_| "timed out waiting for the main thread".to_string())
}

#[tauri::command]
fn hide_palette(app: AppHandle) {
    if let Some(win) = app.get_webview_window(PALETTE) {
        let _ = win.hide();
    }
}

/// Bring the palette up without resetting it — used when an in-place expansion
/// turns out to need values filled in, so the query and stage are preserved.
#[tauri::command]
fn show_palette(app: AppHandle) {
    reveal_palette(&app);
}

#[tauri::command]
fn open_editor(app: AppHandle) {
    show_editor(&app);
}

/// Tell the backend the window switched between the macro list and the
/// clipboard on its own, so the next hotkey press still knows whether it means
/// "open the other one" or "close this".
#[tauri::command]
fn set_palette_mode(app: AppHandle, clipboard: bool) {
    app.state::<PaletteMode>().0.store(clipboard, Ordering::SeqCst);
}

/// Which mode was last asked for.
///
/// Read once when the palette's JavaScript starts. Tauri events are not
/// buffered, so a hotkey pressed in the second between the process starting and
/// the web view attaching its listeners would otherwise open the window on the
/// wrong thing — most likely right after logging in, which is exactly when the
/// app has just started.
#[tauri::command]
fn palette_mode(app: AppHandle) -> bool {
    app.state::<PaletteMode>().0.load(Ordering::SeqCst)
}

/// Quit for real. A menu-bar app has no application menu, so the usual ⌘Q has
/// nothing to hang off — the windows provide it themselves.
#[tauri::command]
fn quit(app: AppHandle) {
    app.exit(0);
}

#[tauri::command]
fn resize_palette(app: AppHandle, height: f64) {
    if let Some(win) = app.get_webview_window(PALETTE) {
        if let Ok(size) = win.inner_size() {
            let scale = win.scale_factor().unwrap_or(1.0);
            let width = size.width as f64 / scale;
            let _ = win.set_size(tauri::LogicalSize::new(width, height.max(80.0)));
        }
    }
}

/// The two registered shortcuts, kept so the handler can tell them apart
/// without re-reading the library from disk on every keypress.
#[derive(Default)]
struct Hotkeys {
    palette: Mutex<Option<Shortcut>>,
    expand: Mutex<Option<Shortcut>>,
    clipboard: Mutex<Option<Shortcut>>,
}

#[tauri::command]
fn set_hotkeys(
    app: AppHandle,
    palette: String,
    expand: String,
    clipboard: String,
) -> Result<(), String> {
    register_hotkeys(&app, &palette, &expand, &clipboard)
}

/// Registers both shortcuts, replacing whatever was registered before.
///
/// Parsed up front so an invalid accelerator is rejected before we tear down
/// the working ones — otherwise a typo in Settings would leave the user with
/// no way to open the app at all.
fn register_hotkeys(
    app: &AppHandle,
    palette: &str,
    expand: &str,
    clipboard: &str,
) -> Result<(), String> {
    let parse = |what: &str, accelerator: &str| {
        Shortcut::from_str(accelerator).map_err(|e| format!("invalid {what} hotkey: {e}"))
    };
    let palette_shortcut = parse("palette", palette)?;
    let expand_shortcut = parse("expand", expand)?;
    let clipboard_shortcut = parse("clipboard", clipboard)?;

    app.global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;

    // Every one is attempted even after a failure. A combination another app
    // already owns is an ordinary thing to hit — ⌘⇧V especially — and having
    // that take the *other* two hotkeys down with it would be absurd.
    let mut refused = Vec::new();
    for (accelerator, shortcut) in [
        (palette, palette_shortcut),
        (expand, expand_shortcut),
        (clipboard, clipboard_shortcut),
    ] {
        if let Err(e) = app.global_shortcut().register(shortcut) {
            refused.push(format!("{accelerator} ({e})"));
        }
    }

    let state = app.state::<Hotkeys>();
    *state.palette.lock().unwrap() = Some(palette_shortcut);
    *state.expand.lock().unwrap() = Some(expand_shortcut);
    *state.clipboard.lock().unwrap() = Some(clipboard_shortcut);

    match refused.is_empty() {
        true => Ok(()),
        false => Err(format!("already taken: {}", refused.join(", "))),
    }
}

#[tauri::command]
fn library_path() -> String {
    store::library_path().to_string_lossy().to_string()
}

#[tauri::command]
fn reveal_library() {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("-R")
        .arg(store::library_path())
        .spawn();

    #[cfg(target_os = "linux")]
    reveal_on_linux(&store::library_path());

    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer")
        .arg("/select,")
        .arg(store::library_path())
        .spawn();
}

/// Show `path` selected in the file manager.
///
/// There is no `open -R`, but there is a D-Bus interface every mainstream file
/// manager implements for exactly this (`org.freedesktop.FileManager1`). Where
/// nothing answers it, open the folder instead: `xdg-open` on the file itself
/// would launch whatever app opens it, which is not what "reveal" means.
#[cfg(target_os = "linux")]
fn reveal_on_linux(path: &std::path::Path) {
    use std::process::{Command, Stdio};
    let path = path.to_path_buf();
    // Off the calling thread: `gdbus` waits for the file manager to start.
    std::thread::spawn(move || {
        let Ok(uri) = gtk::glib::filename_to_uri(&path, None) else {
            return;
        };
        let shown = Command::new("gdbus")
            .args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.FileManager1",
                "--object-path",
                "/org/freedesktop/FileManager1",
                "--method",
                "org.freedesktop.FileManager1.ShowItems",
            ])
            .arg(format!("['{}']", uri.replace('\'', "%27")))
            .arg("")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if !shown {
            let folder = match path.is_dir() {
                true => path.clone(),
                false => path.parent().map(|p| p.to_path_buf()).unwrap_or(path.clone()),
            };
            let _ = Command::new("xdg-open").arg(folder).spawn();
        }
    });
}

// ---------------------------------------------------------------- windows

fn show_editor(app: &AppHandle) {
    if let Some(win) = app.get_webview_window(EDITOR) {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
    }
}

/// Centre the palette horizontally on whichever display the cursor is on, so it
/// appears where the user is looking rather than always on the primary display.
fn position_palette(app: &AppHandle, win: &tauri::WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());

    let Some(monitor) = monitor else { return };
    let scale = monitor.scale_factor();
    let m_pos = monitor.position().to_logical::<f64>(scale);
    let m_size = monitor.size().to_logical::<f64>(scale);
    let Ok(win_size) = win.outer_size() else { return };
    let win_size = win_size.to_logical::<f64>(win.scale_factor().unwrap_or(scale));

    let x = m_pos.x + (m_size.width - win_size.width) / 2.0;
    let y = m_pos.y + m_size.height * PALETTE_TOP_RATIO;
    let _ = win.set_position(tauri::LogicalPosition::new(x, y));
}

/// Note who currently has focus so [`deliver`] can hand it back.
///
/// Skips our own windows: opening the palette twice in a row must not record
/// Reze as the app to paste into.
fn remember_frontmost(app: &AppHandle) {
    if let Some(target) = focus::frontmost_elsewhere() {
        *app.state::<AppState>().previous_app.lock().unwrap() = Some(target);
    }
}

fn reveal_palette(app: &AppHandle) {
    let Some(win) = app.get_webview_window(PALETTE) else {
        return;
    };
    remember_frontmost(app);
    position_palette(app, &win);
    let _ = win.show();
    let _ = win.set_focus();
    #[cfg(target_os = "linux")]
    focus_on_x11(&win);
}

/// Give the palette the keyboard, which `set_focus` alone does not on X11.
///
/// Window managers with focus-stealing prevention — GNOME's included — map a
/// window a background app shows but leave the keyboard where it was, since an
/// app nobody is using asking for focus is precisely what that feature exists
/// to refuse. Here the user did ask, by pressing the hotkey, so ask the way a
/// pager would: see [`x11::activate`].
#[cfg(target_os = "linux")]
fn focus_on_x11(win: &tauri::WebviewWindow) {
    let handle = win.clone();
    let _ = win.run_on_main_thread(move || {
        use gtk::glib::Cast;
        use gtk::prelude::WidgetExt;
        let Some(xid) = handle
            .gtk_window()
            .ok()
            .and_then(|w| w.window())
            .and_then(|w| w.downcast::<gdkx11::X11Window>().ok())
            .map(|w| w.xid() as u32)
        else {
            return;
        };
        // Off the main thread: the window has to finish mapping first, and
        // that happens on this thread once the closure returns.
        std::thread::spawn(move || {
            x11::activate(xid);
        });
    });
}

/// Which of the two things the palette is currently showing.
///
/// Kept here rather than asked of the web view because both hotkeys have to
/// decide *before* the window is touched whether this press means "open" or
/// "close" — and a round trip to JavaScript to find out would show the window
/// first and hide it a frame later.
#[derive(Default)]
struct PaletteMode(AtomicBool);

fn toggle_palette(app: &AppHandle) {
    let Some(win) = app.get_webview_window(PALETTE) else {
        return;
    };
    let mode = app.state::<PaletteMode>();
    if win.is_visible().unwrap_or(false) && !mode.0.load(Ordering::SeqCst) {
        let _ = win.hide();
        return;
    }
    mode.0.store(false, Ordering::SeqCst);
    // Tell the UI to reset before it becomes visible, so the previous query
    // never flashes on screen.
    let _ = app.emit_to(PALETTE, "palette-opened", ());
    reveal_palette(app);
}

/// The same window, opened onto the clipboard history instead.
///
/// A mode of the palette rather than a window of its own: it wants the same
/// search field, the same list, the same preview pane, and above all the same
/// careful business of handing focus back to the app you were in before
/// pressing ⌘V for you. A second window would be all of that again.
fn toggle_clipboard(app: &AppHandle) {
    let Some(win) = app.get_webview_window(PALETTE) else {
        return;
    };
    let mode = app.state::<PaletteMode>();
    if win.is_visible().unwrap_or(false) && mode.0.load(Ordering::SeqCst) {
        let _ = win.hide();
        return;
    }
    mode.0.store(true, Ordering::SeqCst);
    let _ = app.emit_to(PALETTE, "palette-clipboard", ());
    reveal_palette(app);
}

// ---------------------------------------------------------- expand in place

/// Expand the trigger the user just typed, without showing anything.
///
/// Runs off the hotkey thread so the event loop stays responsive while we do
/// several round-trips of synthesized keystrokes.
fn expand_in_place(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        if let Err(e) = run_expand(&app) {
            eprintln!("reze: expand-in-place: {e}");
            let _ = app.emit_to(PALETTE, "expand-failed", e);
        }
    });
}

fn run_expand(app: &AppHandle) -> Result<(), String> {
    if !paste::has_accessibility() {
        return Err("accessibility-denied".into());
    }

    // The app being typed into is the one to paste back into — not whichever
    // one the palette was last opened over and then closed without pasting.
    *app.state::<AppState>().previous_app.lock().unwrap() = focus::frontmost();

    let library = store::load()?;
    let names: Vec<String> = library.macros.iter().map(|m| m.name.clone()).collect();
    if names.is_empty() {
        return Ok(());
    }

    // The keystroke tap cannot be created without Accessibility, so permission
    // granted after launch leaves it uninstalled. Retry here instead of making
    // the user restart — this attempt still falls back to selection, but the
    // next one will not.
    if library.settings.track_typing && !typed::is_running() {
        let _ = on_main(app, typed::start);
    }

    // Preferred strategy: replace what we watched being typed. This is the only
    // one that works in a terminal or any TUI, where the line being edited
    // cannot be selected or copied at all.
    if library.settings.track_typing && typed::is_running() {
        let buffer = typed::snapshot();
        if let Some((index, start, end)) = expand::match_trigger(&buffer, &names) {
            // Everything from the trigger to the caret goes; whatever the user
            // typed after it comes back on the other side of the expansion.
            let doomed = buffer[start..].chars().count();
            // Only now, with the buffer safely read: on X11 a keystroke sent
            // while the hotkey is still down combines with its modifiers, or
            // goes to the hotkey's grab instead of the app.
            focus::wait_for_hotkey_release();
            on_main(app, move || paste::send(paste::Chord::Backspace, doomed))
                .and_then(|inner| inner)?;
            typed::drop_last(doomed);

            return app
                .emit_to(
                    PALETTE,
                    "expand-selected",
                    Expansion {
                        id: library.macros[index].id.clone(),
                        head: String::new(),
                        tail: buffer[end..].to_string(),
                    },
                )
                .map_err(|e| e.to_string());
        }
    }

    // Fallback: select the words in front of the caret and read them back.
    // Needed when the buffer is cold — text typed before Reze started, or
    // pasted in rather than typed.
    //
    // Never in a Linux terminal, where the Ctrl+C this sends is SIGINT to
    // whatever is running rather than a copy.
    if focus::frontmost_is_terminal() {
        return Err("a terminal line can only be expanded from what was seen typed".into());
    }
    //
    // From here to the end this borrows the clipboard and synthesizes a ⌘C, so
    // the history is held quiet: the words in front of your caret are not
    // something you copied.
    let _quiet = history::suspend();
    let span = expand::words_to_select(&names);

    // Borrow the clipboard to read the text back, then hand it straight back to
    // the user before anything else can go wrong.
    let saved = paste::read_clipboard().unwrap_or_default();
    let before = focus::pasteboard_change_count();

    // Where there is no change counter, leave a value on the clipboard that
    // nothing else could produce; finding it still there proves the copy did
    // not happen.
    if before.is_none() {
        let _ = paste::write_clipboard(paste::SENTINEL);
    }

    focus::wait_for_hotkey_release();

    on_main(app, move || -> Result<(), String> {
        paste::send(paste::Chord::ExtendWordLeft, span)?;
        paste::send(paste::Chord::Copy, 1)
    })
    .and_then(|inner| inner)?;

    // Wait for the copy rather than assuming it worked: with the caret at the
    // start of a line there is nothing to select, and reading the clipboard
    // blind would match against something copied hours ago.
    let mut selection = None;
    for _ in 0..25 {
        std::thread::sleep(Duration::from_millis(20));
        match before {
            Some(count) => {
                if focus::pasteboard_change_count() != Some(count) {
                    selection = paste::read_clipboard().ok();
                    break;
                }
            }
            None => {
                let current = paste::read_clipboard().unwrap_or_default();
                if current != paste::SENTINEL {
                    selection = Some(current);
                    break;
                }
            }
        }
    }

    let restore = |app: &AppHandle| {
        let _ = on_main(app, || paste::send(paste::Chord::CollapseRight, 1));
        let _ = paste::write_clipboard(&saved);
    };

    let Some(selection) = selection.filter(|s| !s.trim().is_empty()) else {
        restore(app);
        return Err("nothing selectable before the caret".into());
    };
    let _ = paste::write_clipboard(&saved);

    let Some((index, start, end)) = expand::match_trigger(&selection, &names) else {
        restore(app);
        return Err(format!("no macro named {:?}", selection.trim()));
    };

    // The selection is usually wider than the trigger, so the replacement puts
    // the surrounding characters back verbatim around the expansion. Retyping
    // them costs nothing and avoids depending on word-motion keystrokes to land
    // exactly on the trigger boundary, which they do not.
    let payload = Expansion {
        id: library.macros[index].id.clone(),
        head: selection[..start].to_string(),
        tail: selection[end..].to_string(),
    };

    // Hand off to the palette's JS, which owns the template engine. It stays
    // hidden unless the macro needs values filling in.
    app.emit_to(PALETTE, "expand-selected", payload)
        .map_err(|e| e.to_string())
}

/// What the palette needs to rebuild the line: which macro, and the characters
/// on either side of the trigger that must survive the replacement.
#[derive(Clone, serde::Serialize)]
struct Expansion {
    id: String,
    head: String,
    tail: String,
}

// -------------------------------------------------------- folder watcher

/// Holds the watcher over the configured folders, so it can be replaced when
/// the list changes. Dropping the old one is what stops it.
#[derive(Default)]
struct FolderWatch(Mutex<Option<notify::RecommendedWatcher>>);

/// How long the folders must be quiet before a rescan.
///
/// Copying fifty files in produces fifty events; rescanning on each would mean
/// fifty walks of the tree and fifty repaints for one gesture.
const FOLDER_SETTLE: Duration = Duration::from_millis(400);

/// Watch `folders` and rebroadcast the library whenever their contents change.
///
/// Replaces any previous watch wholesale. The old watcher is dropped, which
/// closes its channel, which ends the thread draining it — so changing the
/// folder list leaves nothing behind.
fn watch_folders(app: &AppHandle, folders: &[String]) {
    let dirs = folders::existing(folders);
    let (tx, rx) = channel();
    let Ok(mut watcher) = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if res.is_ok() {
            let _ = tx.send(());
        }
    }) else {
        return;
    };
    for dir in &dirs {
        if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
            eprintln!("reze: not watching {dir:?}: {e}");
        }
        // The previews are drawn from these files directly, and the asset
        // protocol refuses anything outside its scope. Widening it to a folder
        // the user chose is the same trust we already extend to the asset
        // library: the web view only ever loads our own bundled code, and no
        // remote content runs in it.
        if let Err(e) = app.asset_protocol_scope().allow_directory(dir, true) {
            eprintln!("reze: no previews for {dir:?}: {e}");
        }
    }

    let handle = app.clone();
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            // Swallow the rest of the burst before doing any work.
            while rx.recv_timeout(FOLDER_SETTLE).is_ok() {}
            if let Ok(library) = merged() {
                let _ = handle.emit("library-changed", &library);
            }
        }
    });

    *app.state::<FolderWatch>().0.lock().unwrap() = Some(watcher);
}

// ---------------------------------------------------------------- watcher

/// Reload and broadcast whenever the library file changes underneath us,
/// ignoring the writes we made ourselves.
fn spawn_watcher(app: AppHandle) {
    std::thread::spawn(move || {
        let dir = store::config_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let (tx, rx) = channel();
        let Ok(mut watcher) = notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) else {
            return;
        };
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }

        for event in rx {
            let Ok(event) = event else { continue };
            if !event.paths.iter().any(|p| p == &store::library_path()) {
                continue;
            }
            // Editors often write in several steps; let it land before reading.
            std::thread::sleep(Duration::from_millis(80));

            match take_hand_edit() {
                None => {}
                Some(Ok(edited)) => {
                    // The switches these two mirror are in the file too.
                    history::configure(&edited.settings);
                    tailnet::configure(&edited.settings);
                    library_changed(&app);
                    tailnet::sync_soon();
                }
                Some(Err(e)) => {
                    let _ = app.emit("library-error", e);
                }
            }
        }
    });
}

/// Take the library file as it now is, if it is not as we left it.
///
/// `None` when it is — the change was our own save — and otherwise the
/// library that is there now, or why it is not one.
fn take_hand_edit() -> Option<Result<Library, String>> {
    // Held while looking, so that a save of ours is either not begun or
    // finished and remembered — never caught in between and taken for
    // somebody else's.
    let _writing = store::lock();
    let raw = std::fs::read_to_string(store::library_path()).ok()?;
    let ours = store::last_written();
    if ours == raw {
        return None;
    }
    store::remember(&raw);

    let mut edited: Library = match serde_json::from_str(&raw) {
        Ok(edited) => edited,
        Err(e) => return Some(Err(e.to_string())),
    };
    // An edit made in a text editor is a change like any other, and has to be
    // dated like one for the other machines to hear of it. Dated against what
    // we last wrote, which is the library as it was before the edit — unless
    // that was not a library either, and then there is nothing to go by.
    if let Ok(before) = serde_json::from_str::<Library>(&ours) {
        // Sync switched on in the file is the same step as switching it on in
        // Settings, and is taken as carefully: see `save_library`. The macros
        // are still as they were — nothing has been fetched yet.
        if edited.settings.syncs() && !before.settings.syncs() {
            if let Err(e) = store::backup() {
                eprintln!("reze: could not set the library aside before syncing: {e}");
            }
        }
        let dates = |l: &Library| {
            let macros: Vec<u64> = l.macros.iter().map(|m| m.updated_at).collect();
            (macros, l.deleted.clone())
        };
        let as_written = dates(&edited);
        sync::stamp(&before, &mut edited, sync::now());
        // Written back only if dating it changed something: an edit to a
        // setting leaves the file exactly as its author saved it.
        if dates(&edited) != as_written {
            if let Err(e) = store::save(&edited) {
                eprintln!("reze: could not date a hand edit: {e}");
            }
        }
    }
    Some(Ok(edited))
}

// ---------------------------------------------------------------- setup

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    // Fire on press only; without this the palette toggles twice per tap.
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let hotkeys = app.state::<Hotkeys>();
                    let is = |slot: &std::sync::Mutex<Option<Shortcut>>| {
                        slot.lock().unwrap().as_ref() == Some(shortcut)
                    };
                    if is(&hotkeys.expand) {
                        expand_in_place(app);
                    } else if is(&hotkeys.clipboard) {
                        toggle_clipboard(app);
                    } else if is(&hotkeys.palette) {
                        toggle_palette(app);
                    }
                })
                .build(),
        )
        .manage(AppState::new())
        .manage(Hotkeys::default())
        .manage(Grabbing::default())
        .manage(FolderWatch::default())
        .manage(PaletteMode::default())
        .invoke_handler(tauri::generate_handler![
            get_library,
            save_library,
            bump_usage,
            read_clipboard,
            accessibility_status,
            request_accessibility,
            open_accessibility_settings,
            login_item_status,
            set_login_item,
            open_login_items_settings,
            deliver,
            hide_palette,
            show_palette,
            open_editor,
            quit,
            resize_palette,
            set_hotkeys,
            library_path,
            reveal_library,
            assets_dir,
            import_files,
            import_clipboard,
            set_folders,
            reveal_path,
            grab_tools,
            grab_url,
            rename_grab,
            set_palette_mode,
            palette_mode,
            clipboard_history,
            clipboard_dir,
            forget_clip,
            clear_clips,
            deliver_clip,
            tailnet_status,
            import_preview,
            import_library,
            undo_import,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();

            // Attachment previews are drawn straight from disk over `asset://`.
            // Granted here rather than as a config glob because the directory
            // hangs off the real home directory, which only the running process
            // can resolve without guessing.
            {
                let dir = assets::assets_dir();
                let _ = std::fs::create_dir_all(&dir);
                if let Err(e) = handle.asset_protocol_scope().allow_directory(&dir, false) {
                    eprintln!("reze: attachment previews unavailable ({e})");
                }
            }

            // What is on disk is remembered as it is, so that the first edit
            // made to it by hand can be told from our own saves and dated. A
            // file that does not load is remembered as nothing: there is no
            // library there to date an edit against.
            let lib = match store::load() {
                Ok(lib) => {
                    if let Ok(raw) = std::fs::read_to_string(store::library_path()) {
                        store::remember(&raw);
                    }
                    lib
                }
                Err(e) => {
                    eprintln!("reze: {e}");
                    Library::default()
                }
            };

            if let Err(e) = register_hotkeys(
                &handle,
                &lib.settings.hotkey,
                &lib.settings.expand_hotkey,
                &lib.settings.clipboard_hotkey,
            ) {
                eprintln!("reze: {e}");
            }

            // Attaches to this thread's run loop, so it has to happen here on
            // the main thread. Failure is not fatal: expansion falls back to
            // selecting and copying the text in front of the caret.
            if lib.settings.track_typing {
                if let Err(e) = typed::start() {
                    eprintln!("reze: typing not tracked ({e}); using selection fallback");
                }
            }

            let open_item = MenuItem::with_id(app, "open", "Macro Editor…", true, None::<&str>)?;
            let palette_item =
                MenuItem::with_id(app, "palette", "Show Palette", true, None::<&str>)?;
            let clipboard_item =
                MenuItem::with_id(app, "clipboard", "Clipboard History", true, None::<&str>)?;
            let quit_item =
                MenuItem::with_id(app, "quit", "Quit Reze", true, Some("CmdOrCtrl+Q"))?;
            let sep = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(
                app,
                &[&palette_item, &clipboard_item, &open_item, &sep, &quit_item],
            )?;

            // macOS gets a monochrome template glyph, which the system tints to
            // match the menu bar. Nothing else has template images, so a black
            // glyph would vanish into a dark panel — those get the colour icon.
            #[cfg(target_os = "macos")]
            let tray_icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;
            #[cfg(not(target_os = "macos"))]
            let tray_icon = Image::from_bytes(include_bytes!("../icons/32x32.png"))?;

            TrayIconBuilder::new()
                .icon(tray_icon)
                .icon_as_template(cfg!(target_os = "macos"))
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_editor(app),
                    "palette" => toggle_palette(app),
                    "clipboard" => toggle_clipboard(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            // Real NSVisualEffectView blur behind the palette, so it reads as a
            // system overlay rather than a floating web page.
            #[cfg(target_os = "macos")]
            if let Some(win) = app.get_webview_window(PALETTE) {
                use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};
                let _ = apply_vibrancy(
                    &win,
                    NSVisualEffectMaterial::HudWindow,
                    None,
                    Some(PALETTE_CORNER_RADIUS),
                );
            }

            // Remembered images and the source-app icons are drawn the same
            // way attachments are. Recursive: the icons sit in a subdirectory.
            {
                let dir = history::clip_dir();
                let _ = std::fs::create_dir_all(&dir);
                if let Err(e) = handle.asset_protocol_scope().allow_directory(&dir, true) {
                    eprintln!("reze: clipboard previews unavailable ({e})");
                }
            }
            history::configure(&lib.settings);
            history::start(handle.clone());

            folders::ensure_ours(&lib.settings.folders);
            watch_folders(&handle, &lib.settings.folders);

            tailnet::configure(&lib.settings);
            tailnet::start(handle.clone());

            spawn_watcher(handle);
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // The palette is a transient overlay: losing focus means the user
            // moved on, so get out of the way — unless a download is running in
            // it, which would take the progress and the naming step with it.
            WindowEvent::Focused(false) if window.label() == PALETTE => {
                if !window.state::<Grabbing>().0.load(Ordering::SeqCst) {
                    let _ = window.hide();
                }
            }
            // Closing either window should not tear down the menu-bar app.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                let _ = window.hide();
            }
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("error while building reze")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                // A menu-bar app must survive its last window closing, which is
                // what this guard is for. But `code` is Some only when the exit
                // was asked for programmatically — i.e. the Quit menu item —
                // and refusing that made Quit do nothing at all.
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
