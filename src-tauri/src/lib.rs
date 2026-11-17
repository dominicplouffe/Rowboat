//! Rowboat's Rust core.
//!
//! All database, AI and secret handling lives on this side of the IPC boundary;
//! the webview only renders. M0 provides the settings store and app metadata.

pub mod connections;
pub mod db;
pub mod error;
pub mod manager;
pub mod secrets;
pub mod settings;
pub mod ssh;
pub mod statements;
pub mod tls;

use std::path::PathBuf;
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager as _, State};
use tauri_plugin_dialog::DialogExt;

use connections::{AppDb, ConnectionConfig};
use db::{DbKind, DbResult, SchemaNode};
use error::{Error, Result};
use manager::{ConnectionView, Manager, RunResult, SecretInput};
use secrets::Secrets;
use settings::{Settings, SettingsStore};
use statements::StatementSpan;

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
    /// False when there is no OS keychain and secrets last only until quit.
    secrets_persistent: bool,
}

#[tauri::command]
async fn app_info(
    app: AppHandle,
    store: State<'_, SettingsStore>,
    manager: State<'_, Manager>,
) -> Result<AppInfo> {
    // Async on purpose: finding out whether a keychain exists can take a
    // moment and must not run on the main thread.
    Ok(AppInfo {
        name: "Rowboat",
        version: app.package_info().version.to_string(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        default_client_id: settings::default_client_id(),
        settings_path: store.path().display().to_string(),
        secrets_persistent: manager.secrets_are_persistent(),
    })
}

/// Milliseconds since the harness launched this process, if it told us when
/// (`ROWBOAT_BENCH_T0`, Unix epoch milliseconds). Unlike a clock started in
/// `run()`, this includes process creation and loading the webview libraries.
fn ms_since_launch() -> Option<f64> {
    let t0: f64 = std::env::var("ROWBOAT_BENCH_T0").ok()?.parse().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64()
        * 1000.0;
    Some(now - t0)
}

/// Called by the UI once the window is usable (the editor accepts input).
#[tauri::command]
fn app_ready(app: AppHandle, started: State<'_, StartedAt>) {
    let Some(out) = std::env::var_os(BENCH_OUT_ENV) else {
        return;
    };
    let ms = ms_since_launch().unwrap_or_else(|| started.0.elapsed().as_secs_f64() * 1000.0);
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

// ---------------------------------------------------------- connections ----
//
// Secrets only ever flow inwards through these commands (`SecretInput`);
// nothing returned to the webview contains one.

#[tauri::command]
async fn connections_list(manager: State<'_, Manager>) -> DbResult<Vec<ConnectionView>> {
    manager.list().await
}

#[tauri::command]
async fn connection_save(
    manager: State<'_, Manager>,
    config: ConnectionConfig,
    secrets: SecretInput,
) -> DbResult<ConnectionView> {
    manager.save(config, secrets).await
}

#[tauri::command]
async fn connection_delete(manager: State<'_, Manager>, id: String) -> DbResult<()> {
    manager.delete(&id).await
}

/// Tries a connection without saving it. Returns the server version.
#[tauri::command]
async fn connection_test(
    manager: State<'_, Manager>,
    store: State<'_, SettingsStore>,
    config: ConnectionConfig,
    secrets: SecretInput,
    trusted_fingerprint: Option<String>,
) -> DbResult<String> {
    manager
        .test(config, secrets, &store.get(), trusted_fingerprint)
        .await
}

#[tauri::command]
async fn connection_open(
    manager: State<'_, Manager>,
    store: State<'_, SettingsStore>,
    id: String,
    trusted_fingerprint: Option<String>,
) -> DbResult<ConnectionView> {
    manager.open(&id, &store.get(), trusted_fingerprint).await
}

#[tauri::command]
async fn connection_close(manager: State<'_, Manager>, id: String) -> DbResult<()> {
    manager.close(&id).await;
    Ok(())
}

/// Runs one statement. The AI never calls this: only the Run actions do.
#[tauri::command]
async fn query_run(
    manager: State<'_, Manager>,
    store: State<'_, SettingsStore>,
    connection_id: String,
    query_id: String,
    sql: String,
    row_limit: u32,
    confirmed: bool,
) -> DbResult<RunResult> {
    manager
        .run(
            &connection_id,
            &query_id,
            &sql,
            row_limit,
            confirmed,
            &store.get(),
        )
        .await
}

#[tauri::command]
async fn query_cancel(manager: State<'_, Manager>, query_id: String) -> DbResult<()> {
    manager.cancel(&query_id).await;
    Ok(())
}

#[tauri::command]
async fn transaction_end(
    manager: State<'_, Manager>,
    connection_id: String,
    commit: bool,
) -> DbResult<()> {
    manager.end_transaction(&connection_id, commit).await
}

#[tauri::command]
async fn connection_set_auto_commit(
    manager: State<'_, Manager>,
    connection_id: String,
    auto_commit: bool,
) -> DbResult<()> {
    manager.set_auto_commit(&connection_id, auto_commit).await
}

#[tauri::command]
async fn schema_children(
    manager: State<'_, Manager>,
    connection_id: String,
    path: Vec<String>,
) -> DbResult<Vec<SchemaNode>> {
    manager.children(&connection_id, &path).await
}

#[tauri::command]
async fn schema_ddl(
    manager: State<'_, Manager>,
    connection_id: String,
    path: Vec<String>,
) -> DbResult<String> {
    manager.ddl(&connection_id, &path).await
}

/// Splits editor text into statements (offsets in UTF-16 code units).
#[tauri::command]
fn sql_split(kind: DbKind, text: String) -> Vec<StatementSpan> {
    statements::split(kind, &text)
}

/// Opens a native file dialog and returns the chosen path. The dialog runs in
/// Rust, so the webview needs no filesystem access.
#[tauri::command]
async fn pick_file(app: AppHandle, purpose: String) -> Result<Option<String>> {
    let dialog = app.dialog().file();
    let picked = match purpose.as_str() {
        "sqlite-new" => dialog
            .set_title("Create a SQLite database")
            .set_file_name("database.sqlite")
            .blocking_save_file(),
        "sqlite-open" => dialog
            .set_title("Open a SQLite database")
            .add_filter("SQLite", &["sqlite", "sqlite3", "db", "db3"])
            .add_filter("All files", &["*"])
            .blocking_pick_file(),
        "certificate" => dialog
            .set_title("Choose a certificate (PEM)")
            .add_filter("Certificates", &["pem", "crt", "cer", "key"])
            .add_filter("All files", &["*"])
            .blocking_pick_file(),
        _ => dialog.set_title("Choose a file").blocking_pick_file(),
    };
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.display().to_string()))
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
    tls::ensure_provider();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(started)
        .setup(|app| {
            let dir = match std::env::var_os("ROWBOAT_CONFIG_DIR") {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_config_dir()?,
            };
            app.manage(SettingsStore::load(dir.join("settings.json")));
            // Tests and benchmarks must not touch the user's real keychain.
            let secrets = if std::env::var_os("ROWBOAT_CONFIG_DIR").is_some() {
                Secrets::in_memory()
            } else {
                Secrets::system()
            };
            app.manage(Manager::new(AppDb::open(dir.join("rowboat.db"))?, secrets));
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
            connections_list,
            connection_save,
            connection_delete,
            connection_test,
            connection_open,
            connection_close,
            query_run,
            query_cancel,
            transaction_end,
            connection_set_auto_commit,
            schema_children,
            schema_ddl,
            sql_split,
            pick_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Rowboat");
}
