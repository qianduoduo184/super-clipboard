import test from 'node:test';
import assert from 'node:assert/strict';
import * as clipboardAdapter from './clipboard-adapter.js';

const {
  advanceImageFallback,
  beginDetailRequest,
  createImageFallbackState,
  createItemIdentity,
  formatBytes,
  getDetailDisplayContent,
  getImageFallbackPath,
  mapBackendItemDetailToViewItem,
  mapBackendItemToViewItem,
  reconcileDetailSlot,
  reconcileImageFallbackState,
  resolveDetailResponse,
  selectDetailLoadStatus,
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
    hash: 'hash',
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
  assert.equal(item.hash, 'hash');
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

test('reconcileDetailSlot invalidates cached detail when authoritative summary identity changes', () => {
  const summary = { id: 'a', hash: 'hash-a', updatedAt: 100 };
  const slot = {
    identity: createItemIdentity(summary),
    detail: { ...summary, content: '旧正文' },
  };

  assert.equal(reconcileDetailSlot(slot, { ...summary, hash: 'hash-new' }), null);
  assert.equal(reconcileDetailSlot(slot, { ...summary, updatedAt: 101 }), null);
});

test('reconcileDetailSlot evicts detail when selected item is deleted or history is cleared', () => {
  const summary = { id: 'a', hash: 'hash-a', updatedAt: 100 };
  const slot = {
    identity: createItemIdentity(summary),
    detail: { ...summary, content: '旧正文' },
  };

  assert.equal(reconcileDetailSlot(slot, undefined), null);
  assert.equal(reconcileDetailSlot(slot, null), null);
});

test('resolveDetailResponse rejects old generation and old identity responses', () => {
  const identityA = createItemIdentity({ id: 'a', hash: 'hash-a', updatedAt: 100 });
  const refreshedIdentityA = createItemIdentity({ id: 'a', hash: 'hash-new', updatedAt: 101 });
  const requestA = beginDetailRequest({ identity: null, generation: 0 }, identityA);
  const refreshedRequestA = beginDetailRequest(requestA, refreshedIdentityA);
  const oldDetail = { id: 'a', hash: 'hash-a', updatedAt: 100, content: '旧正文' };

  assert.equal(resolveDetailResponse(refreshedRequestA, requestA, refreshedIdentityA, oldDetail), null);
  assert.equal(resolveDetailResponse(requestA, requestA, refreshedIdentityA, oldDetail), null);
  assert.equal(
    resolveDetailResponse(requestA, requestA, identityA, { ...oldDetail, hash: 'unexpected-hash' }),
    null,
  );
});

test('resolveDetailResponse stores at most the current selected detail', () => {
  const identityA = createItemIdentity({ id: 'a', hash: 'hash-a', updatedAt: 100 });
  const identityB = createItemIdentity({ id: 'b', hash: 'hash-b', updatedAt: 200 });
  const requestA = beginDetailRequest({ identity: null, generation: 0 }, identityA);
  const requestB = beginDetailRequest(requestA, identityB);
  const detailB = { id: 'b', hash: 'hash-b', updatedAt: 200, content: '完整 B' };

  const slot = resolveDetailResponse(requestB, requestB, identityB, detailB);

  assert.deepEqual(slot, { identity: identityB, detail: detailB });
  assert.equal(Object.hasOwn(slot, 'a'), false);
});

test('getDetailDisplayContent prefers fetched full text or HTML over summary preview', () => {
  const summary = { id: 'html-1', preview: '<p>截断...</p>' };
  const detail = { id: 'html-1', content: '<p>完整 HTML 正文</p>' };

  assert.equal(getDetailDisplayContent(summary, detail), '<p>完整 HTML 正文</p>');
  assert.equal(getDetailDisplayContent(summary, undefined), '<p>截断...</p>');
  assert.equal(getDetailDisplayContent(summary, { id: 'other', content: '错误内容' }), '<p>截断...</p>');
});

test('image fallback tries thumbnail then original once before showing the type icon', () => {
  let state = createImageFallbackState('thumb.png', 'original.bmp');
  assert.equal(getImageFallbackPath(state), 'thumb.png');

  state = advanceImageFallback(state);
  assert.equal(getImageFallbackPath(state), 'original.bmp');

  state = advanceImageFallback(state);
  assert.equal(getImageFallbackPath(state), null);
  assert.equal(advanceImageFallback(state), state);
});

test('image fallback resets to a new thumbnail when paths change', () => {
  const failed = advanceImageFallback(advanceImageFallback(
    createImageFallbackState('old-thumb.png', 'old-original.bmp'),
  ));

  const reset = reconcileImageFallbackState(failed, 'new-thumb.png', 'new-original.bmp');

  assert.equal(getImageFallbackPath(reset), 'new-thumb.png');
});

test('selectDetailLoadStatus hides loading and errors from a different selection', () => {
  const identityA = createItemIdentity({ id: 'a', hash: 'hash-a', updatedAt: 100 });
  const identityB = createItemIdentity({ id: 'b', hash: 'hash-b', updatedAt: 200 });
  const statusA = { identity: identityA, loading: true, error: 'A 加载失败' };

  assert.deepEqual(selectDetailLoadStatus(statusA, identityB), { loading: false, error: null });
  assert.deepEqual(selectDetailLoadStatus(statusA, identityA), { loading: true, error: 'A 加载失败' });
});
