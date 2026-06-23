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
    pinned: false,
    size_bytes: 5,
    created_at: 100_000,
    updated_at: 200_000,
  });

  assert.deepEqual(item, {
    id: 'id-1',
    type: 'text',
    preview: 'hello',
    contentPath: null,
    favorite: true,
    pinned: false,
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
    pinned: false,
    size_bytes: 0,
    created_at: 100_000,
    updated_at: 200_000,
  });

  assert.equal(item.type, 'text');
  assert.equal(item.preview, '(空内容)');
  assert.equal(item.source, '未知来源');
});

test('mapBackendItemToViewItem renders file list count as files', () => {
  const item = mapBackendItemToViewItem({
    id: 'id-3',
    hash: 'hash',
    item_type: 'files',
    content: '["a.txt","b.txt"]',
    content_path: null,
    preview: 'a.txt, b.txt',
    favorite: false,
    pinned: false,
    size_bytes: 2,
    created_at: 100_000,
    updated_at: 200_000,
  });

  assert.equal(item.size, '2 个文件');
});

test('mapBackendItemToViewItem keeps image content path for previews', () => {
  const item = mapBackendItemToViewItem({
    id: 'id-4',
    hash: 'hash',
    item_type: 'image',
    content_path: 'C:\\blob\\image.bmp',
    preview: 'image.bmp',
    favorite: false,
    pinned: false,
    size_bytes: 2048,
    created_at: 100_000,
    updated_at: 200_000,
  });

  assert.equal(item.contentPath, 'C:\\blob\\image.bmp');
  assert.equal(item.size, '2.0 KB');
});
