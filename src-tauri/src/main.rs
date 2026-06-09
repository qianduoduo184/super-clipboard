mod blobs;
mod clipboard;
mod commands;
mod diagnostics;
mod storage;
mod system;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use storage::repository::ClipboardRepository;
use system::settings::AppSettings;
use tauri::Manager;

pub struct AppState {
    pub repository: Arc<Mutex<ClipboardRepository>>,
    pub settings: Arc<Mutex<AppSettings>>,
    pub app_data_dir: PathBuf,
    pub settings_path: PathBuf,
    pub blob_dir: PathBuf,
    pub log_path: PathBuf,
}

fn main() {
    diagnostics::install_panic_hook();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let fallback_log_path = app_data.join("logs").join("super-clipboard.log");
            let log_path = match diagnostics::init(&app_data) {
                Ok(path) => path,
                Err(error) => {
                    eprintln!("failed to initialize diagnostics log: {error}");
                    fallback_log_path
                }
            };
            diagnostics::info("setup: app data directory resolved");

            let settings_path = app_data.join("settings.json");
            diagnostics::info(format!("setup: settings_path={}", settings_path.display()));

            let blob_dir = blobs::ensure_blob_dir(&app_data)?;
            diagnostics::info(format!("setup: blob_dir={}", blob_dir.display()));

            let database_path = app_data.join("super-clipboard.sqlite3");
            diagnostics::info(format!("setup: database_path={}", database_path.display()));
            let repository = match ClipboardRepository::open(database_path) {
                Ok(repository) => repository,
                Err(error) => {
                    diagnostics::error(format!("setup: sqlite repository failed: {error}"));
                    return Err(error.into());
                }
            };
            diagnostics::info("setup: sqlite repository opened");
            let repository = Arc::new(Mutex::new(repository));

            let loaded_settings = match AppSettings::load(&settings_path) {
                Ok(settings) => settings,
                Err(error) => {
                    diagnostics::error(format!(
                        "setup: settings load failed, using defaults: {error}"
                    ));
                    AppSettings::default()
                }
            };
            let settings = Arc::new(Mutex::new(loaded_settings));
            diagnostics::info("setup: settings loaded");
            app.manage(AppState {
                repository: repository.clone(),
                settings: settings.clone(),
                app_data_dir: app_data.clone(),
                settings_path: settings_path.clone(),
                blob_dir: blob_dir.clone(),
                log_path,
            });

            if let Err(error) = clipboard::start_background_listener(repository, settings, blob_dir) {
                diagnostics::error(format!("setup: clipboard listener failed: {error}"));
            } else {
                diagnostics::info("setup: clipboard listener started");
            }

            if let Err(error) = system::tray::setup(app) {
                diagnostics::error(format!("setup: tray setup failed: {error}"));
            } else {
                diagnostics::info("setup: tray initialized");
            }

            if let Err(error) = system::shortcuts::register_default_shortcuts(app) {
                diagnostics::error(format!("setup: global shortcut registration failed: {error}"));
            } else {
                diagnostics::info("setup: global shortcut registered");
            }

            diagnostics::info("setup: completed");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::search_items,
            commands::get_item_detail,
            commands::copy_item,
            commands::paste_item,
            commands::toggle_favorite,
            commands::delete_item,
            commands::set_recording_enabled,
            commands::get_settings,
            commands::update_settings,
            commands::clear_history,
            commands::get_diagnostics
        ])
        .run(tauri::generate_context!())
        .expect("failed to run super-clipboard");
}
