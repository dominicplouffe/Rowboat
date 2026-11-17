//! Rowboat's Rust core.
//!
//! All database, AI and secret handling lives on this side of the IPC boundary;
//! the webview only renders. M0 provides the settings store and app metadata.

pub mod error;
pub mod settings;

use std::path::PathBuf;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_dialog::DialogExt;

use error::{Error, Result};
use settings::{Settings, SettingsStore};

/// When set, the app writes its startup time to this file and exits once the
/// window is usable. Used by `scripts/check-startup-budget.mjs`; a local
/// measurement only, nothing is ever sent anywhere.
const BENCH_OUT_ENV: &str = "ROWBOAT_BENCH_OUT";

struct StartedAt(Instant);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    name: &'static str,
    version: String,
    os: &'static str,
    arch: &'static str,
    default_client_id: String,
    settings_path: String,
}

#[tauri::command]
fn app_info(app: AppHandle, store: State<'_, SettingsStore>) -> AppInfo {
    AppInfo {
        name: "Rowboat",
        version: app.package_info().version.to_string(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        default_client_id: settings::default_client_id(),
        settings_path: store.path().display().to_string(),
    }
}

/// Called by the UI after its first paint.
#[tauri::command]
fn app_ready(app: AppHandle, started: State<'_, StartedAt>) {
    let Some(out) = std::env::var_os(BENCH_OUT_ENV) else {
        return;
    };
    let ms = started.0.elapsed().as_secs_f64() * 1000.0;
    let _ = std::fs::write(&out, format!("{{\"startupMs\":{ms:.1}}}\n"));
    // Optionally stay up so the harness can sample idle memory.
    let idle_ms: u64 = std::env::var("ROWBOAT_BENCH_IDLE_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(idle_ms));
        app.exit(0);
    });
}

#[tauri::command]
fn settings_get(store: State<'_, SettingsStore>) -> Settings {
    store.get()
}

#[tauri::command]
fn settings_set(store: State<'_, SettingsStore>, settings: Settings) -> Result<Settings> {
    store.set(settings)
}

#[tauri::command]
fn settings_reset(store: State<'_, SettingsStore>) -> Result<Settings> {
    store.reset()
}

/// Opens a native save dialog and writes the settings there. Returns the chosen
/// path, or `None` when the dialog was cancelled. The dialog is driven from
/// Rust so the webview needs no filesystem access.
#[tauri::command]
async fn settings_export(
    app: AppHandle,
    store: State<'_, SettingsStore>,
) -> Result<Option<String>> {
    let picked = app
        .dialog()
        .file()
        .set_title("Export Rowboat settings")
        .set_file_name("rowboat-settings.json")
        .add_filter("JSON", &["json"])
        .blocking_save_file();
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    store.export_to(&path)?;
    Ok(Some(path.display().to_string()))
}

/// Opens a native file dialog and imports the chosen settings file. Returns the
/// new settings, or `None` when the dialog was cancelled.
#[tauri::command]
async fn settings_import(
    app: AppHandle,
    store: State<'_, SettingsStore>,
) -> Result<Option<Settings>> {
    let picked = app
        .dialog()
        .file()
        .set_title("Import Rowboat settings")
        .add_filter("JSON", &["json"])
        .blocking_pick_file();
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else {
        return Ok(None);
    };
    store.import_from(&path).map(Some)
}

/// Reveals the folder that holds the settings file in the OS file manager.
#[tauri::command]
fn open_data_folder(store: State<'_, SettingsStore>) -> Result<()> {
    let dir = store
        .path()
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| Error::Other("settings file has no parent folder".into()))?;
    std::fs::create_dir_all(&dir)?;
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener).arg(&dir).spawn()?;
    Ok(())
}

/// The macOS menu bar. Tauri's default menu binds ⌘W to "Close Window", which
/// would steal the shortcut from "Close tab"; this one leaves ⌘W to the app and
/// keeps the Edit menu, which macOS needs for ⌘C/⌘V/⌘Z to work in the webview.
#[cfg(target_os = "macos")]
fn macos_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{Menu, PredefinedMenuItem as Item, Submenu};

    let app_menu = Submenu::with_items(
        app,
        "Rowboat",
        true,
        &[
            &Item::about(app, None, None)?,
            &Item::separator(app)?,
            &Item::services(app, None)?,
            &Item::separator(app)?,
            &Item::hide(app, None)?,
            &Item::hide_others(app, None)?,
            &Item::show_all(app, None)?,
            &Item::separator(app)?,
            &Item::quit(app, None)?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &Item::undo(app, None)?,
            &Item::redo(app, None)?,
            &Item::separator(app)?,
            &Item::cut(app, None)?,
            &Item::copy(app, None)?,
            &Item::paste(app, None)?,
            &Item::select_all(app, None)?,
        ],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &Item::minimize(app, None)?,
            &Item::maximize(app, None)?,
            &Item::separator(app)?,
            &Item::fullscreen(app, None)?,
        ],
    )?;
    Menu::with_items(app, &[&app_menu, &edit, &window])
}

pub fn run() {
    let started = StartedAt(Instant::now());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(started)
        .setup(|app| {
            let dir = match std::env::var_os("ROWBOAT_CONFIG_DIR") {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_config_dir()?,
            };
            app.manage(SettingsStore::load(dir.join("settings.json")));
            #[cfg(target_os = "macos")]
            app.set_menu(macos_menu(app.handle())?)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            app_ready,
            settings_get,
            settings_set,
            settings_reset,
            settings_export,
            settings_import,
            open_data_folder,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rowboat");
}
