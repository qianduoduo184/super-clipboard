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

export type AppSettings = {
  recording_enabled: boolean;
  max_history_items: number;
  retention_days: number;
  global_shortcut: string;
  autostart_enabled: boolean;
  preview_enabled: boolean;
  theme_mode: 'light' | 'dark';
};

export type DiagnosticsInfo = {
  app_data_dir: string;
  log_path: string;
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
