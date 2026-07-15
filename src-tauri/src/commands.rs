use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_updater::UpdaterExt;

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use crate::storage::repository::{ClipboardItem, ClipboardItemSummary, SearchFilters};
use crate::system::settings::AppSettings;
use crate::AppState;

fn copy_image_blob_with<T>(
    image_store: &crate::blobs::store::ImageBlobStore,
    content_path: &Path,
    read_dib: impl FnOnce(&Path) -> anyhow::Result<Vec<u8>>,
    write_clipboard: impl FnOnce(&[u8]) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    image_store.with_read(|blob_dir| {
        let blob_path = content_path
            .canonicalize()
            .map_err(|error| anyhow::anyhow!("invalid blob path: {error}"))?;
        anyhow::ensure!(
            blob_path.parent() == Some(blob_dir),
            "blob path outside allowed directory"
        );
        let dib = read_dib(&blob_path)?;
        write_clipboard(&dib)
    })
}

fn load_item_for_copy(
    repository: &Mutex<crate::storage::repository::ClipboardRepository>,
    id: &str,
) -> anyhow::Result<ClipboardItem> {
    let repository = repository
        .lock()
        .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?;
    repository
        .get_item(id)?
        .ok_or_else(|| anyhow::anyhow!("clipboard item not found"))
}

pub(crate) fn mutate_and_cleanup_blobs<T>(
    repository: &Mutex<crate::storage::repository::ClipboardRepository>,
    image_store: &crate::blobs::store::ImageBlobStore,
    mutate: impl FnOnce(&crate::storage::repository::ClipboardRepository) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    image_store.with_write(|blob_dir, _| {
        let value = {
            let repository = repository
                .lock()
                .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?;
            mutate(&repository)?
        };
        cleanup_pending_blobs(repository, blob_dir)?;
        Ok(value)
    })
}

fn delete_item_with(
    repository: &Mutex<crate::storage::repository::ClipboardRepository>,
    image_store: &crate::blobs::store::ImageBlobStore,
    id: &str,
) -> anyhow::Result<()> {
    mutate_and_cleanup_blobs(repository, image_store, |repository| {
        repository.soft_delete(id)
    })
}

fn clear_history_with(
    repository: &Mutex<crate::storage::repository::ClipboardRepository>,
    image_store: &crate::blobs::store::ImageBlobStore,
) -> anyhow::Result<()> {
    mutate_and_cleanup_blobs(repository, image_store, |repository| {
        repository.clear_history()
    })
}

pub(crate) fn prune_history_with(
    repository: &Mutex<crate::storage::repository::ClipboardRepository>,
    image_store: &crate::blobs::store::ImageBlobStore,
    max_history_items: i64,
    retention_days: i64,
) -> anyhow::Result<()> {
    mutate_and_cleanup_blobs(repository, image_store, |repository| {
        repository.prune_history(max_history_items, retention_days)
    })
}

pub(crate) fn cleanup_pending_blobs(
    repository: &Mutex<crate::storage::repository::ClipboardRepository>,
    blob_dir: &Path,
) -> anyhow::Result<()> {
    let (pending_paths, active_paths) = {
        let repository = repository
            .lock()
            .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?;
        (
            repository.pending_cleanup_paths()?,
            repository.active_blob_paths()?,
        )
    };
    let active_paths = active_paths
        .into_iter()
        .flat_map(|path| {
            let thumbnail = crate::blobs::thumbnail_path_for(&path);
            [path, thumbnail]
        })
        .collect::<HashSet<_>>();

    let mut first_error = None;
    for path in pending_paths {
        if active_paths.contains(&path) {
            continue;
        }
        let cleanup_result = (|| {
            anyhow::ensure!(
                path.is_absolute() && path.parent() == Some(blob_dir),
                "cleanup path outside allowed directory: {}",
                path.display()
            );
            match fs::symlink_metadata(&path) {
                Ok(metadata) => {
                    anyhow::ensure!(
                        metadata.file_type().is_file() || metadata.file_type().is_symlink(),
                        "cleanup path is not a file: {}",
                        path.display()
                    );
                    fs::remove_file(&path)?;
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            repository
                .lock()
                .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?
                .complete_cleanup_path(&path)?;
            Ok::<(), anyhow::Error>(())
        })();
        if let Err(error) = cleanup_result {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupMetadata {
    pub version: String,
    pub created_at: String,
    pub item_count: usize,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupData {
    pub metadata: BackupMetadata,
    pub items: Vec<ClipboardItem>,
    pub blobs: Vec<BlobData>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BlobData {
    pub item_id: String,
    pub filename: String,
    pub data_base64: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct BackupInfo {
    pub created_at: String,
    pub item_count: usize,
    pub version: String,
}

#[tauri::command]
pub fn search_items(
    state: State<'_, AppState>,
    query: String,
    filters: SearchFilters,
    limit: i64,
    cursor: Option<i64>,
) -> Result<Vec<ClipboardItemSummary>, String> {
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
pub fn get_item_detail(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<ClipboardItem>, String> {
    crate::diagnostics::info(format!("command: get_item_detail id={id}"));
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository.get_item(&id).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn copy_item(
    state: State<'_, AppState>,
    id: String,
    plain_text: Option<bool>,
) -> Result<(), String> {
    crate::diagnostics::info(format!("command: copy_item id={id}"));
    let item = load_item_for_copy(&state.repository, &id).map_err(|error| error.to_string())?;
    if item.item_type == "text" {
        if let Some(content) = item.content {
            #[cfg(target_os = "windows")]
            crate::clipboard::win::write_text_to_clipboard(&content)
                .map_err(|error| error.to_string())?;
            #[cfg(not(target_os = "windows"))]
            let _ = content;
        }
        return Ok(());
    }
    if item.item_type == "html" {
        if let Some(content) = item.content {
            // Preserve formatting by default (CF_HTML); write plain text only when
            // the caller explicitly asks for "paste as plain text".
            let plain = html_to_plain_text(&content);
            #[cfg(target_os = "windows")]
            {
                if plain_text.unwrap_or(false) {
                    crate::clipboard::win::write_text_to_clipboard(&plain)
                        .map_err(|error| error.to_string())?;
                } else {
                    crate::clipboard::win::write_html_to_clipboard(&content, &plain)
                        .map_err(|error| error.to_string())?;
                }
            }
            #[cfg(not(target_os = "windows"))]
            let _ = (content, plain, plain_text);
        }
        return Ok(());
    }
    if item.item_type == "image" {
        let content_path = item
            .content_path
            .ok_or_else(|| "image item has no blob path".to_string())?;

        #[cfg(target_os = "windows")]
        copy_image_blob_with(
            &state.image_store,
            Path::new(&content_path),
            crate::blobs::read_dib_from_bmp_file,
            |dib| crate::clipboard::win::write_dib_to_clipboard(dib),
        )
        .map_err(|error| error.to_string())?;
        #[cfg(not(target_os = "windows"))]
        copy_image_blob_with(
            &state.image_store,
            Path::new(&content_path),
            crate::blobs::read_dib_from_bmp_file,
            |_dib| Ok(()),
        )
        .map_err(|error| error.to_string())?;
        return Ok(());
    }
    if item.item_type == "files" {
        if let Some(content) = item.content {
            // Parse JSON array of file paths
            let file_paths: Vec<String> = serde_json::from_str(&content)
                .map_err(|e| format!("failed to parse file paths: {}", e))?;
            if !file_paths.is_empty() {
                #[cfg(target_os = "windows")]
                crate::clipboard::win::write_files_to_clipboard(&file_paths)
                    .map_err(|error| error.to_string())?;
                return Ok(());
            }
        }
    }

    let error = format!("copy is not implemented for {} items", item.item_type);
    crate::diagnostics::warn(format!("command: copy_item failed: {error}"));
    Err(error)
}

#[tauri::command]
pub fn paste_item(
    state: State<'_, AppState>,
    id: String,
    plain_text: Option<bool>,
) -> Result<(), String> {
    crate::diagnostics::info(format!("command: paste_item id={id}"));
    copy_item(state, id, plain_text)?;
    #[cfg(target_os = "windows")]
    crate::clipboard::win::simulate_paste_shortcut().map_err(|error| error.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn toggle_favorite(state: State<'_, AppState>, id: String) -> Result<(), String> {
    crate::diagnostics::info(format!("command: toggle_favorite id={id}"));
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository
        .toggle_favorite(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn toggle_pin(state: State<'_, AppState>, id: String) -> Result<(), String> {
    crate::diagnostics::info(format!("command: toggle_pin id={id}"));
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository
        .toggle_pin(&id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn delete_item(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<(), String> {
    crate::diagnostics::info(format!("command: delete_item id={id}"));
    delete_item_with(&state.repository, &state.image_store, &id)
        .map_err(|error| error.to_string())?;
    if let Err(error) = crate::storage::capacity::clear_capacity_status_if_recovered(
        &app,
        &state.capacity_status,
        &state.image_store,
    ) {
        crate::diagnostics::warn(format!(
            "command: failed to refresh capacity status after delete: {error}"
        ));
    }
    Ok(())
}

#[tauri::command]
pub fn reorder_items(state: State<'_, AppState>, ids: Vec<String>) -> Result<(), String> {
    crate::diagnostics::info(format!("command: reorder_items count={}", ids.len()));
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository
        .reorder_items(&ids)
        .map_err(|error| error.to_string())
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
pub fn get_clipboard_status(
    state: State<'_, AppState>,
) -> Result<crate::storage::capacity::ClipboardCapacityStatus, String> {
    crate::storage::capacity::current_capacity_status(&state.capacity_status)
        .map_err(|error| error.to_string())
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
        (
            settings.global_shortcut.clone(),
            settings.last_update_check_date.clone(),
        )
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
    prune_history_with(
        &state.repository,
        &state.image_store,
        next_settings.max_history_items,
        next_settings.retention_days,
    )
    .map_err(|error| error.to_string())?;
    Ok(next_settings)
}

#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<UpdateInfo, String> {
    crate::diagnostics::info("command: check_update");

    let current_version = app.package_info().version.to_string();
    crate::diagnostics::info(format!(
        "check_update: current version = {}",
        current_version
    ));

    let today = chrono::Local::now().date_naive().to_string();
    let update = app
        .updater()
        .map_err(|error| {
            let error_msg = error.to_string();
            crate::diagnostics::error(format!("更新检查失败: {}", error_msg));
            if error_msg.contains("error sending request") || error_msg.contains("timeout") {
                format!("检查更新失败: 网络连接超时。如果您在中国大陆，请检查网络连接或稍后重试。原始错误: {}", error_msg)
            } else {
                format!("检查更新失败: {}", error_msg)
            }
        })?
        .check()
        .await
        .map_err(|error| {
            let error_msg = error.to_string();
            crate::diagnostics::error(format!("更新检查失败: {}", error_msg));
            if error_msg.contains("error sending request") || error_msg.contains("timeout") {
                format!("检查更新失败: 网络连接超时。如果您在中国大陆，请检查网络连接或稍后重试。原始错误: {}", error_msg)
            } else {
                format!("检查更新失败: {}", error_msg)
            }
        })?;

    {
        let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
        settings.last_update_check_date = Some(today);
        settings
            .save(&state.settings_path)
            .map_err(|error| error.to_string())?;
    }

    Ok(match update {
        Some(update) => {
            crate::diagnostics::info(format!(
                "check_update: new version available = {}",
                update.version
            ));
            UpdateInfo {
                available: true,
                version: Some(update.version.to_string()),
                body: update.body,
            }
        }
        None => {
            crate::diagnostics::info("check_update: no update available");
            UpdateInfo {
                available: false,
                version: None,
                body: None,
            }
        }
    })
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    crate::diagnostics::info("command: install_update");
    let Some(update) = app
        .updater()
        .map_err(|error| {
            let error_msg = error.to_string();
            crate::diagnostics::error(format!("获取更新失败: {}", error_msg));
            if error_msg.contains("error sending request") || error_msg.contains("timeout") {
                format!("获取更新失败: 网络连接超时。如果您在中国大陆，请检查网络连接或稍后重试。原始错误: {}", error_msg)
            } else {
                format!("获取更新失败: {}", error_msg)
            }
        })?
        .check()
        .await
        .map_err(|error| {
            let error_msg = error.to_string();
            crate::diagnostics::error(format!("获取更新失败: {}", error_msg));
            if error_msg.contains("error sending request") || error_msg.contains("timeout") {
                format!("获取更新失败: 网络连接超时。如果您在中国大陆，请检查网络连接或稍后重试。原始错误: {}", error_msg)
            } else {
                format!("获取更新失败: {}", error_msg)
            }
        })?
    else {
        return Err("当前没有可用更新".to_string());
    };

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| {
            let error_msg = error.to_string();
            crate::diagnostics::error(format!("下载或安装更新失败: {}", error_msg));
            if error_msg.contains("error sending request") || error_msg.contains("timeout") {
                format!("下载更新失败: 网络连接超时。如果您在中国大陆，请检查网络连接或稍后重试。原始错误: {}", error_msg)
            } else {
                format!("安装更新失败: {}", error_msg)
            }
        })?;

    // Return success and let the app restart after a short delay
    // This allows the frontend to show feedback before restart
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(2));
        app.restart();
    });

    Ok(())
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
pub fn clear_history(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    crate::diagnostics::warn("command: clear_history");
    clear_history_with(&state.repository, &state.image_store).map_err(|error| error.to_string())?;
    if let Err(error) = crate::storage::capacity::clear_capacity_status_if_recovered(
        &app,
        &state.capacity_status,
        &state.image_store,
    ) {
        crate::diagnostics::warn(format!(
            "command: failed to refresh capacity status after clear: {error}"
        ));
    }
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
    if !output
        .chars()
        .last()
        .map(char::is_whitespace)
        .unwrap_or(false)
    {
        output.push(' ');
    }
}

fn validate_migration_paths(old_dir: &Path, new_dir: &Path) -> Result<(), String> {
    let old_dir = old_dir
        .canonicalize()
        .map_err(|e| format!("解析源目录失败: {}", e))?;

    // Security: Create and validate new directory atomically to prevent TOCTOU attacks
    // If directory doesn't exist, create it with safe permissions
    if !new_dir.exists() {
        std::fs::create_dir_all(new_dir).map_err(|e| format!("创建新目录失败: {}", e))?;
    }

    // Immediately canonicalize after creation to prevent symlink swap attacks
    let new_dir = new_dir
        .canonicalize()
        .map_err(|e| format!("解析新目录失败: {}", e))?;

    // Verify it's a real directory, not a symlink
    let metadata =
        std::fs::symlink_metadata(&new_dir).map_err(|e| format!("读取新目录元数据失败: {}", e))?;
    if metadata.is_symlink() {
        return Err("新目录不能是符号链接".to_string());
    }

    if old_dir == new_dir {
        return Err("新目录不能与源目录相同".to_string());
    }
    if new_dir.starts_with(&old_dir) {
        return Err("新目录不能位于源目录内部".to_string());
    }
    if old_dir.starts_with(&new_dir) {
        return Err("源目录不能位于新目录内部".to_string());
    }
    Ok(())
}

fn safe_backup_blob_filename(filename: &str) -> Result<String, String> {
    let path = Path::new(filename);
    if path.is_absolute() {
        return Err("blob 文件名不能是绝对路径".to_string());
    }
    let mut components = path.components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(name)), None) => {
            let name = name
                .to_str()
                .ok_or_else(|| "blob 文件名必须是有效文本".to_string())?;
            if name.is_empty() {
                return Err("blob 文件名不能为空".to_string());
            }
            if name.ends_with('.') || name.ends_with(' ') {
                return Err("blob 文件名不能以空格或点结尾".to_string());
            }
            Ok(name.to_string())
        }
        _ => Err("blob 文件名不能包含路径分隔符或上级目录".to_string()),
    }
}

// 存储路径管理命令

#[tauri::command]
pub async fn select_directory() -> Result<Option<String>, String> {
    use rfd::FileDialog;

    let selected = FileDialog::new().set_title("选择目录").pick_folder();

    Ok(selected.map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn migrate_directory(
    old_path: String,
    new_path: String,
    move_files: bool,
) -> Result<(), String> {
    use std::fs;
    let old_dir = PathBuf::from(&old_path);
    let new_dir = PathBuf::from(&new_path);

    crate::diagnostics::info(format!(
        "migrate_directory: old={} new={} move={}",
        old_path, new_path, move_files
    ));

    // 验证路径
    if !old_dir.exists() {
        return Err(format!("源目录不存在: {}", old_path));
    }
    validate_migration_paths(&old_dir, &new_dir)?;

    // 创建新目录
    fs::create_dir_all(&new_dir).map_err(|e| format!("创建新目录失败: {}", e))?;

    // 检查写权限
    let test_file = new_dir.join(".test_write");
    if let Err(e) = fs::write(&test_file, b"test") {
        return Err(format!("新目录无写入权限: {}", e));
    }
    let _ = fs::remove_file(&test_file);

    if move_files {
        // 迁移文件
        for entry in fs::read_dir(&old_dir).map_err(|e| format!("读取源目录失败: {}", e))? {
            let entry = entry.map_err(|e| format!("读取目录项失败: {}", e))?;
            let file_name = entry.file_name();
            let old_file = old_dir.join(&file_name);
            let new_file = new_dir.join(&file_name);

            // 跳过 .test_write 文件
            if file_name == ".test_write" {
                continue;
            }

            // Security: Check if entry is a symlink before processing
            let metadata = fs::symlink_metadata(&old_file)
                .map_err(|e| format!("读取文件元数据失败: {}", e))?;
            if metadata.is_symlink() {
                crate::diagnostics::warn(format!(
                    "migrate_directory: skipping symlink: {}",
                    file_name.to_string_lossy()
                ));
                continue;
            }

            if metadata.is_file() {
                fs::copy(&old_file, &new_file)
                    .map_err(|e| format!("复制文件 {} 失败: {}", file_name.to_string_lossy(), e))?;
                crate::diagnostics::info(format!("migrated: {}", file_name.to_string_lossy()));
            } else if metadata.is_dir() {
                // 递归复制目录
                copy_dir_all(&old_file, &new_file)
                    .map_err(|e| format!("复制目录 {} 失败: {}", file_name.to_string_lossy(), e))?;
            }
        }

        crate::diagnostics::info("migrate_directory: files copied successfully");
    }

    Ok(())
}

fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    use std::fs;

    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn update_storage_settings(
    state: State<'_, AppState>,
    custom_data_dir: Option<String>,
    custom_log_dir: Option<String>,
) -> Result<AppSettings, String> {
    crate::diagnostics::info(format!(
        "command: update_storage_settings data_dir={:?} log_dir={:?}",
        custom_data_dir, custom_log_dir
    ));

    let next_settings = {
        let mut settings = state.settings.lock().map_err(|error| error.to_string())?;
        settings.custom_data_dir = custom_data_dir;
        settings.custom_log_dir = custom_log_dir;
        settings.clone()
    };

    next_settings
        .save(&state.settings_path)
        .map_err(|error| error.to_string())?;

    Ok(next_settings)
}

// 导入/导出备份功能

#[tauri::command]
pub async fn export_backup(state: State<'_, AppState>) -> Result<String, String> {
    use chrono::Utc;
    use rfd::FileDialog;

    let default_filename = format!(
        "super-clipboard-backup-{}.zip",
        Utc::now().format("%Y%m%d-%H%M%S")
    );

    let save_path = FileDialog::new()
        .set_title("导出备份")
        .set_file_name(&default_filename)
        .add_filter("ZIP 备份", &["zip"])
        .save_file();

    let Some(save_path) = save_path else {
        return Err("用户取消导出".to_string());
    };

    crate::diagnostics::info(format!("export_backup: path={}", save_path.display()));
    crate::backup::export_zip_to(&save_path, &state.repository, &state.image_store)
        .map_err(|error| format!("导出备份失败: {error:#}"))?;
    crate::diagnostics::info("export_backup: streaming ZIP export succeeded");

    Ok(save_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn select_backup_file() -> Result<Option<String>, String> {
    use rfd::FileDialog;

    let selected = FileDialog::new()
        .set_title("选择备份文件")
        .add_filter("备份文件", &["zip", "json"])
        .add_filter("ZIP 备份", &["zip"])
        .add_filter("旧版 JSON 备份", &["json"])
        .pick_file();

    Ok(selected.map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn parse_backup_info(backup_path: String) -> Result<BackupInfo, String> {
    crate::backup::parse_backup_info_path(Path::new(&backup_path))
        .map_err(|error| format!("解析备份文件失败: {error:#}"))
}

fn ensure_legacy_json_import(path: &Path) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|error| format!("读取备份文件失败: {error}"))?;
    let mut magic = [0u8; 4];
    let count = file
        .read(&mut magic)
        .map_err(|error| format!("读取备份文件失败: {error}"))?;
    if count == magic.len() && crate::backup::is_zip_magic(magic) {
        return Err("ZIP import requires transactional importer".to_string());
    }
    Ok(())
}

fn import_backup_data_with(
    repository: &Mutex<crate::storage::repository::ClipboardRepository>,
    image_store: &crate::blobs::store::ImageBlobStore,
    backup: BackupData,
    merge: bool,
) -> Result<usize, String> {
    image_store
        .with_write(|blob_dir, stage_root| {
            let items = if merge {
                let repository = repository
                    .lock()
                    .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?;
                backup
                    .items
                    .into_iter()
                    .filter(|item| {
                        !repository
                            .find_by_hash(&item.hash)
                            .is_ok_and(|existing| existing.is_some())
                    })
                    .collect::<Vec<_>>()
            } else {
                backup.items
            };
            let referenced_blob_names = items
                .iter()
                .filter_map(|item| item.content_path.clone())
                .collect::<HashSet<_>>();

            struct PreparedBlob {
                target_path: PathBuf,
                data: Vec<u8>,
                original_names: Vec<String>,
                should_write: bool,
                staged_path: Option<PathBuf>,
            }

            let mut prepared_blobs: HashMap<String, PreparedBlob> = HashMap::new();
            for blob in &backup.blobs {
                if merge && !referenced_blob_names.contains(&blob.filename) {
                    continue;
                }
                let filename = safe_backup_blob_filename(&blob.filename)
                    .map_err(|error| anyhow::anyhow!(error))?;
                let target_identity = filename.to_ascii_lowercase();
                let blob_path = blob_dir.join(&filename);
                let data = base64_decode(&blob.data_base64).map_err(|error| {
                    anyhow::anyhow!("解码 blob {} 失败: {error}", blob.filename)
                })?;
                if let Some(existing) = prepared_blobs.get_mut(&target_identity) {
                    anyhow::ensure!(
                        existing.data == data,
                        "backup blob target conflict: {}",
                        blob.filename
                    );
                    existing.original_names.push(blob.filename.clone());
                    continue;
                }
                let should_write = match fs::read(&blob_path) {
                    Ok(existing_data) if merge => {
                        anyhow::ensure!(
                            existing_data == data,
                            "merge blob conflict: {}",
                            blob.filename
                        );
                        false
                    }
                    Ok(_) => true,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                    Err(error) => return Err(error.into()),
                };
                prepared_blobs.insert(
                    target_identity,
                    PreparedBlob {
                        target_path: blob_path,
                        data,
                        original_names: vec![blob.filename.clone()],
                        should_write,
                        staged_path: None,
                    },
                );
            }

            let stage_dir = if prepared_blobs.values().any(|blob| blob.should_write) {
                let stage_dir = stage_root.join(format!("import-{}", uuid::Uuid::new_v4()));
                fs::create_dir(&stage_dir)?;
                let stage_result = (|| {
                    for (index, blob) in prepared_blobs.values_mut().enumerate() {
                        if !blob.should_write {
                            continue;
                        }
                        let staged_path = stage_dir.join(format!("{index}.blob"));
                        let mut staged_file = fs::File::create(&staged_path)?;
                        staged_file.write_all(&blob.data)?;
                        staged_file.flush()?;
                        staged_file.sync_all()?;
                        blob.staged_path = Some(staged_path);
                    }
                    Ok::<(), anyhow::Error>(())
                })();
                if let Err(error) = stage_result {
                    let _ = fs::remove_dir_all(&stage_dir);
                    return Err(error);
                }
                Some(stage_dir)
            } else {
                None
            };

            if !merge {
                repository
                    .lock()
                    .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?
                    .clear_history()
                    .map_err(|error| anyhow::anyhow!("清空历史失败: {error}"))?;
                cleanup_pending_blobs(repository, blob_dir)
                    .map_err(|error| anyhow::anyhow!("清空 blob 目录失败: {error}"))?;
            }

            let mut restored_blob_map = HashMap::new();
            let install_result = (|| {
                for blob in prepared_blobs.values_mut() {
                    if let Some(staged_path) = blob.staged_path.take() {
                        if blob.target_path.exists() {
                            fs::remove_file(&blob.target_path)?;
                        }
                        fs::rename(staged_path, &blob.target_path)?;
                    }
                    for original_name in &blob.original_names {
                        restored_blob_map.insert(
                            original_name.clone(),
                            blob.target_path.to_string_lossy().to_string(),
                        );
                    }
                }
                Ok::<(), anyhow::Error>(())
            })();
            if let Some(stage_dir) = &stage_dir {
                let _ = fs::remove_dir_all(stage_dir);
            }
            install_result?;

            let mut imported_count = 0;
            for mut item in items {
                if let Some(original_filename) = &item.content_path {
                    item.content_path = restored_blob_map.get(original_filename).cloned();
                }
                if merge {
                    let duplicate = repository
                        .lock()
                        .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?
                        .find_by_hash(&item.hash)
                        .is_ok_and(|existing| existing.is_some());
                    if duplicate {
                        continue;
                    }
                }
                repository
                    .lock()
                    .map_err(|error| anyhow::anyhow!("repository lock poisoned: {error}"))?
                    .insert_imported_item(&item)
                    .map_err(|error| anyhow::anyhow!("导入记录失败: {error}"))?;
                imported_count += 1;
            }
            Ok(imported_count)
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn import_backup(
    state: State<'_, AppState>,
    backup_path: String,
    merge: bool,
) -> Result<usize, String> {
    use chrono::Utc;
    use std::fs;

    crate::diagnostics::info(format!(
        "import_backup: path={} merge={}",
        backup_path, merge
    ));

    ensure_legacy_json_import(Path::new(&backup_path))?;

    // 读取备份文件
    let content =
        fs::read_to_string(&backup_path).map_err(|e| format!("读取备份文件失败: {}", e))?;

    let backup: BackupData =
        serde_json::from_str(&content).map_err(|e| format!("解析备份文件失败: {}", e))?;

    // 创建临时备份（防止误操作）
    if !merge {
        let temp_backup_path = state.app_data_dir.join(format!(
            "temp-backup-before-import-{}.json",
            Utc::now().format("%Y%m%d-%H%M%S")
        ));

        let repository = state.repository.lock().map_err(|e| e.to_string())?;
        let current_items = repository
            .list_items_for_backup(100000)
            .map_err(|e| format!("创建临时备份失败: {}", e))?;
        drop(repository);

        let temp_backup = BackupData {
            metadata: BackupMetadata {
                version: "1.0".to_string(),
                created_at: Utc::now().to_rfc3339(),
                item_count: current_items.len(),
            },
            items: current_items,
            blobs: vec![],
        };

        let temp_json = serde_json::to_string_pretty(&temp_backup)
            .map_err(|e| format!("序列化临时备份失败: {}", e))?;

        fs::write(&temp_backup_path, temp_json).map_err(|e| format!("写入临时备份失败: {}", e))?;

        crate::diagnostics::info(format!(
            "import_backup: temp backup created at {}",
            temp_backup_path.display()
        ));
    }

    let imported_count =
        import_backup_data_with(&state.repository, &state.image_store, backup, merge)?;
    if !merge {
        crate::diagnostics::info("import_backup: existing data cleared");
    }

    crate::diagnostics::info(format!(
        "import_backup: success, imported {} items",
        imported_count
    ));

    Ok(imported_count)
}

#[cfg(test)]
fn base64_encode(data: &[u8]) -> String {
    use base64::{engine::general_purpose, Engine as _};
    general_purpose::STANDARD.encode(data)
}

fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    use base64::{engine::general_purpose, Engine as _};
    general_purpose::STANDARD
        .decode(s)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use super::{
        clear_history_with, copy_image_blob_with, delete_item_with, ensure_legacy_json_import,
        html_to_plain_text, import_backup_data_with, load_item_for_copy, mutate_and_cleanup_blobs,
        prune_history_with, safe_backup_blob_filename, validate_migration_paths, BackupData,
        BackupMetadata, BlobData,
    };
    use crate::blobs::image::stage_dib;
    use crate::blobs::store::ImageBlobStore;
    use crate::clipboard::types::{ClipboardItemDraft, ClipboardItemType};
    use crate::storage::repository::{ClipboardItem, ClipboardRepository};

    fn dib32() -> Vec<u8> {
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1i32.to_le_bytes());
        dib[8..12].copy_from_slice(&(-1i32).to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&32u16.to_le_bytes());
        dib[20..24].copy_from_slice(&4u32.to_le_bytes());
        dib.extend_from_slice(&[30, 20, 10, 255]);
        dib
    }

    #[test]
    fn zip_import_requires_transactional_importer() {
        let path = std::env::temp_dir().join(format!(
            "super-clipboard-zip-import-{}.zip",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, b"PK\x03\x04not a complete archive").expect("write ZIP fixture");

        let error = ensure_legacy_json_import(&path).expect_err("ZIP import must be rejected");

        assert_eq!(error, "ZIP import requires transactional importer");
        std::fs::remove_file(path).expect("cleanup");
    }

    fn imported_image_item(id: &str, hash: &str, filename: &str) -> ClipboardItem {
        ClipboardItem {
            id: id.to_string(),
            hash: hash.to_string(),
            item_type: "image".to_string(),
            content: None,
            content_path: Some(filename.to_string()),
            content_hash: Some(format!("content-{hash}")),
            preview: filename.to_string(),
            source_app: None,
            favorite: false,
            pinned: false,
            size_bytes: 4,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn assert_overwrite_preflight_preserves_existing(label: &str, backup: BackupData) {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-overwrite-preflight-{label}-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let staged = stage_dib(store.stage_dir(), dib32()).expect("stage old image");
        let installed = store
            .install_staged_with(staged, |installed| Ok(installed.clone()), |_| Ok(false))
            .expect("install old image");
        let original_bmp = std::fs::read(installed.bmp_path()).expect("old bmp");
        let original_thumbnail = std::fs::read(installed.thumbnail_path()).expect("old thumbnail");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let old_item = repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(ClipboardItemDraft {
                item_type: ClipboardItemType::Image,
                content: None,
                content_path: Some(installed.bmp_path().to_string_lossy().to_string()),
                content_hash: Some(installed.content_hash().to_string()),
                preview: "old image".to_string(),
                source_app: None,
                size_bytes: i64::try_from(original_bmp.len()).expect("bmp size"),
            })
            .expect("insert old image");

        import_backup_data_with(&repository, &store, backup, false)
            .expect_err("overwrite preflight must fail");

        assert_eq!(
            std::fs::read(installed.bmp_path()).expect("old bmp after failure"),
            original_bmp
        );
        assert_eq!(
            std::fs::read(installed.thumbnail_path()).expect("old thumbnail after failure"),
            original_thumbnail
        );
        let repository_guard = repository.lock().expect("repository lock");
        assert!(repository_guard
            .get_item(&old_item.id)
            .expect("old item")
            .is_some());
        assert!(repository_guard
            .pending_cleanup_paths()
            .expect("cleanup queue")
            .is_empty());
        drop(repository_guard);
        assert!(std::fs::read_dir(store.stage_dir())
            .expect("stage directory")
            .next()
            .is_none());
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn overwrite_invalid_base64_preserves_existing_history_and_blobs() {
        assert_overwrite_preflight_preserves_existing(
            "base64",
            BackupData {
                metadata: BackupMetadata {
                    version: "1.0".to_string(),
                    created_at: "test".to_string(),
                    item_count: 0,
                },
                items: Vec::new(),
                blobs: vec![BlobData {
                    item_id: "invalid".to_string(),
                    filename: "invalid.bmp".to_string(),
                    data_base64: "%%%not-base64%%%".to_string(),
                }],
            },
        );
    }

    #[test]
    fn overwrite_escaping_filename_preserves_existing_history_and_blobs() {
        assert_overwrite_preflight_preserves_existing(
            "filename",
            BackupData {
                metadata: BackupMetadata {
                    version: "1.0".to_string(),
                    created_at: "test".to_string(),
                    item_count: 0,
                },
                items: Vec::new(),
                blobs: vec![BlobData {
                    item_id: "escape".to_string(),
                    filename: "../escape.bmp".to_string(),
                    data_base64: super::base64_encode(b"escape"),
                }],
            },
        );
    }

    #[test]
    fn overwrite_windows_alias_conflict_preserves_existing_history_and_blobs() {
        assert_overwrite_preflight_preserves_existing(
            "alias",
            BackupData {
                metadata: BackupMetadata {
                    version: "1.0".to_string(),
                    created_at: "test".to_string(),
                    item_count: 0,
                },
                items: Vec::new(),
                blobs: vec![
                    BlobData {
                        item_id: "lower".to_string(),
                        filename: "image.bmp".to_string(),
                        data_base64: super::base64_encode(b"lower"),
                    },
                    BlobData {
                        item_id: "upper".to_string(),
                        filename: "IMAGE.bmp".to_string(),
                        data_base64: super::base64_encode(b"upper"),
                    },
                ],
            },
        );
    }

    #[test]
    fn dib_from_bmp_image_copy_releases_repository_before_io_and_holds_read_lease() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-copy-image-{}",
            uuid::Uuid::new_v4()
        ));
        let store =
            Arc::new(ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store"));
        let staged = stage_dib(store.stage_dir(), dib32()).expect("stage");
        let image_path = store
            .install_staged_with(
                staged,
                |installed| Ok(installed.bmp_path().to_path_buf()),
                |_| Ok(false),
            )
            .expect("install");
        let repository = Arc::new(Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        ));
        let item = repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(ClipboardItemDraft {
                item_type: ClipboardItemType::Image,
                content: None,
                content_path: Some(image_path.to_string_lossy().to_string()),
                content_hash: Some("copy-image-content".to_string()),
                preview: "image".to_string(),
                source_app: None,
                size_bytes: 44,
            })
            .expect("insert image");
        let (read_started_tx, read_started_rx) = mpsc::channel();
        let (release_read_tx, release_read_rx) = mpsc::channel();
        let (write_started_tx, write_started_rx) = mpsc::channel();
        let (release_copy_tx, release_copy_rx) = mpsc::channel();
        let (lease_entered_tx, lease_entered_rx) = mpsc::channel();

        let reader_store = Arc::clone(&store);
        let reader_repository = Arc::clone(&repository);
        let reader = thread::spawn(move || {
            let item = load_item_for_copy(&reader_repository, &item.id)?;
            let image_path = item
                .content_path
                .ok_or_else(|| anyhow::anyhow!("image item has no blob path"))?;
            copy_image_blob_with(
                &reader_store,
                Path::new(&image_path),
                |path| {
                    let repository_available = reader_repository.try_lock().is_ok();
                    read_started_tx
                        .send(repository_available)
                        .expect("read started");
                    release_read_rx.recv().expect("release read");
                    crate::blobs::read_dib_from_bmp_file(path)
                },
                |_dib| {
                    write_started_tx.send(()).expect("write started");
                    release_copy_rx.recv().expect("release copy");
                    Ok(())
                },
            )
        });
        assert!(read_started_rx.recv().expect("image read started"));
        let writer_store = Arc::clone(&store);
        let writer = thread::spawn(move || {
            writer_store.with_write(|_, _| {
                lease_entered_tx.send(()).expect("write lease entered");
                Ok(())
            })
        });

        assert!(lease_entered_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        release_read_tx.send(()).expect("release read");
        write_started_rx.recv().expect("copy callback entered");
        assert!(lease_entered_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        release_copy_tx.send(()).expect("release copy");
        lease_entered_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("writer entered after copy");
        reader.join().expect("reader thread").expect("copy result");
        writer
            .join()
            .expect("writer thread")
            .expect("writer result");
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn image_delete_waits_for_read_lease_then_removes_files_and_queue_row() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-delete-image-{}",
            uuid::Uuid::new_v4()
        ));
        let store =
            Arc::new(ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store"));
        let staged = stage_dib(store.stage_dir(), dib32()).expect("stage");
        let installed = store
            .install_staged_with(staged, |installed| Ok(installed.clone()), |_| Ok(false))
            .expect("install");
        let image_path = installed.bmp_path().to_path_buf();
        let thumbnail_path = installed.thumbnail_path().to_path_buf();
        let repository = Arc::new(Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        ));
        let item = repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(ClipboardItemDraft {
                item_type: ClipboardItemType::Image,
                content: None,
                content_path: Some(image_path.to_string_lossy().to_string()),
                content_hash: Some(installed.content_hash().to_string()),
                preview: "image".to_string(),
                source_app: None,
                size_bytes: 44,
            })
            .expect("insert image");
        let (read_started_tx, read_started_rx) = mpsc::channel();
        let (release_read_tx, release_read_rx) = mpsc::channel();
        let reader_store = Arc::clone(&store);
        let reader = thread::spawn(move || {
            reader_store.with_read(|_| {
                read_started_tx.send(()).expect("read started");
                release_read_rx.recv().expect("release read");
                Ok(())
            })
        });
        read_started_rx.recv().expect("reader entered");

        let worker_store = Arc::clone(&store);
        let worker_repository = Arc::clone(&repository);
        let item_id = item.id.clone();
        let (delete_finished_tx, delete_finished_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = delete_item_with(&worker_repository, &worker_store, &item_id);
            delete_finished_tx.send(()).expect("delete finished");
            result
        });

        assert!(delete_finished_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        assert!(repository
            .lock()
            .expect("repository lock")
            .get_item(&item.id)
            .expect("item before release")
            .is_some());
        assert!(image_path.exists());
        assert!(thumbnail_path.exists());

        release_read_tx.send(()).expect("release reader");
        delete_finished_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("delete after read");
        reader.join().expect("reader thread").expect("read result");
        worker
            .join()
            .expect("delete thread")
            .expect("delete result");

        assert!(!image_path.exists());
        assert!(!thumbnail_path.exists());
        assert!(repository
            .lock()
            .expect("repository lock")
            .pending_cleanup_paths()
            .expect("cleanup queue")
            .is_empty());
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn image_cleanup_keeps_files_and_queue_while_path_is_still_active() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-shared-image-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let staged = stage_dib(store.stage_dir(), dib32()).expect("stage");
        let installed = store
            .install_staged_with(staged, |installed| Ok(installed.clone()), |_| Ok(false))
            .expect("install");
        let image_path = installed.bmp_path().to_path_buf();
        let thumbnail_path = installed.thumbnail_path().to_path_buf();
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let image_draft = |content_hash: &str| ClipboardItemDraft {
            item_type: ClipboardItemType::Image,
            content: None,
            content_path: Some(image_path.to_string_lossy().to_string()),
            content_hash: Some(content_hash.to_string()),
            preview: "image".to_string(),
            source_app: None,
            size_bytes: 44,
        };
        let deleted = repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(image_draft(installed.content_hash()))
            .expect("insert deleted image");
        repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(image_draft("legacy-second-reference"))
            .expect("insert shared image");

        delete_item_with(&repository, &store, &deleted.id).expect("delete one reference");

        assert!(image_path.exists());
        assert!(thumbnail_path.exists());
        assert_eq!(
            repository
                .lock()
                .expect("repository lock")
                .pending_cleanup_paths()
                .expect("cleanup queue"),
            vec![image_path.clone(), thumbnail_path.clone()]
        );
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn image_cleanup_completes_queue_when_files_are_already_absent() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-absent-image-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let image_path = store.blob_dir().join("missing.bmp");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let item = repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(ClipboardItemDraft {
                item_type: ClipboardItemType::Image,
                content: None,
                content_path: Some(image_path.to_string_lossy().to_string()),
                content_hash: Some("missing-image".to_string()),
                preview: "image".to_string(),
                source_app: None,
                size_bytes: 44,
            })
            .expect("insert image");

        delete_item_with(&repository, &store, &item.id).expect("delete absent image");

        assert!(repository
            .lock()
            .expect("repository lock")
            .pending_cleanup_paths()
            .expect("cleanup queue")
            .is_empty());
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn image_cleanup_continues_after_invalid_queue_path() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-cleanup-poison-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let outside_path = root.join("outside.bmp");
        std::fs::write(&outside_path, b"outside").expect("outside file");
        repository
            .lock()
            .expect("repository lock")
            .update_image_references_and_enqueue_cleanup(&[], &[outside_path.clone()])
            .expect("enqueue invalid path");
        thread::sleep(Duration::from_millis(2));
        let valid_path = store.blob_dir().join("valid.bmp");
        let valid_thumbnail = crate::blobs::thumbnail_path_for(&valid_path);
        std::fs::write(&valid_path, b"valid").expect("valid file");
        std::fs::write(&valid_thumbnail, b"thumbnail").expect("valid thumbnail");
        repository
            .lock()
            .expect("repository lock")
            .update_image_references_and_enqueue_cleanup(&[], &[valid_path.clone()])
            .expect("enqueue valid path");

        mutate_and_cleanup_blobs(&repository, &store, |_| Ok(()))
            .expect_err("invalid cleanup path must be reported");

        assert!(outside_path.exists());
        assert!(!valid_path.exists());
        assert!(!valid_thumbnail.exists());
        assert_eq!(
            repository
                .lock()
                .expect("repository lock")
                .pending_cleanup_paths()
                .expect("remaining queue"),
            vec![
                outside_path.clone(),
                crate::blobs::thumbnail_path_for(&outside_path),
            ]
        );
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn clear_history_cleans_queued_image_paths_under_blob_lease() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-clear-image-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let image_path = store.blob_dir().join("missing.bmp");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let item = repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(ClipboardItemDraft {
                item_type: ClipboardItemType::Image,
                content: None,
                content_path: Some(image_path.to_string_lossy().to_string()),
                content_hash: Some("clear-image".to_string()),
                preview: "image".to_string(),
                source_app: None,
                size_bytes: 44,
            })
            .expect("insert image");

        clear_history_with(&repository, &store).expect("clear history");

        {
            let repository = repository.lock().expect("repository lock");
            assert!(repository
                .get_item(&item.id)
                .expect("cleared item")
                .is_none());
            assert!(repository
                .pending_cleanup_paths()
                .expect("cleanup queue")
                .is_empty());
        }
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn prune_history_cleans_queued_image_paths_under_blob_lease() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-prune-image-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let image_path = store.blob_dir().join("missing.bmp");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let image = repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(ClipboardItemDraft {
                item_type: ClipboardItemType::Image,
                content: None,
                content_path: Some(image_path.to_string_lossy().to_string()),
                content_hash: Some("prune-image".to_string()),
                preview: "image".to_string(),
                source_app: None,
                size_bytes: 44,
            })
            .expect("insert image");
        repository
            .lock()
            .expect("repository lock")
            .insert_or_touch(ClipboardItemDraft {
                item_type: ClipboardItemType::Text,
                content: Some("keep".to_string()),
                content_path: None,
                content_hash: None,
                preview: "keep".to_string(),
                source_app: None,
                size_bytes: 4,
            })
            .expect("insert recent text");

        prune_history_with(&repository, &store, 1, 0).expect("prune history");

        {
            let repository = repository.lock().expect("repository lock");
            assert!(repository
                .get_item(&image.id)
                .expect("pruned image")
                .is_none());
            assert!(repository
                .pending_cleanup_paths()
                .expect("cleanup queue")
                .is_empty());
        }
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn overwrite_import_waits_for_read_lease_then_cleans_and_restores() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-overwrite-import-{}",
            uuid::Uuid::new_v4()
        ));
        let store =
            Arc::new(ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store"));
        let staged = stage_dib(store.stage_dir(), dib32()).expect("stage old image");
        let installed = store
            .install_staged_with(staged, |installed| Ok(installed.clone()), |_| Ok(false))
            .expect("install old image");
        let old_image_path = installed.bmp_path().to_path_buf();
        let old_thumbnail_path = installed.thumbnail_path().to_path_buf();
        let repository = Arc::new(Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        ));
        let old_item = repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(ClipboardItemDraft {
                item_type: ClipboardItemType::Image,
                content: None,
                content_path: Some(old_image_path.to_string_lossy().to_string()),
                content_hash: Some(installed.content_hash().to_string()),
                preview: "old image".to_string(),
                source_app: None,
                size_bytes: 44,
            })
            .expect("insert old image");
        let restored_id = uuid::Uuid::new_v4().to_string();
        let backup = BackupData {
            metadata: BackupMetadata {
                version: "1.0".to_string(),
                created_at: "test".to_string(),
                item_count: 1,
            },
            items: vec![crate::storage::repository::ClipboardItem {
                id: restored_id.clone(),
                hash: "restored-hash".to_string(),
                item_type: "image".to_string(),
                content: None,
                content_path: Some("restored.bmp".to_string()),
                content_hash: Some("restored-content-hash".to_string()),
                preview: "restored image".to_string(),
                source_app: None,
                favorite: false,
                pinned: false,
                size_bytes: 8,
                created_at: 1,
                updated_at: 1,
            }],
            blobs: vec![BlobData {
                item_id: restored_id.clone(),
                filename: "restored.bmp".to_string(),
                data_base64: super::base64_encode(b"restored"),
            }],
        };
        let (read_started_tx, read_started_rx) = mpsc::channel();
        let (release_read_tx, release_read_rx) = mpsc::channel();
        let reader_store = Arc::clone(&store);
        let reader = thread::spawn(move || {
            reader_store.with_read(|_| {
                read_started_tx.send(()).expect("read started");
                release_read_rx.recv().expect("release read");
                Ok(())
            })
        });
        read_started_rx.recv().expect("reader entered");

        let worker_store = Arc::clone(&store);
        let worker_repository = Arc::clone(&repository);
        let (import_finished_tx, import_finished_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let result = import_backup_data_with(&worker_repository, &worker_store, backup, false);
            import_finished_tx.send(()).expect("import finished");
            result
        });

        assert!(import_finished_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err());
        assert!(repository
            .lock()
            .expect("repository lock")
            .get_item(&old_item.id)
            .expect("old item before release")
            .is_some());
        assert!(old_image_path.exists());
        assert!(old_thumbnail_path.exists());

        release_read_tx.send(()).expect("release reader");
        import_finished_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("import after read");
        reader.join().expect("reader thread").expect("read result");
        assert_eq!(
            worker
                .join()
                .expect("import thread")
                .expect("import result"),
            1
        );

        assert!(!old_image_path.exists());
        assert!(!old_thumbnail_path.exists());
        assert_eq!(
            std::fs::read(store.blob_dir().join("restored.bmp")).expect("restored blob"),
            b"restored"
        );
        let repository_guard = repository.lock().expect("repository lock");
        assert!(repository_guard
            .get_item(&old_item.id)
            .expect("old item after import")
            .is_none());
        assert!(repository_guard
            .get_item(&restored_id)
            .expect("restored item")
            .is_some());
        assert!(repository_guard
            .pending_cleanup_paths()
            .expect("cleanup queue")
            .is_empty());
        drop(repository_guard);
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn merge_duplicate_item_does_not_overwrite_existing_blob() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-merge-import-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let staged = stage_dib(store.stage_dir(), dib32()).expect("stage old image");
        let installed = store
            .install_staged_with(staged, |installed| Ok(installed.clone()), |_| Ok(false))
            .expect("install old image");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let old_item = repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(ClipboardItemDraft {
                item_type: ClipboardItemType::Image,
                content: None,
                content_path: Some(installed.bmp_path().to_string_lossy().to_string()),
                content_hash: Some(installed.content_hash().to_string()),
                preview: "old image".to_string(),
                source_app: None,
                size_bytes: 44,
            })
            .expect("insert old image");
        let original_bytes = std::fs::read(installed.bmp_path()).expect("existing blob");
        let filename = installed
            .bmp_path()
            .file_name()
            .and_then(|value| value.to_str())
            .expect("blob filename")
            .to_string();
        let backup = BackupData {
            metadata: BackupMetadata {
                version: "1.0".to_string(),
                created_at: "test".to_string(),
                item_count: 1,
            },
            items: vec![crate::storage::repository::ClipboardItem {
                id: uuid::Uuid::new_v4().to_string(),
                hash: old_item.hash.clone(),
                item_type: "image".to_string(),
                content: None,
                content_path: Some(filename.clone()),
                content_hash: Some("duplicate-content-hash".to_string()),
                preview: "duplicate image".to_string(),
                source_app: None,
                favorite: false,
                pinned: false,
                size_bytes: 9,
                created_at: 1,
                updated_at: 1,
            }],
            blobs: vec![BlobData {
                item_id: "duplicate".to_string(),
                filename,
                data_base64: super::base64_encode(b"different"),
            }],
        };

        assert_eq!(
            import_backup_data_with(&repository, &store, backup, true).expect("merge import"),
            0
        );

        assert_eq!(
            std::fs::read(installed.bmp_path()).expect("existing blob after merge"),
            original_bytes
        );
        assert!(installed.thumbnail_path().exists());
        assert!(repository
            .lock()
            .expect("repository lock")
            .get_item(&old_item.id)
            .expect("existing item")
            .is_some());
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn merge_new_item_rejects_conflicting_existing_blob_before_mutation() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-merge-conflict-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let staged = stage_dib(store.stage_dir(), dib32()).expect("stage old image");
        let installed = store
            .install_staged_with(staged, |installed| Ok(installed.clone()), |_| Ok(false))
            .expect("install old image");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let old_item = repository
            .lock()
            .expect("repository lock")
            .insert_or_touch_image(ClipboardItemDraft {
                item_type: ClipboardItemType::Image,
                content: None,
                content_path: Some(installed.bmp_path().to_string_lossy().to_string()),
                content_hash: Some(installed.content_hash().to_string()),
                preview: "old image".to_string(),
                source_app: None,
                size_bytes: 44,
            })
            .expect("insert old image");
        let original_bytes = std::fs::read(installed.bmp_path()).expect("existing blob");
        let filename = installed
            .bmp_path()
            .file_name()
            .and_then(|value| value.to_str())
            .expect("blob filename")
            .to_string();
        let new_id = uuid::Uuid::new_v4().to_string();
        let backup = BackupData {
            metadata: BackupMetadata {
                version: "1.0".to_string(),
                created_at: "test".to_string(),
                item_count: 1,
            },
            items: vec![crate::storage::repository::ClipboardItem {
                id: new_id.clone(),
                hash: "new-item-hash".to_string(),
                item_type: "image".to_string(),
                content: None,
                content_path: Some(filename.clone()),
                content_hash: Some("new-content-hash".to_string()),
                preview: "new image".to_string(),
                source_app: None,
                favorite: false,
                pinned: false,
                size_bytes: 9,
                created_at: 1,
                updated_at: 1,
            }],
            blobs: vec![BlobData {
                item_id: new_id.clone(),
                filename,
                data_base64: super::base64_encode(b"different"),
            }],
        };

        let error = import_backup_data_with(&repository, &store, backup, true)
            .expect_err("merge conflict must fail");

        assert!(error.contains("conflict"), "unexpected error: {error}");
        assert_eq!(
            std::fs::read(installed.bmp_path()).expect("existing blob after merge"),
            original_bytes
        );
        let repository_guard = repository.lock().expect("repository lock");
        assert!(repository_guard
            .get_item(&old_item.id)
            .expect("old item")
            .is_some());
        assert!(repository_guard
            .get_item(&new_id)
            .expect("new item")
            .is_none());
        drop(repository_guard);
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn merge_reuses_readonly_existing_blob_when_bytes_match() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-merge-reuse-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let blob_path = store.blob_dir().join("reuse.bmp");
        std::fs::write(&blob_path, b"same bytes").expect("existing blob");
        let mut permissions = std::fs::metadata(&blob_path)
            .expect("blob metadata")
            .permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&blob_path, permissions).expect("readonly blob");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let new_id = uuid::Uuid::new_v4().to_string();
        let backup = BackupData {
            metadata: BackupMetadata {
                version: "1.0".to_string(),
                created_at: "test".to_string(),
                item_count: 1,
            },
            items: vec![crate::storage::repository::ClipboardItem {
                id: new_id.clone(),
                hash: "reuse-item-hash".to_string(),
                item_type: "image".to_string(),
                content: None,
                content_path: Some("reuse.bmp".to_string()),
                content_hash: Some("reuse-content-hash".to_string()),
                preview: "reused image".to_string(),
                source_app: None,
                favorite: false,
                pinned: false,
                size_bytes: 10,
                created_at: 1,
                updated_at: 1,
            }],
            blobs: vec![BlobData {
                item_id: new_id.clone(),
                filename: "reuse.bmp".to_string(),
                data_base64: super::base64_encode(b"same bytes"),
            }],
        };

        let result = import_backup_data_with(&repository, &store, backup, true);

        let mut permissions = std::fs::metadata(&blob_path)
            .expect("blob metadata after merge")
            .permissions();
        permissions.set_readonly(false);
        std::fs::set_permissions(&blob_path, permissions).expect("writable cleanup");
        assert_eq!(result.expect("reuse existing blob"), 1);
        assert_eq!(
            std::fs::read(&blob_path).expect("reused blob"),
            b"same bytes"
        );
        assert!(repository
            .lock()
            .expect("repository lock")
            .get_item(&new_id)
            .expect("reused item")
            .is_some());
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn merge_rejects_windows_case_alias_conflict_before_mutation() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-merge-alias-conflict-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let lower_id = uuid::Uuid::new_v4().to_string();
        let upper_id = uuid::Uuid::new_v4().to_string();
        let backup = BackupData {
            metadata: BackupMetadata {
                version: "1.0".to_string(),
                created_at: "test".to_string(),
                item_count: 2,
            },
            items: vec![
                imported_image_item(&lower_id, "lower-hash", "image.bmp"),
                imported_image_item(&upper_id, "upper-hash", "IMAGE.bmp"),
            ],
            blobs: vec![
                BlobData {
                    item_id: lower_id.clone(),
                    filename: "image.bmp".to_string(),
                    data_base64: super::base64_encode(b"lower"),
                },
                BlobData {
                    item_id: upper_id.clone(),
                    filename: "IMAGE.bmp".to_string(),
                    data_base64: super::base64_encode(b"upper"),
                },
            ],
        };

        import_backup_data_with(&repository, &store, backup, true)
            .expect_err("Windows alias conflict must fail");

        assert!(std::fs::read_dir(store.blob_dir())
            .expect("blob directory")
            .next()
            .is_none());
        let repository_guard = repository.lock().expect("repository lock");
        assert!(repository_guard
            .get_item(&lower_id)
            .expect("lower item")
            .is_none());
        assert!(repository_guard
            .get_item(&upper_id)
            .expect("upper item")
            .is_none());
        drop(repository_guard);
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn merge_windows_case_alias_with_same_bytes_uses_one_target() {
        let root = std::env::temp_dir().join(format!(
            "super-clipboard-merge-alias-reuse-{}",
            uuid::Uuid::new_v4()
        ));
        let store = ImageBlobStore::new(root.join("blobs"), root.join("stage")).expect("store");
        let repository = Mutex::new(
            ClipboardRepository::open(root.join("history.sqlite3")).expect("repository"),
        );
        let lower_id = uuid::Uuid::new_v4().to_string();
        let upper_id = uuid::Uuid::new_v4().to_string();
        let backup = BackupData {
            metadata: BackupMetadata {
                version: "1.0".to_string(),
                created_at: "test".to_string(),
                item_count: 2,
            },
            items: vec![
                imported_image_item(&lower_id, "lower-hash", "image.bmp"),
                imported_image_item(&upper_id, "upper-hash", "IMAGE.bmp"),
            ],
            blobs: vec![
                BlobData {
                    item_id: lower_id.clone(),
                    filename: "image.bmp".to_string(),
                    data_base64: super::base64_encode(b"same"),
                },
                BlobData {
                    item_id: upper_id.clone(),
                    filename: "IMAGE.bmp".to_string(),
                    data_base64: super::base64_encode(b"same"),
                },
            ],
        };

        assert_eq!(
            import_backup_data_with(&repository, &store, backup, true).expect("merge aliases"),
            2
        );

        assert_eq!(
            std::fs::read_dir(store.blob_dir())
                .expect("blob directory")
                .count(),
            1
        );
        let repository_guard = repository.lock().expect("repository lock");
        let lower_path = repository_guard
            .get_item(&lower_id)
            .expect("lower item")
            .expect("active lower")
            .content_path
            .expect("lower path");
        let upper_path = repository_guard
            .get_item(&upper_id)
            .expect("upper item")
            .expect("active upper")
            .content_path
            .expect("upper path");
        assert_eq!(lower_path, upper_path);
        assert_eq!(std::fs::read(lower_path).expect("shared blob"), b"same");
        drop(repository_guard);
        drop(repository);
        drop(store);
        std::fs::remove_dir_all(root).expect("cleanup");
    }

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

    #[test]
    fn safe_backup_blob_filename_rejects_path_traversal() {
        assert!(safe_backup_blob_filename("../escape.bmp").is_err());
        assert!(safe_backup_blob_filename("nested/file.bmp").is_err());
        assert!(safe_backup_blob_filename("C:\\temp\\escape.bmp").is_err());
        assert!(safe_backup_blob_filename("image.bmp.").is_err());
        assert!(safe_backup_blob_filename("image.bmp ").is_err());
        assert_eq!(
            safe_backup_blob_filename("image.bmp").expect("safe filename"),
            "image.bmp"
        );
    }

    #[test]
    fn validate_migration_paths_rejects_nested_destination() {
        let temp =
            std::env::temp_dir().join(format!("super-clipboard-migrate-{}", uuid::Uuid::new_v4()));
        let old_dir = temp.join("old");
        let nested_new_dir = old_dir.join("backup");
        std::fs::create_dir_all(&old_dir).expect("old dir");

        let result = validate_migration_paths(&old_dir, &nested_new_dir);

        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(Path::new(&temp));
    }

    #[test]
    fn validate_migration_paths_rejects_same_directory() {
        let temp =
            std::env::temp_dir().join(format!("super-clipboard-migrate-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&temp).expect("temp dir");

        let result = validate_migration_paths(&temp, &temp);

        assert!(result.is_err());
        let _ = std::fs::remove_dir_all(Path::new(&temp));
    }
}
