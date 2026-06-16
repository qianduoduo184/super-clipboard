import { invoke } from '@tauri-apps/api/core';

export type ClipboardFilter = 'all' | 'favorites' | 'text' | 'html' | 'image' | 'files';

export type ClipboardSearchItem = {
  id: string;
  hash: string;
  item_type: string;
  content?: string | null;
  content_path?: string | null;
  preview: string;
  source_app?: string | null;
  favorite: boolean;
  size_bytes: number;
  created_at: number;
  updated_at: number;
};

export type SearchFilters = {
  item_type?: string | null;
  favorites_only: boolean;
};

export type NavFiltersConfig = {
  visible: string[];
};

export type AppSettings = {
  recording_enabled: boolean;
  max_history_items: number;
  retention_days: number;
  global_shortcut: string;
  autostart_enabled: boolean;
  preview_enabled: boolean;
  theme_mode: 'light' | 'dark';
  auto_update_enabled: boolean;
  last_update_check_date: string | null;
  nav_filters_config: NavFiltersConfig;
  custom_data_dir?: string | null;
  custom_log_dir?: string | null;
};

export type DiagnosticsInfo = {
  app_data_dir: string;
  log_path: string;
};

export type UpdateInfo = {
  available: boolean;
  version?: string | null;
  body?: string | null;
};

export function toSearchFilters(filter: ClipboardFilter): SearchFilters {
  return {
    item_type: filter === 'all' || filter === 'favorites' ? null : filter,
    favorites_only: filter === 'favorites',
  };
}

export async function searchItems(query: string, filter: ClipboardFilter, cursor?: number) {
  return invoke<ClipboardSearchItem[]>('search_items', {
    query,
    filters: toSearchFilters(filter),
    limit: 50,
    cursor: cursor ?? null,
  });
}

export async function getItemDetail(id: string) {
  return invoke<ClipboardSearchItem | null>('get_item_detail', { id });
}

export async function copyItem(id: string) {
  return invoke<void>('copy_item', { id });
}

export async function pasteItem(id: string) {
  return invoke<void>('paste_item', { id });
}

export async function toggleFavorite(id: string) {
  return invoke<void>('toggle_favorite', { id });
}

export async function deleteItem(id: string) {
  return invoke<void>('delete_item', { id });
}

export async function reorderItems(ids: string[]) {
  return invoke<void>('reorder_items', { ids });
}

export async function setRecordingEnabled(enabled: boolean) {
  return invoke<void>('set_recording_enabled', { enabled });
}

export async function getSettings() {
  return invoke<AppSettings>('get_settings');
}

export async function updateSettings(nextSettings: AppSettings) {
  return invoke<AppSettings>('update_settings', { nextSettings });
}

export async function setGlobalShortcut(shortcut: string) {
  return invoke<AppSettings>('set_global_shortcut', { shortcut });
}

export async function clearHistory() {
  return invoke<void>('clear_history');
}

export async function getDiagnostics() {
  return invoke<DiagnosticsInfo>('get_diagnostics');
}

export async function checkForUpdates() {
  return invoke<UpdateInfo>('check_update');
}

export async function installUpdate() {
  return invoke<void>('install_update');
}

// 存储路径管理
export async function selectDirectory() {
  return invoke<string | null>('select_directory');
}

export async function migrateDirectory(oldPath: string, newPath: string, moveFiles: boolean) {
  return invoke<void>('migrate_directory', { oldPath, newPath, moveFiles });
}

export async function updateStorageSettings(customDataDir: string | null, customLogDir: string | null) {
  return invoke<AppSettings>('update_storage_settings', { customDataDir, customLogDir });
}

// 导入/导出备份
export type BackupInfo = {
  created_at: string;
  item_count: number;
  version: string;
};

export async function exportBackup() {
  return invoke<string>('export_backup');
}

export async function selectBackupFile() {
  return invoke<string | null>('select_backup_file');
}

export async function parseBackupInfo(backupPath: string) {
  return invoke<BackupInfo>('parse_backup_info', { backupPath });
}

export async function importBackup(backupPath: string, merge: boolean) {
  return invoke<number>('import_backup', { backupPath, merge });
}

