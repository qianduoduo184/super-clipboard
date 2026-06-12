export function createDefaultSettings() {
  return {
    recording_enabled: true,
    max_history_items: 10000,
    retention_days: 30,
    global_shortcut: 'Ctrl+Shift+V',
    autostart_enabled: false,
    preview_enabled: true,
    theme_mode: 'light',
    auto_update_enabled: false,
    last_update_check_date: null,
    nav_filters_config: {
      visible: ['all', 'favorites', 'text', 'image', 'files'],
    },
  };
}

export function mergeSettings(settings) {
  return {
    ...createDefaultSettings(),
    ...settings,
  };
}

export function applyThemeMode(themeMode) {
  const normalizedTheme = themeMode === 'dark' ? 'dark' : 'light';
  document.documentElement.dataset.theme = normalizedTheme;
  return normalizedTheme;
}

export function formatShortcutFromEvent(event) {
  const parts = [];
  if (event.ctrlKey) parts.push('Ctrl');
  if (event.altKey) parts.push('Alt');
  if (event.shiftKey) parts.push('Shift');
  if (event.metaKey) parts.push('Meta');

  const key = normalizeShortcutKey(event.key);
  if (key && !['Ctrl', 'Alt', 'Shift', 'Meta'].includes(key)) {
    parts.push(key);
  }

  return parts.join('+');
}

export function validateShortcut(shortcut) {
  const parts = shortcut.split('+').map((part) => part.trim()).filter(Boolean);
  const hasModifier = parts.some((part) => ['Ctrl', 'Alt', 'Shift', 'Meta'].includes(part));
  const hasKey = parts.some((part) => !['Ctrl', 'Alt', 'Shift', 'Meta'].includes(part));
  return hasModifier && hasKey;
}

function normalizeShortcutKey(key) {
  if (!key) return '';
  if (key === 'Control') return 'Ctrl';
  if (key === 'Alt') return 'Alt';
  if (key === 'Shift') return 'Shift';
  if (key === 'Meta') return 'Meta';
  if (key === ' ') return 'Space';
  if (key.length === 1) return key.toUpperCase();
  return key.length > 1 ? key[0].toUpperCase() + key.slice(1) : key;
}

export function updateSettingValue(settings, key, value) {
  return {
    ...settings,
    [key]: value,
  };
}

export function getErrorMessage(error, fallback) {
  if (typeof error === 'string' && error.trim().length > 0) {
    return error;
  }
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message;
  }
  return fallback;
}

export function shouldClearHistory(confirmed) {
  return confirmed === true;
}

export function shouldCheckForUpdatesToday(enabled, lastCheckDate, today) {
  return enabled === true && lastCheckDate !== today;
}

export function toLocalDateString(date) {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${year}-${month}-${day}`;
}
