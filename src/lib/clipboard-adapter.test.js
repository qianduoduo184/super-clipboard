import test from 'node:test';
import assert from 'node:assert/strict';
import * as clipboardAdapter from './clipboard-adapter.js';

const {
  beginDetailRequest,
  cacheItemDetailById,
  formatBytes,
  getDetailDisplayContent,
  isDetailResponseCurrent,
  mapBackendItemDetailToViewItem,
  mapBackendItemToViewItem,
} = clipboardAdapter;

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
    thumbnailPath: null,
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

test('mapBackendItemToViewItem maps summary image paths without inventing full content', () => {
  const item = mapBackendItemToViewItem({
    id: 'summary-image',
    hash: 'hash',
    item_type: 'image',
    content_path: 'C:\\blob\\image.bmp',
    thumbnail_path: 'C:\\blob\\image.thumb.png',
    preview: 'image.bmp',
    source_app: 'Snipping Tool',
    favorite: false,
    pinned: false,
    size_bytes: 2048,
    created_at: 100_000,
    updated_at: 200_000,
  });

  assert.equal(item.thumbnailPath, 'C:\\blob\\image.thumb.png');
  assert.equal(item.contentPath, 'C:\\blob\\image.bmp');
  assert.equal(Object.hasOwn(item, 'content'), false);
});

test('mapBackendItemDetailToViewItem maps full content from item detail', () => {
  const item = mapBackendItemDetailToViewItem({
    id: 'detail-text',
    hash: 'hash',
    item_type: 'text',
    content: '完整的多行正文\n第二行',
    content_path: null,
    preview: '完整的多行正文...',
    source_app: 'VS Code',
    favorite: false,
    pinned: false,
    size_bytes: 30,
    created_at: 100_000,
    updated_at: 200_000,
  });

  assert.equal(item.content, '完整的多行正文\n第二行');
  assert.equal(item.contentPath, null);
});

test('cacheItemDetailById enriches only the matching cache entry without changing list order', () => {
  const items = [{ id: 'a' }, { id: 'b' }];
  const cachedA = { id: 'a', content: 'A' };
  const detailB = { id: 'b', content: '完整 B' };
  const detailsById = { a: cachedA };

  const nextDetails = cacheItemDetailById(detailsById, detailB);

  assert.deepEqual(items.map((item) => item.id), ['a', 'b']);
  assert.equal(nextDetails.a, cachedA);
  assert.equal(nextDetails.b, detailB);
  assert.equal(detailsById.b, undefined);
});

test('detail request generation rejects a late response after selection changes', () => {
  const requestA = beginDetailRequest({ itemId: null, generation: 0 }, 'a');
  const requestB = beginDetailRequest(requestA, 'b');

  assert.equal(isDetailResponseCurrent(requestB, requestA, 'b'), false);
  assert.equal(isDetailResponseCurrent(requestB, requestB, 'b'), true);
});

test('getDetailDisplayContent prefers fetched full text or HTML over summary preview', () => {
  const summary = { id: 'html-1', preview: '<p>截断...</p>' };
  const detail = { id: 'html-1', content: '<p>完整 HTML 正文</p>' };

  assert.equal(getDetailDisplayContent(summary, detail), '<p>完整 HTML 正文</p>');
  assert.equal(getDetailDisplayContent(summary, undefined), '<p>截断...</p>');
  assert.equal(getDetailDisplayContent(summary, { id: 'other', content: '错误内容' }), '<p>截断...</p>');
});
