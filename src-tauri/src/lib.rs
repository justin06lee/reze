mod paste;
mod store;

use std::str::FromStr;
use std::sync::mpsc::channel;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use store::{AppState, Library};
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
    if let Some(win) = app.get_webview_window(PALETTE) {
        let _ = win.hide();
    }
    let restore = store::load()
        .map(|l| l.settings.restore_clipboard)
        .unwrap_or(true);

    tauri::async_runtime::spawn_blocking(move || {
        if copy_only {
            paste::write_clipboard(&text)
        } else {
            paste::paste(&text, restore)
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn hide_palette(app: AppHandle) {
    if let Some(win) = app.get_webview_window(PALETTE) {
        let _ = win.hide();
    }
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

#[tauri::command]
fn set_hotkey(app: AppHandle, hotkey: String) -> Result<(), String> {
    let shortcut = Shortcut::from_str(&hotkey).map_err(|e| format!("invalid hotkey: {e}"))?;
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| format!("could not register {hotkey}: {e}"))?;
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
    position_palette(app, &win);
    let _ = win.show();
    let _ = win.set_focus();
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
                .with_handler(|app, _shortcut, event| {
                    // Fire on press only; without this the palette toggles twice per tap.
                    if event.state() == ShortcutState::Pressed {
                        toggle_palette(app);
                    }
                })
                .build(),
        )
        .manage(AppState::new())
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
            open_editor,
            resize_palette,
            set_hotkey,
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

            if let Ok(shortcut) = Shortcut::from_str(&lib.settings.hotkey) {
                if let Err(e) = handle.global_shortcut().register(shortcut) {
                    eprintln!("reze: could not register {}: {e}", lib.settings.hotkey);
                }
            }

            let open_item = MenuItem::with_id(app, "open", "Macro Editor…", true, None::<&str>)?;
            let palette_item =
                MenuItem::with_id(app, "palette", "Show Palette", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit Reze", true, None::<&str>)?;
            let sep = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&palette_item, &open_item, &sep, &quit_item])?;

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
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
