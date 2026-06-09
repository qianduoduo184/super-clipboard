use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

use crate::storage::repository::{ClipboardItem, SearchFilters};
use crate::system::settings::AppSettings;
use crate::AppState;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticsInfo {
    pub app_data_dir: String,
    pub log_path: String,
}

#[tauri::command]
pub fn search_items(
    state: State<'_, AppState>,
    query: String,
    filters: SearchFilters,
    limit: i64,
    cursor: Option<i64>,
) -> Result<Vec<ClipboardItem>, String> {
    crate::diagnostics::info(format!(
        "command: search_items query_len={} limit={} cursor={cursor:?}",
        query.len(),
        limit
    ));
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository
        .search(query, filters, limit.clamp(1, 100), cursor)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_item_detail(state: State<'_, AppState>, id: String) -> Result<Option<ClipboardItem>, String> {
    crate::diagnostics::info(format!("command: get_item_detail id={id}"));
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository.get_item(&id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn copy_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
    crate::diagnostics::info(format!("command: copy_item id={id}"));
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    let item = repository
        .get_item(&id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "clipboard item not found".to_string())?;
    if item.item_type == "text" || item.item_type == "html" {
        if let Some(content) = item.content {
            #[cfg(target_os = "windows")]
            crate::clipboard::win::write_text_to_clipboard(&content)
                .map_err(|error| error.to_string())?;
        }
        return Ok(());
    }

    let error = format!("copy is not implemented for {} items", item.item_type);
    crate::diagnostics::warn(format!("command: copy_item failed: {error}"));
    Err(error)
}

#[tauri::command]
pub fn paste_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
    crate::diagnostics::info(format!("command: paste_item id={id}"));
    copy_item(state, id)?;
    #[cfg(target_os = "windows")]
    crate::clipboard::win::simulate_paste_shortcut().map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn toggle_favorite(state: State<'_, AppState>, id: String) -> Result<(), String> {
    crate::diagnostics::info(format!("command: toggle_favorite id={id}"));
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository.toggle_favorite(&id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
    crate::diagnostics::info(format!("command: delete_item id={id}"));
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository.soft_delete(&id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_recording_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    crate::diagnostics::info(format!("command: set_recording_enabled enabled={enabled}"));
    let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
    settings.recording_enabled = enabled;
    settings
        .save(&state.settings_path)
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    crate::diagnostics::info("command: get_settings");
    let settings = state.settings.lock().map_err(|error| error.to_string())?;
    Ok(settings.clone())
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    next_settings: AppSettings,
) -> Result<AppSettings, String> {
    crate::diagnostics::info("command: update_settings");
    let current_shortcut = {
        let settings = state.settings.lock().map_err(|error| error.to_string())?;
        settings.global_shortcut.clone()
    };
    let mut next_settings = next_settings;
    next_settings.global_shortcut = current_shortcut;
    apply_autostart_setting(&app, next_settings.autostart_enabled)
        .map_err(|error| error.to_string())?;
    next_settings
        .save(&state.settings_path)
        .map_err(|error| error.to_string())?;
    {
        let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
        *settings = next_settings.clone();
    }
    {
        let repository = state.repository.lock().map_err(|error| error.to_string())?;
        repository
            .prune_history(next_settings.max_history_items, next_settings.retention_days)
            .map_err(|error| error.to_string())?;
    }
    Ok(next_settings)
}

#[tauri::command]
pub fn set_global_shortcut(
    app: AppHandle,
    state: State<'_, AppState>,
    shortcut: String,
) -> Result<AppSettings, String> {
    crate::diagnostics::info(format!("command: set_global_shortcut shortcut={shortcut}"));
    crate::system::shortcuts::replace_shortcut(&app, &shortcut, state.current_shortcut.clone())
        .map_err(|error| error.to_string())?;

    let next_settings = {
        let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
        settings.global_shortcut = shortcut;
        settings.clone()
    };
    next_settings
        .save(&state.settings_path)
        .map_err(|error| error.to_string())?;
    Ok(next_settings)
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    crate::diagnostics::warn("command: clear_history");
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository.clear_history().map_err(|error| error.to_string())?;
    crate::blobs::clear_blob_dir(&state.blob_dir).map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_diagnostics(state: State<'_, AppState>) -> Result<DiagnosticsInfo, String> {
    let log_path = crate::diagnostics::log_path().unwrap_or_else(|| state.log_path.clone());
    Ok(DiagnosticsInfo {
        app_data_dir: state.app_data_dir.to_string_lossy().to_string(),
        log_path: log_path.to_string_lossy().to_string(),
    })
}

fn apply_autostart_setting(app: &AppHandle, enabled: bool) -> anyhow::Result<()> {
    if enabled {
        app.autolaunch().enable()?;
        crate::diagnostics::info("autostart: enabled");
    } else {
        app.autolaunch().disable()?;
        crate::diagnostics::info("autostart: disabled");
    }
    Ok(())
}
