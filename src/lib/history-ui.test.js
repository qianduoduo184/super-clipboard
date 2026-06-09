import test from 'node:test';
import assert from 'node:assert/strict';
import { calculateVirtualWindow, moveSelection } from './history-ui.js';

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
