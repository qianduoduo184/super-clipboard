import test from 'node:test';
import assert from 'node:assert/strict';
import * as historyUi from './history-ui.js';

const {
  calculateVirtualWindow,
  mergeHistoryPage,
  moveSelection,
  shouldLoadNextHistoryPage,
} = historyUi;

test('calculateVirtualWindow returns a bounded visible slice', () => {
  const window = calculateVirtualWindow({
    itemCount: 100,
    scrollTop: 330,
    itemHeight: 66,
    viewportHeight: 198,
    overscan: 1,
  });

  assert.deepEqual(window, {
    startIndex: 4,
    endIndex: 9,
    offsetTop: 264,
  });
});

test('calculateVirtualWindow handles empty lists', () => {
  assert.deepEqual(
    calculateVirtualWindow({
      itemCount: 0,
      scrollTop: 300,
      itemHeight: 66,
      viewportHeight: 198,
      overscan: 1,
    }),
    { startIndex: 0, endIndex: 0, offsetTop: 0 },
  );
});

test('moveSelection moves within list bounds', () => {
  assert.equal(moveSelection(['a', 'b', 'c'], 'b', 'down'), 'c');
  assert.equal(moveSelection(['a', 'b', 'c'], 'b', 'up'), 'a');
  assert.equal(moveSelection(['a', 'b', 'c'], 'c', 'down'), 'c');
  assert.equal(moveSelection(['a', 'b', 'c'], 'a', 'up'), 'a');
});

test('moveSelection selects first item when current id is missing', () => {
  assert.equal(moveSelection(['a', 'b', 'c'], 'missing', 'down'), 'a');
});

test('calculateVirtualWindow keeps 10,000 item history lists bounded', () => {
  const window = calculateVirtualWindow({
    itemCount: 10000,
    scrollTop: 6600,
    itemHeight: 66,
    viewportHeight: 396,
    overscan: 3,
  });

  assert.deepEqual(window, {
    startIndex: 97,
    endIndex: 109,
    offsetTop: 6402,
  });
  assert.ok(window.endIndex - window.startIndex <= 12);
});

test('mergeHistoryPage appends unseen records without duplicating refreshed ids', () => {
  const current = [
    { id: 'a', preview: 'old a' },
    { id: 'b', preview: 'old b' },
  ];
  const incoming = [
    { id: 'b', preview: 'new b' },
    { id: 'c', preview: 'new c' },
  ];

  assert.deepEqual(mergeHistoryPage(current, incoming), [
    { id: 'a', preview: 'old a' },
    { id: 'b', preview: 'old b' },
    { id: 'c', preview: 'new c' },
  ]);
});

test('shouldLoadNextHistoryPage only loads an available page near the bottom', () => {
  const nearBottom = {
    scrollTop: 700,
    clientHeight: 300,
    scrollHeight: 1100,
    hasNextPage: true,
    loading: false,
    threshold: 120,
  };

  assert.equal(shouldLoadNextHistoryPage(nearBottom), true);
  assert.equal(shouldLoadNextHistoryPage({ ...nearBottom, scrollTop: 500 }), false);
  assert.equal(shouldLoadNextHistoryPage({ ...nearBottom, hasNextPage: false }), false);
  assert.equal(shouldLoadNextHistoryPage({ ...nearBottom, loading: true }), false);
});
