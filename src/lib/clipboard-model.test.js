import test from 'node:test';
import assert from 'node:assert/strict';
import {
  filterItems,
  getTypeLabel,
  normalizePreview,
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
