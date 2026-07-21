export function calculateVirtualWindow({
  itemCount,
  scrollTop,
  itemHeight,
  viewportHeight,
  overscan = 2,
}) {
  if (itemCount <= 0) {
    return { startIndex: 0, endIndex: 0, offsetTop: 0 };
  }

  const firstVisible = Math.floor(scrollTop / itemHeight);
  const visibleCount = Math.ceil(viewportHeight / itemHeight);
  const startIndex = Math.max(0, firstVisible - overscan);
  const endIndex = Math.min(itemCount, firstVisible + visibleCount + overscan);

  return {
    startIndex,
    endIndex,
    offsetTop: startIndex * itemHeight,
  };
}

export function moveSelection(ids, currentId, direction) {
  if (ids.length === 0) {
    return undefined;
  }

  const currentIndex = ids.indexOf(currentId);
  if (currentIndex === -1) {
    return ids[0];
  }

  const delta = direction === 'down' ? 1 : -1;
  const nextIndex = Math.min(ids.length - 1, Math.max(0, currentIndex + delta));
  return ids[nextIndex];
}

export function mergeHistoryPage(current, incoming) {
  const seen = new Set(current.map((item) => item.id));
  return current.concat(incoming.filter((item) => !seen.has(item.id)));
}

export function shouldLoadNextHistoryPage({
  scrollTop,
  clientHeight,
  scrollHeight,
  hasNextPage,
  loading,
  threshold = 200,
}) {
  if (!hasNextPage || loading) return false;
  return scrollTop + clientHeight >= scrollHeight - threshold;
}
