mod blobs;
mod clipboard;
mod commands;
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
    pub settings_path: PathBuf,
    pub blob_dir: PathBuf,
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .setup(|app| {
            let app_data = app.path().app_data_dir()?;
            let settings_path = app_data.join("settings.json");
            let blob_dir = blobs::ensure_blob_dir(&app_data)?;
            let repository = ClipboardRepository::open(app_data.join("super-clipboard.sqlite3"))?;
            let repository = Arc::new(Mutex::new(repository));
            let settings = Arc::new(Mutex::new(AppSettings::load(&settings_path)?));
            app.manage(AppState {
                repository: repository.clone(),
                settings: settings.clone(),
                settings_path: settings_path.clone(),
                blob_dir: blob_dir.clone(),
            });
            clipboard::start_background_listener(repository, settings, blob_dir)?;
            system::tray::setup(app)?;
            system::shortcuts::register_default_shortcuts(app)?;
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
            commands::clear_history
        ])
        .run(tauri::generate_context!())
        .expect("failed to run super-clipboard");
}
