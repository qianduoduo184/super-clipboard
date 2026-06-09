export function createDefaultSettings() {
  return {
    recording_enabled: true,
    max_history_items: 10000,
    retention_days: 30,
    global_shortcut: 'Ctrl+Shift+V',
    autostart_enabled: false,
  };
}

export function mergeSettings(settings) {
  return {
    ...createDefaultSettings(),
    ...settings,
  };
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
