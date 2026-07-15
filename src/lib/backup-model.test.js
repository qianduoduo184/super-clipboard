import test from 'node:test';
import assert from 'node:assert/strict';
import {
  BACKUP_FILE_EXTENSIONS,
  getBackupFormat,
  mapBackendBackupInfo,
} from './backup-model.js';

test('backup file filters include ZIP export and legacy JSON import', () => {
  assert.deepEqual(BACKUP_FILE_EXTENSIONS, ['zip', 'json']);
  assert.equal(getBackupFormat('C:\\backup\\history.ZIP'), 'ZIP');
  assert.equal(getBackupFormat('/backup/legacy.json'), '旧版 JSON');
  assert.equal(getBackupFormat('/backup/unknown.txt'), '未知格式');
});

test('backup info maps the backend snake_case contract for settings preview', () => {
  assert.deepEqual(
    mapBackendBackupInfo({
      created_at: '2026-07-15T00:00:00Z',
      item_count: 42,
      version: '2',
    }),
    {
      createdAt: '2026-07-15T00:00:00Z',
      itemCount: 42,
      version: '2',
    },
  );
});
