import test from 'node:test';
import assert from 'node:assert/strict';
import {
  createDefaultSettings,
  formatShortcutFromEvent,
  getErrorMessage,
  mergeSettings,
  shouldCheckForUpdatesToday,
  shouldClearHistory,
  toLocalDateString,
  updateSettingValue,
  validateShortcut,
} from './settings-model.js';

test('createDefaultSettings returns product defaults', () => {
  assert.deepEqual(createDefaultSettings(), {
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
    custom_data_dir: null,
    custom_log_dir: null,
  });
});

test('mergeSettings keeps defaults for missing backend values', () => {
  assert.deepEqual(mergeSettings({ recording_enabled: false }), {
    recording_enabled: false,
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
    custom_data_dir: null,
    custom_log_dir: null,
  });
});

test('updateSettingValue updates one setting without mutating original', () => {
  const settings = createDefaultSettings();
  const next = updateSettingValue(settings, 'max_history_items', 50000);

  assert.equal(settings.max_history_items, 10000);
  assert.equal(next.max_history_items, 50000);
});

test('getErrorMessage preserves string errors returned by Tauri invoke', () => {
  assert.equal(getErrorMessage('copy is not implemented for image items', '复制失败'), 'copy is not implemented for image items');
});

test('getErrorMessage uses fallback for unknown errors', () => {
  assert.equal(getErrorMessage({ code: 'UNKNOWN' }, '复制失败'), '复制失败');
});

test('shouldClearHistory only allows explicit confirmation', () => {
  assert.equal(shouldClearHistory(true), true);
  assert.equal(shouldClearHistory(false), false);
});

test('formatShortcutFromEvent formats modifier combinations', () => {
  assert.equal(
    formatShortcutFromEvent({
      key: 'v',
      ctrlKey: true,
      altKey: true,
      shiftKey: false,
      metaKey: false,
    }),
    'Ctrl+Alt+V',
  );
});

test('validateShortcut requires a modifier and a main key', () => {
  assert.equal(validateShortcut('Ctrl+Shift+V'), true);
  assert.equal(validateShortcut('Ctrl+Shift'), false);
  assert.equal(validateShortcut('V'), false);
});

test('updateSettingValue supports theme switching', () => {
  const settings = createDefaultSettings();
  const next = updateSettingValue(settings, 'theme_mode', 'dark');

  assert.equal(settings.theme_mode, 'light');
  assert.equal(next.theme_mode, 'dark');
});

test('mergeSettings preserves auto update values from backend', () => {
  assert.deepEqual(mergeSettings({
    auto_update_enabled: true,
    last_update_check_date: '2026-06-10',
  }), {
    recording_enabled: true,
    max_history_items: 10000,
    retention_days: 30,
    global_shortcut: 'Ctrl+Shift+V',
    autostart_enabled: false,
    preview_enabled: true,
    theme_mode: 'light',
    auto_update_enabled: true,
    last_update_check_date: '2026-06-10',
    nav_filters_config: {
      visible: ['all', 'favorites', 'text', 'image', 'files'],
    },
    custom_data_dir: null,
    custom_log_dir: null,
  });
});

test('shouldCheckForUpdatesToday only runs once per enabled day', () => {
  assert.equal(shouldCheckForUpdatesToday(false, null, '2026-06-10'), false);
  assert.equal(shouldCheckForUpdatesToday(true, '2026-06-10', '2026-06-10'), false);
  assert.equal(shouldCheckForUpdatesToday(true, '2026-06-09', '2026-06-10'), true);
  assert.equal(shouldCheckForUpdatesToday(true, null, '2026-06-10'), true);
});

test('toLocalDateString formats local date with zero padding', () => {
  assert.equal(toLocalDateString(new Date(2026, 5, 9)), '2026-06-09');
  assert.equal(toLocalDateString(new Date(2026, 0, 5)), '2026-01-05');
});
