export type AppSettings = {
  recording_enabled: boolean;
  max_history_items: number;
  retention_days: number;
  global_shortcut: string;
  autostart_enabled: boolean;
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
