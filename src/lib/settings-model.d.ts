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
};

export function createDefaultSettings(): AppSettings;

export function mergeSettings(settings: Partial<AppSettings>): AppSettings;

export function updateSettingValue<K extends keyof AppSettings>(
  settings: AppSettings,
  key: K,
  value: AppSettings[K],
): AppSettings;

export function getErrorMessage(error: unknown, fallback: string): string;

export function shouldClearHistory(confirmed: boolean): boolean;

export function shouldCheckForUpdatesToday(
  enabled: boolean,
  lastCheckDate: string | null,
  today: string,
): boolean;

export function applyThemeMode(themeMode: string): 'light' | 'dark';

export function formatShortcutFromEvent(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>): string;

export function validateShortcut(shortcut: string): boolean;
