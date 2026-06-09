use tauri::State;

use crate::storage::repository::{ClipboardItem, SearchFilters};
use crate::system::settings::AppSettings;
use crate::AppState;

#[tauri::command]
pub fn search_items(
    state: State<'_, AppState>,
    query: String,
    filters: SearchFilters,
    limit: i64,
    cursor: Option<i64>,
) -> Result<Vec<ClipboardItem>, String> {
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository
        .search(query, filters, limit.clamp(1, 100), cursor)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn get_item_detail(state: State<'_, AppState>, id: String) -> Result<Option<ClipboardItem>, String> {
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository.get_item(&id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn copy_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
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

    Err(format!("copy is not implemented for {} items", item.item_type))
}

#[tauri::command]
pub fn paste_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
    copy_item(state, id)?;
    #[cfg(target_os = "windows")]
    crate::clipboard::win::simulate_paste_shortcut().map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn toggle_favorite(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository.toggle_favorite(&id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository.soft_delete(&id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_recording_enabled(state: State<'_, AppState>, enabled: bool) -> Result<(), String> {
    let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
    settings.recording_enabled = enabled;
    settings
        .save(&state.settings_path)
        .map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    let settings = state.settings.lock().map_err(|error| error.to_string())?;
    Ok(settings.clone())
}

#[tauri::command]
pub fn update_settings(state: State<'_, AppState>, next_settings: AppSettings) -> Result<AppSettings, String> {
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
pub fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository.clear_history().map_err(|error| error.to_string())?;
    crate::blobs::clear_blob_dir(&state.blob_dir).map_err(|error| error.to_string())?;
    Ok(())
}
