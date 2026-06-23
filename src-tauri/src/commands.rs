use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_updater::UpdaterExt;

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

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
pub fn get_item_detail(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<ClipboardItem>, String> {
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
pub fn delete_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
    crate::diagnostics::info(format!("command: delete_item id={id}"));
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository
        .soft_delete(&id)
        .map_err(|error| error.to_string())
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
    {
        let repository = state.repository.lock().map_err(|error| error.to_string())?;
        repository
            .prune_history(
                next_settings.max_history_items,
                next_settings.retention_days,
            )
            .map_err(|error| error.to_string())?;
    }
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
pub fn clear_history(state: State<'_, AppState>) -> Result<(), String> {
    crate::diagnostics::warn("command: clear_history");
    let repository = state.repository.lock().map_err(|error| error.to_string())?;
    repository
        .clear_history()
        .map_err(|error| error.to_string())?;
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

    // Create the new directory first to ensure it can be safely canonicalized
    // This prevents symlink-based path traversal attacks
    if !new_dir.exists() {
        std::fs::create_dir_all(new_dir)
            .map_err(|e| format!("创建新目录失败: {}", e))?;
    }

    let new_dir = new_dir
        .canonicalize()
        .map_err(|e| format!("解析新目录失败: {}", e))?;

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

            if entry.path().is_file() {
                fs::copy(&old_file, &new_file)
                    .map_err(|e| format!("复制文件 {} 失败: {}", file_name.to_string_lossy(), e))?;
                crate::diagnostics::info(format!("migrated: {}", file_name.to_string_lossy()));
            } else if entry.path().is_dir() {
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
    use std::fs;

    // 选择保存路径
    let default_filename = format!(
        "super-clipboard-backup-{}.json",
        Utc::now().format("%Y%m%d-%H%M%S")
    );

    let save_path = FileDialog::new()
        .set_title("导出备份")
        .set_file_name(&default_filename)
        .add_filter("JSON 文件", &["json"])
        .save_file();

    let Some(save_path) = save_path else {
        return Err("用户取消导出".to_string());
    };

    crate::diagnostics::info(format!("export_backup: path={}", save_path.display()));

    // 读取所有数据
    let repository = state.repository.lock().map_err(|e| e.to_string())?;
    let mut all_items = repository
        .search(
            "".to_string(),
            SearchFilters {
                item_type: None,
                favorites_only: false,
            },
            100000,
            None,
        )
        .map_err(|e| format!("读取数据失败: {}", e))?;

    drop(repository);

    // 收集关联的 blob 文件
    let mut blobs = Vec::new();
    for item in &mut all_items {
        if let Some(content_path) = &item.content_path {
            let content_path = Path::new(content_path);
            let blob_path = if content_path.is_absolute() {
                content_path.to_path_buf()
            } else {
                state.blob_dir.join(content_path)
            };
            if blob_path.exists() {
                let Some(filename) = blob_path
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(str::to_string)
                else {
                    crate::diagnostics::warn(format!(
                        "export_backup: skipped blob with invalid filename {}",
                        blob_path.display()
                    ));
                    item.content_path = None;
                    continue;
                };
                match fs::read(&blob_path) {
                    Ok(data) => {
                        blobs.push(BlobData {
                            item_id: item.id.clone(),
                            filename: filename.clone(),
                            data_base64: base64_encode(&data),
                        });
                        item.content_path = Some(filename);
                    }
                    Err(e) => {
                        crate::diagnostics::warn(format!(
                            "export_backup: failed to read blob {}: {}",
                            blob_path.display(),
                            e
                        ));
                        item.content_path = None;
                    }
                }
            }
        }
    }

    // 构建备份数据
    let backup = BackupData {
        metadata: BackupMetadata {
            version: "1.0".to_string(),
            created_at: Utc::now().to_rfc3339(),
            item_count: all_items.len(),
        },
        items: all_items,
        blobs,
    };

    // 写入文件
    let json = serde_json::to_string_pretty(&backup).map_err(|e| format!("序列化失败: {}", e))?;

    fs::write(&save_path, json).map_err(|e| format!("写入文件失败: {}", e))?;

    crate::diagnostics::info(format!(
        "export_backup: success, {} items, {} blobs",
        backup.metadata.item_count,
        backup.blobs.len()
    ));

    Ok(save_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn select_backup_file() -> Result<Option<String>, String> {
    use rfd::FileDialog;

    let selected = FileDialog::new()
        .set_title("选择备份文件")
        .add_filter("JSON 文件", &["json"])
        .pick_file();

    Ok(selected.map(|path| path.to_string_lossy().to_string()))
}

#[tauri::command]
pub async fn parse_backup_info(backup_path: String) -> Result<BackupInfo, String> {
    use std::fs;

    let content =
        fs::read_to_string(&backup_path).map_err(|e| format!("读取备份文件失败: {}", e))?;

    let backup: BackupData =
        serde_json::from_str(&content).map_err(|e| format!("解析备份文件失败: {}", e))?;

    Ok(BackupInfo {
        created_at: backup.metadata.created_at,
        item_count: backup.metadata.item_count,
        version: backup.metadata.version,
    })
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
            .search(
                "".to_string(),
                SearchFilters {
                    item_type: None,
                    favorites_only: false,
                },
                100000,
                None,
            )
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

    // 如果是覆盖模式，清空现有数据
    if !merge {
        let repository = state.repository.lock().map_err(|e| e.to_string())?;
        repository
            .clear_history()
            .map_err(|e| format!("清空历史失败: {}", e))?;
        drop(repository);

        crate::blobs::clear_blob_dir(&state.blob_dir)
            .map_err(|e| format!("清空 blob 目录失败: {}", e))?;

        crate::diagnostics::info("import_backup: existing data cleared");
    }

    // 恢复 blob 文件
    let mut restored_blob_map: HashMap<String, String> = HashMap::new();
    for blob in &backup.blobs {
        let filename = safe_backup_blob_filename(&blob.filename)?;
        let blob_path = state.blob_dir.join(&filename);
        let data = base64_decode(&blob.data_base64)
            .map_err(|e| format!("解码 blob {} 失败: {}", blob.filename, e))?;

        fs::write(&blob_path, data)
            .map_err(|e| format!("写入 blob {} 失败: {}", blob.filename, e))?;

        // Use filename as key to avoid confusion with ID conflicts in merge mode
        restored_blob_map.insert(blob.filename.clone(), blob_path.to_string_lossy().to_string());
    }

    // 导入数据到数据库
    let repository = state.repository.lock().map_err(|e| e.to_string())?;
    let mut imported_count = 0;

    for mut item in backup.items {
        // Restore blob path using the original filename from backup
        if let Some(original_filename) = &item.content_path {
            item.content_path = restored_blob_map.get(original_filename).cloned();
        }

        // 在合并模式下，检查是否已存在相同 hash 的记录
        if merge {
            if let Ok(Some(_)) = repository.find_by_hash(&item.hash) {
                continue; // 跳过重复项
            }
        }

        // 插入记录（需要实现 insert_item 方法）
        // 注意：这里需要修改 repository 以支持直接插入完整的 ClipboardItem
        repository
            .insert_imported_item(&item)
            .map_err(|e| format!("导入记录失败: {}", e))?;

        imported_count += 1;
    }

    drop(repository);

    crate::diagnostics::info(format!(
        "import_backup: success, imported {} items",
        imported_count
    ));

    Ok(imported_count)
}

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

    use super::{html_to_plain_text, safe_backup_blob_filename, validate_migration_paths};

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
