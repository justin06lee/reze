mod expand;
mod focus;
mod paste;
mod store;

use std::str::FromStr;
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
const PALETTE_CORNER_RADIUS: f64 = 14.0;

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_library() -> Result<Library, String> {
    store::load()
}

#[tauri::command]
fn save_library(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    library: Library,
) -> Result<(), String> {
    let written = store::save(&library)?;
    *state.last_written.lock().unwrap() = written;
    app.emit("library-changed", &library).ok();
    Ok(())
}

/// Increment the use counter so the palette can rank by what you actually reach for.
#[tauri::command]
fn bump_usage(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let mut lib = store::load()?;
    if let Some(m) = lib.macros.iter_mut().find(|m| m.id == id) {
        m.usage_count += 1;
    }
    let written = store::save(&lib)?;
    *state.last_written.lock().unwrap() = written;
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

#[tauri::command]
fn open_accessibility_settings() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
}

/// Hide the palette, then deliver `text` to whatever app regained focus.
#[tauri::command]
async fn deliver(app: AppHandle, text: String, copy_only: bool) -> Result<(), String> {
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
        if let Some(pid) = previous {
            let _ = on_main(&app, move || focus::activate_pid(pid));
            std::thread::sleep(Duration::from_millis(60));
        }
    }

    let restore = store::load()
        .map(|l| l.settings.restore_clipboard)
        .unwrap_or(true);

    let worker = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        if copy_only {
            paste::write_clipboard(&text)
        } else {
            paste::paste(&text, restore, |f| {
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
    palette: std::sync::Mutex<Option<Shortcut>>,
    expand: std::sync::Mutex<Option<Shortcut>>,
}

#[tauri::command]
fn set_hotkeys(app: AppHandle, palette: String, expand: String) -> Result<(), String> {
    register_hotkeys(&app, &palette, &expand)
}

/// Registers both shortcuts, replacing whatever was registered before.
///
/// Parsed up front so an invalid accelerator is rejected before we tear down
/// the working ones — otherwise a typo in Settings would leave the user with
/// no way to open the app at all.
fn register_hotkeys(app: &AppHandle, palette: &str, expand: &str) -> Result<(), String> {
    let palette_shortcut =
        Shortcut::from_str(palette).map_err(|e| format!("invalid palette hotkey: {e}"))?;
    let expand_shortcut =
        Shortcut::from_str(expand).map_err(|e| format!("invalid expand hotkey: {e}"))?;

    app.global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;

    app.global_shortcut()
        .register(palette_shortcut)
        .map_err(|e| format!("could not register {palette}: {e}"))?;
    app.global_shortcut()
        .register(expand_shortcut)
        .map_err(|e| format!("could not register {expand}: {e}"))?;

    let state = app.state::<Hotkeys>();
    *state.palette.lock().unwrap() = Some(palette_shortcut);
    *state.expand.lock().unwrap() = Some(expand_shortcut);
    Ok(())
}

#[tauri::command]
fn library_path() -> String {
    store::library_path().to_string_lossy().to_string()
}

#[tauri::command]
fn reveal_library() {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(store::library_path())
            .spawn();
    }
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
/// Skips our own process: opening the palette twice in a row must not record
/// Reze as the app to paste into.
fn remember_frontmost(app: &AppHandle) {
    let ours = std::process::id() as i32;
    if let Some(pid) = focus::frontmost_pid() {
        if pid != ours {
            *app.state::<AppState>().previous_app.lock().unwrap() = Some(pid);
        }
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
}

fn toggle_palette(app: &AppHandle) {
    let Some(win) = app.get_webview_window(PALETTE) else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
        return;
    }
    // Tell the UI to reset before it becomes visible, so the previous query
    // never flashes on screen.
    let _ = app.emit_to(PALETTE, "palette-opened", ());
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

    let library = store::load()?;
    let names: Vec<String> = library.macros.iter().map(|m| m.name.clone()).collect();
    if names.is_empty() {
        return Ok(());
    }
    let span = expand::words_to_select(&names);

    // Borrow the clipboard to read the text back, then hand it straight back to
    // the user before anything else can go wrong.
    let saved = paste::read_clipboard().unwrap_or_default();
    let before = focus::pasteboard_change_count();

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
        if focus::pasteboard_change_count() != before {
            selection = paste::read_clipboard().ok();
            break;
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

            let Ok(raw) = std::fs::read_to_string(store::library_path()) else {
                continue;
            };
            {
                let state = app.state::<AppState>();
                let mut last = state.last_written.lock().unwrap();
                if *last == raw {
                    continue; // our own save
                }
                *last = raw.clone();
            }
            match serde_json::from_str::<Library>(&raw) {
                Ok(lib) => {
                    let _ = app.emit("library-changed", &lib);
                }
                Err(e) => {
                    let _ = app.emit("library-error", e.to_string());
                }
            }
        }
    });
}

// ---------------------------------------------------------------- setup

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
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
                    } else if is(&hotkeys.palette) {
                        toggle_palette(app);
                    }
                })
                .build(),
        )
        .manage(AppState::new())
        .manage(Hotkeys::default())
        .invoke_handler(tauri::generate_handler![
            get_library,
            save_library,
            bump_usage,
            read_clipboard,
            accessibility_status,
            request_accessibility,
            open_accessibility_settings,
            deliver,
            hide_palette,
            show_palette,
            open_editor,
            resize_palette,
            set_hotkeys,
            library_path,
            reveal_library,
        ])
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let handle = app.handle().clone();

            let lib = store::load().unwrap_or_default();
            if let Ok(body) = store::serialize(&lib) {
                *app.state::<AppState>().last_written.lock().unwrap() = body;
            }

            if let Err(e) = register_hotkeys(&handle, &lib.settings.hotkey, &lib.settings.expand_hotkey)
            {
                eprintln!("reze: {e}");
            }

            let open_item = MenuItem::with_id(app, "open", "Macro Editor…", true, None::<&str>)?;
            let palette_item =
                MenuItem::with_id(app, "palette", "Show Palette", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Reze", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&palette_item, &open_item, &sep, &quit_item])?;

            // A dedicated monochrome template glyph — the full-colour app icon
            // would look wrong in the menu bar and would not tint with the
            // system appearance. Embedded rather than bundled as a resource so
            // it cannot go missing at runtime.
            let tray_icon = Image::from_bytes(include_bytes!("../icons/tray.png"))?;

            TrayIconBuilder::new()
                .icon(tray_icon)
                .icon_as_template(true)
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_editor(app),
                    "palette" => toggle_palette(app),
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

            spawn_watcher(handle);
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // The palette is a transient overlay: losing focus means the user
            // moved on, so get out of the way.
            WindowEvent::Focused(false) if window.label() == PALETTE => {
                let _ = window.hide();
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
            if let tauri::RunEvent::ExitRequested { api, .. } = event {
                api.prevent_exit();
            }
        });
}
