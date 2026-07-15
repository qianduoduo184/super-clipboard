#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod blobs;
mod clipboard;
mod commands;
mod diagnostics;
mod storage;
mod system;

#[cfg(test)]
mod security_tests;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use blobs::store::ImageBlobStore;
use storage::repository::ClipboardRepository;
use system::settings::AppSettings;
use tauri::{Emitter, Manager, WindowEvent};

pub struct AppState {
    pub repository: Arc<Mutex<ClipboardRepository>>,
    pub settings: Arc<Mutex<AppSettings>>,
    pub current_shortcut: Arc<Mutex<Option<String>>>,
    pub app_data_dir: PathBuf,
    pub settings_path: PathBuf,
    pub blob_dir: PathBuf,
    pub image_store: Arc<ImageBlobStore>,
    pub capacity_status: Arc<Mutex<storage::capacity::ClipboardCapacityStatus>>,
    pub log_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StartupCapacityState {
    usage: u64,
    blocked: bool,
}

fn initialize_startup_capacity_status(
    state: &Mutex<storage::capacity::ClipboardCapacityStatus>,
    capacity: StartupCapacityState,
) -> anyhow::Result<storage::capacity::ClipboardCapacityStatus> {
    storage::capacity::update_capacity_status_with(state, capacity.blocked, 0, |_| Ok(()))
}

fn run_startup_capacity_then<T>(
    repository: &Mutex<ClipboardRepository>,
    image_store: &ImageBlobStore,
    max_history_items: i64,
    retention_days: i64,
    mut on_step: impl FnMut(&'static str),
    start_listener: impl FnOnce(StartupCapacityState) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let startup_prune = storage::capacity::prune_for_capacity(
        repository,
        image_store,
        max_history_items,
        retention_days,
        0,
    );
    match startup_prune {
        Ok(_) => {}
        Err(error)
            if error
                .downcast_ref::<storage::capacity::CapacityError>()
                .is_some() =>
        {
            diagnostics::warn(format!(
                "setup: startup capacity prune could not reach quota: {error}"
            ));
        }
        Err(error) => return Err(error),
    }
    on_step("prune");
    let usage = image_store.managed_usage()?;
    on_step("usage");
    start_listener(StartupCapacityState {
        usage,
        blocked: usage > storage::capacity::MANAGED_BLOB_QUOTA,
    })
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
        .plugin(tauri_plugin_updater::Builder::new().build())
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
            let image_store = Arc::new(ImageBlobStore::new(
                blob_dir.clone(),
                app_data.join("blob-stage"),
            )?);

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

            let migration_outcome = match storage::image_migration::run_image_migration(
                repository.as_ref(),
                image_store.as_ref(),
            ) {
                Ok(outcome) => outcome,
                Err(error) => {
                    diagnostics::error(format!("setup: image migration failed: {error:#}"));
                    return Err(error.into());
                }
            };
            diagnostics::info(format!(
                "setup: image migration completed, managed_usage={}, quota_blocked={}",
                migration_outcome.usage, migration_outcome.quota_blocked
            ));

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
            let current_shortcut = Arc::new(Mutex::new(None));
            let capacity_status = Arc::new(Mutex::new(
                storage::capacity::ClipboardCapacityStatus::default(),
            ));
            diagnostics::info("setup: settings loaded");
            if let Some(window) = app.get_webview_window("main") {
                let window_for_close = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        diagnostics::info("window: close requested, hiding to tray");
                        api.prevent_close();
                        let _ = window_for_close.hide();
                    }
                });
            }
            let startup_settings = settings
                .lock()
                .map(|settings| settings.clone())
                .unwrap_or_default();
            run_startup_capacity_then(
                repository.as_ref(),
                image_store.as_ref(),
                startup_settings.max_history_items,
                startup_settings.retention_days,
                |_| {},
                |capacity| {
                    let initial_status =
                        initialize_startup_capacity_status(&capacity_status, capacity)?;
                    diagnostics::info(format!(
                        "setup: startup capacity prune completed, managed_usage={}, migration_quota_blocked={}, image_capture_blocked={}",
                        capacity.usage, migration_outcome.quota_blocked, capacity.blocked
                    ));
                    app.manage(AppState {
                        repository: repository.clone(),
                        settings: settings.clone(),
                        current_shortcut: current_shortcut.clone(),
                        app_data_dir: app_data.clone(),
                        settings_path: settings_path.clone(),
                        blob_dir: blob_dir.clone(),
                        image_store: image_store.clone(),
                        capacity_status: capacity_status.clone(),
                        log_path: log_path.clone(),
                    });
                    if initial_status.blocked {
                        if let Err(error) = app.handle().emit("clipboard-status", &initial_status) {
                            diagnostics::warn(format!(
                                "setup: failed to emit initial capacity status: {error}"
                            ));
                        }
                    }
                    if let Err(error) = clipboard::start_background_listener(
                        app.handle().clone(),
                        repository.clone(),
                        settings.clone(),
                        image_store.clone(),
                        capacity_status.clone(),
                    ) {
                        diagnostics::error(format!("setup: clipboard listener failed: {error}"));
                    } else {
                        diagnostics::info("setup: clipboard listener started");
                    }
                    Ok(())
                },
            )?;

            if let Err(error) = system::tray::setup(app) {
                diagnostics::error(format!("setup: tray setup failed: {error}"));
            } else {
                diagnostics::info("setup: tray initialized");
            }

            let startup_shortcut = settings
                .lock()
                .map(|settings| settings.global_shortcut.clone())
                .unwrap_or_else(|_| AppSettings::default().global_shortcut);
            if let Err(error) = system::shortcuts::register_shortcut(
                app.handle(),
                &startup_shortcut,
                current_shortcut,
            ) {
                diagnostics::error(format!(
                    "setup: global shortcut registration failed: {error}"
                ));
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
            commands::toggle_pin,
            commands::delete_item,
            commands::reorder_items,
            commands::set_recording_enabled,
            commands::get_settings,
            commands::get_clipboard_status,
            commands::update_settings,
            commands::set_global_shortcut,
            commands::clear_history,
            commands::get_diagnostics,
            commands::check_update,
            commands::install_update,
            commands::select_directory,
            commands::migrate_directory,
            commands::update_storage_settings,
            commands::export_backup,
            commands::select_backup_file,
            commands::parse_backup_info,
            commands::import_backup
        ])
        .run(tauri::generate_context!())
        .expect("failed to run super-clipboard");
}

#[cfg(test)]
mod startup_capacity_tests {
    use std::cell::RefCell;
    use std::fs;
    use std::sync::Mutex;

    use uuid::Uuid;

    use super::{initialize_startup_capacity_status, run_startup_capacity_then};
    use crate::blobs::store::ImageBlobStore;
    use crate::storage::repository::ClipboardRepository;

    #[test]
    fn startup_prune_and_usage_complete_before_listener_starts() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-startup-capacity-{}",
            Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let pending = store.blob_dir().join("startup-pending.tmp");
        fs::write(&pending, b"pending").expect("pending blob");
        repository
            .lock()
            .expect("repository lock")
            .update_image_references_and_enqueue_cleanup(&[], &[pending.clone()])
            .expect("enqueue pending cleanup");
        let events = RefCell::new(Vec::new());

        run_startup_capacity_then(
            &repository,
            &store,
            10_000,
            30,
            |event| events.borrow_mut().push(event),
            |state| {
                events.borrow_mut().push("listener");
                assert!(!pending.exists());
                assert_eq!(state.usage, 0);
                Ok(())
            },
        )
        .expect("startup chain");

        assert_eq!(&*events.borrow(), &["prune", "usage", "listener"]);
        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn startup_blocked_status_is_queryable_before_listener_starts() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-startup-blocked-status-{}",
            Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        fs::File::create(store.blob_dir().join("startup-orphan.tmp"))
            .expect("startup orphan")
            .set_len(crate::storage::capacity::MANAGED_BLOB_QUOTA + 1)
            .expect("sparse startup orphan");
        let status_state = Mutex::new(crate::storage::capacity::ClipboardCapacityStatus::default());

        run_startup_capacity_then(
            &repository,
            &store,
            10_000,
            30,
            |_| {},
            |capacity| {
                let initialized = initialize_startup_capacity_status(&status_state, capacity)?;
                assert!(initialized.blocked);
                assert_eq!(initialized.revision, 1);
                assert_eq!(
                    crate::storage::capacity::current_capacity_status(&status_state)?,
                    initialized
                );
                Ok(())
            },
        )
        .expect("blocked startup chain");

        drop(repository);
        drop(store);
        fs::remove_dir_all(root).expect("cleanup");
    }
}
