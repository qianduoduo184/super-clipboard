import test from 'node:test';
import assert from 'node:assert/strict';
import {
  BACKUP_FILE_EXTENSIONS,
  getBackupFormat,
  mapBackendBackupInfo,
} from './backup-model.js';

test('backup file filters include ZIP export and legacy JSON import', () => {
  assert.deepEqual(BACKUP_FILE_EXTENSIONS, ['zip', 'json']);
});

test('backup format follows parsed version when the file extension is misleading', () => {
  const zipRenamedAsJson = {
    path: '/backup/history.json',
    info: { createdAt: '2026-07-15T00:00:00Z', itemCount: 42, version: '2' },
  };
  const legacyRenamedAsZip = {
    path: '/backup/legacy.zip',
    info: { createdAt: '2026-07-15T00:00:00Z', itemCount: 42, version: '1.0' },
  };

  assert.equal(getBackupFormat(zipRenamedAsJson.info), 'ZIP');
  assert.equal(getBackupFormat(legacyRenamedAsZip.info), '旧版 JSON');
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
