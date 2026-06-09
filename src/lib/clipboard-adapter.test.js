import test from 'node:test';
import assert from 'node:assert/strict';
import { formatBytes, mapBackendItemToViewItem } from './clipboard-adapter.js';

test('formatBytes renders compact byte labels', () => {
  assert.equal(formatBytes(12), '12 B');
  assert.equal(formatBytes(2048), '2.0 KB');
  assert.equal(formatBytes(2 * 1024 * 1024), '2.0 MB');
});

test('mapBackendItemToViewItem maps backend fields to UI fields', () => {
  const item = mapBackendItemToViewItem({
    id: 'id-1',
    hash: 'hash',
    item_type: 'text',
    content: 'hello',
    content_path: null,
    preview: 'hello',
    source_app: 'VS Code',
    favorite: true,
    size_bytes: 5,
    created_at: 100,
    updated_at: 200,
  });

  assert.deepEqual(item, {
    id: 'id-1',
    type: 'text',
    preview: 'hello',
    favorite: true,
    updatedAt: 200,
    size: '5 B',
    source: 'VS Code',
  });
});

test('mapBackendItemToViewItem falls back for unknown source and type', () => {
  const item = mapBackendItemToViewItem({
    id: 'id-2',
    hash: 'hash',
    item_type: 'unknown',
    preview: '',
    favorite: false,
    size_bytes: 0,
    created_at: 100,
    updated_at: 200,
  });

  assert.equal(item.type, 'text');
  assert.equal(item.preview, '(空内容)');
  assert.equal(item.source, '未知来源');
});
