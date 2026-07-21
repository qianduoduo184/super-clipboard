import test from 'node:test';
import assert from 'node:assert/strict';
import { performance } from 'node:perf_hooks';
import {
  filterItems,
  getTypeLabel,
  getVisibleFilters,
  getVisualPreview,
  normalizePreview,
  reorderItemsByDrag,
  reorderNavFiltersByDrag,
  sortItemsByUpdatedTime,
} from './clipboard-model.js';

const items = [
  {
    id: '1',
    type: 'text',
    preview: 'SQLite WAL clipboard history',
    favorite: false,
    updatedAt: 300,
  },
  {
    id: '2',
    type: 'image',
    preview: 'screenshot.png',
    favorite: true,
    updatedAt: 100,
  },
  {
    id: '3',
    type: 'files',
    preview: 'report.docx',
    favorite: false,
    updatedAt: 200,
  },
];

test('sortItemsByUpdatedTime returns newest clipboard records first', () => {
  assert.deepEqual(sortItemsByUpdatedTime(items).map((item) => item.id), ['1', '3', '2']);
});

test('filterItems filters by favorite and search query', () => {
  assert.deepEqual(
    filterItems(items, { type: 'favorites', query: 'screen' }).map((item) => item.id),
    ['2'],
  );
});

test('filterItems filters by concrete clipboard type', () => {
  assert.deepEqual(filterItems(items, { type: 'files', query: '' }).map((item) => item.id), ['3']);
});

test('normalizePreview keeps compact single-line previews', () => {
  assert.equal(normalizePreview(' first line\nsecond line ', 16), 'first line seco...');
});

test('getTypeLabel returns Chinese labels for supported clipboard types', () => {
  assert.equal(getTypeLabel('text'), '文本');
  assert.equal(getTypeLabel('html'), 'HTML');
  assert.equal(getTypeLabel('image'), '图片');
  assert.equal(getTypeLabel('files'), '文件');
});

test('getVisibleFilters returns all filters with default config', () => {
  assert.deepEqual(
    getVisibleFilters({ visible: ['all', 'favorites', 'text', 'image', 'files'] }).map((filter) => filter.key),
    ['all', 'favorites', 'text', 'image', 'files'],
  );
});

test('getVisibleFilters respects custom config', () => {
  assert.deepEqual(
    getVisibleFilters({ visible: ['all', 'text'] }).map((filter) => filter.key),
    ['all', 'text'],
  );
});

test('getVisibleFilters falls back to all when config is empty', () => {
  assert.deepEqual(
    getVisibleFilters({ visible: [] }).map((filter) => filter.key),
    ['all', 'favorites', 'text', 'image', 'files'],
  );
});

test('getVisualPreview hides image file paths and names', () => {
  assert.equal(
    getVisualPreview({
      type: 'image',
      preview: 'C:\\Users\\user\\AppData\\Roaming\\super-clipboard\\blobs\\image.bmp',
    }),
    '',
  );
});

test('reorderItemsByDrag moves an item after the drop target', () => {
  assert.deepEqual(
    reorderItemsByDrag(['a', 'b', 'c', 'd'], 'd', 'b'),
    ['a', 'b', 'd', 'c'],
  );
});

test('reorderNavFiltersByDrag moves a filter after its drop target', () => {
  assert.deepEqual(
    reorderNavFiltersByDrag(['all', 'favorites', 'text', 'image'], 'favorites', 'image'),
    ['all', 'text', 'image', 'favorites'],
  );
});

test('reorderNavFiltersByDrag keeps all first and rejects invalid drags', () => {
  const order = ['all', 'favorites', 'text', 'image'];
  assert.deepEqual(reorderNavFiltersByDrag(order, 'image', 'all'), ['all', 'image', 'favorites', 'text']);
  assert.deepEqual(reorderNavFiltersByDrag(order, 'all', 'text'), order);
  assert.deepEqual(reorderNavFiltersByDrag(order, 'missing', 'text'), order);
  assert.deepEqual(reorderNavFiltersByDrag(order, 'text', 'text'), order);
});

test('filterItems handles 1,000 copied text records without obvious slowdown', () => {
  const copiedTextItems = Array.from({ length: 1000 }, (_, index) => ({
    id: `text-${index}`,
    type: 'text',
    preview: `clipboard copied text ${index}`,
    favorite: index % 100 === 0,
    updatedAt: index,
  }));

  const startedAt = performance.now();
  const results = filterItems(copiedTextItems, { type: 'text', query: 'copied text 99' });
  const duration = performance.now() - startedAt;

  assert.equal(results.length, 11);
  assert.ok(duration < 100, `expected 1,000 item filter under 100ms, got ${duration.toFixed(2)}ms`);
});
