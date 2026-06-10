use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_updater::UpdaterExt;

use std::path::Path;

use crate::storage::repository::{ClipboardItem, SearchFilters};
use crate::system::settings::AppSettings;
use crate::AppState;

#[derive(Debug, Clone, serde::Serialize)]
pub struct DiagnosticsInfo {
    pub app_data_dir: String,
    pub log_path: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UpdateInfo {
    pub available: bool,
    pub version: Option<String>,
    pub body: Option<String>,
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
            let content = if item.item_type == "html" {
                html_to_plain_text(&content)
            } else {
                content
            };
            #[cfg(target_os = "windows")]
            crate::clipboard::win::write_text_to_clipboard(&content)
                .map_err(|error| error.to_string())?;
        }
        return Ok(());
    }
    if item.item_type == "image" {
        let content_path = item
            .content_path
            .ok_or_else(|| "image item has no blob path".to_string())?;
        let dib_bytes = crate::blobs::read_dib_from_bmp_file(Path::new(&content_path))
            .map_err(|error| error.to_string())?;
        #[cfg(target_os = "windows")]
        crate::clipboard::win::write_dib_to_clipboard(&dib_bytes)
            .map_err(|error| error.to_string())?;
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
pub fn reorder_items(state: State<'_, AppState>, ids: Vec<String>) -> Result<(), String> {
    crate::diagnostics::info(format!("command: reorder_items count={}", ids.len()));
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository.reorder_items(&ids).map_err(|error| error.to_string())
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
    let (current_shortcut, current_update_check_date) = {
        let settings = state.settings.lock().map_err(|error| error.to_string())?;
        (settings.global_shortcut.clone(), settings.last_update_check_date.clone())
    };
    let mut next_settings = next_settings;
    next_settings.global_shortcut = current_shortcut;
    next_settings.last_update_check_date = current_update_check_date;
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
pub async fn check_update(app: AppHandle, state: State<'_, AppState>) -> Result<UpdateInfo, String> {
    crate::diagnostics::info("command: check_update");
    let today = chrono::Local::now().date_naive().to_string();
    let update = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?;

    {
        let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
        settings.last_update_check_date = Some(today);
        settings
            .save(&state.settings_path)
            .map_err(|error| error.to_string())?;
    }

    Ok(match update {
        Some(update) => UpdateInfo {
            available: true,
            version: Some(update.version.to_string()),
            body: update.body,
        },
        None => UpdateInfo {
            available: false,
            version: None,
            body: None,
        },
    })
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    crate::diagnostics::info("command: install_update");
    let Some(update) = app
        .updater()
        .map_err(|error| error.to_string())?
        .check()
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("当前没有可用更新".to_string());
    };

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| error.to_string())?;
    app.restart();
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

fn html_to_plain_text(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut in_tag = false;
    let mut entity = String::new();

    for character in value.chars() {
        match character {
            '<' => {
                in_tag = true;
                push_spacing(&mut output);
            }
            '>' => {
                in_tag = false;
            }
            '&' if !in_tag => {
                if entity.starts_with('&') {
                    output.push_str(&entity);
                }
                entity.clear();
                entity.push(character);
            }
            ';' if !in_tag && entity.starts_with('&') => {
                entity.push(character);
                output.push_str(match entity.as_str() {
                    "&amp;" => "&",
                    "&lt;" => "<",
                    "&gt;" => ">",
                    "&quot;" => "\"",
                    "&#39;" | "&apos;" => "'",
                    "&nbsp;" => " ",
                    _ => entity.as_str(),
                });
                entity.clear();
            }
            _ if in_tag => {}
            _ if entity.starts_with('&') => {
                entity.push(character);
            }
            _ => output.push(character),
        }
    }

    if entity.starts_with('&') {
        output.push_str(&entity);
    }
    output.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn push_spacing(output: &mut String) {
    if !output.chars().last().map(char::is_whitespace).unwrap_or(false) {
        output.push(' ');
    }
}

#[cfg(test)]
mod tests {
    use super::html_to_plain_text;

    #[test]
    fn html_to_plain_text_removes_tags_and_decodes_common_entities() {
        assert_eq!(
            html_to_plain_text("<p>Hello&nbsp;<strong>world</strong> &amp; clipboard</p>"),
            "Hello world & clipboard"
        );
    }

    #[test]
    fn html_to_plain_text_keeps_text_between_bare_ampersands() {
        assert_eq!(html_to_plain_text("A & B & C"), "A & B & C");
    }
}
