import test from 'node:test';
import assert from 'node:assert/strict';
import {
  createDefaultSettings,
  getErrorMessage,
  mergeSettings,
  shouldClearHistory,
  updateSettingValue,
} from './settings-model.js';

test('createDefaultSettings returns product defaults', () => {
  assert.deepEqual(createDefaultSettings(), {
    recording_enabled: true,
    max_history_items: 10000,
    retention_days: 30,
    global_shortcut: 'Ctrl+Shift+V',
    autostart_enabled: false,
  });
});

test('mergeSettings keeps defaults for missing backend values', () => {
  assert.deepEqual(mergeSettings({ recording_enabled: false }), {
    recording_enabled: false,
    max_history_items: 10000,
    retention_days: 30,
    global_shortcut: 'Ctrl+Shift+V',
    autostart_enabled: false,
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
