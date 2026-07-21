export function sortItemsByUpdatedTime(items) {
  return [...items].sort((a, b) => {
    // Pinned items always come first
    if (a.pinned !== b.pinned) {
      return a.pinned ? -1 : 1;
    }
    // Within pinned or unpinned groups, sort by updatedAt
    return b.updatedAt - a.updatedAt;
  });
}

export function filterItems(items, { type, query }) {
  const normalizedQuery = query.trim().toLowerCase();

  return items.filter((item) => {
    const matchesType =
      type === 'all' ||
      (type === 'favorites' && item.favorite) ||
      (type !== 'favorites' && item.type === type);

    if (!matchesType) {
      return false;
    }

    if (!normalizedQuery) {
      return true;
    }

    return item.preview.toLowerCase().includes(normalizedQuery);
  });
}

export function normalizePreview(value, maxLength = 120) {
  // Collapse all whitespace (including \n, \r, \t) into single spaces for single-line display
  // The original multi-line content is preserved in storage and restored on paste
  const singleLine = value.replace(/\s+/g, ' ').trim();
  if (singleLine.length <= maxLength) {
    return singleLine;
  }
  return `${singleLine.slice(0, maxLength - 1)}...`;
}

export function getVisibleFilters(config) {
  const allFilters = [
    { key: 'all', label: '全部' },
    { key: 'favorites', label: '收藏' },
    { key: 'text', label: '文本' },
    { key: 'image', label: '图片' },
    { key: 'files', label: '文件' },
  ];

  if (!config || !config.visible || config.visible.length === 0) {
    return allFilters;
  }

  return config.visible
    .map((key) => allFilters.find((f) => f.key === key))
    .filter(Boolean);
}

export function getVisualPreview(item) {
  if (item.type === 'image') {
    return '';
  }
  return normalizePreview(item.preview, 88);
}

export function reorderItemsByDrag(ids, draggedId, targetId) {
  if (!draggedId || !targetId || draggedId === targetId) {
    return ids;
  }

  const draggedIndex = ids.indexOf(draggedId);
  const targetIndex = ids.indexOf(targetId);

  if (draggedIndex === -1 || targetIndex === -1) {
    return ids;
  }

  // Remove dragged item
  const nextIds = ids.filter((id) => id !== draggedId);

  // Insert after target (not before)
  const newTargetIndex = nextIds.indexOf(targetId);
  nextIds.splice(newTargetIndex + 1, 0, draggedId);

  return nextIds;
}

export function reorderNavFiltersByDrag(keys, draggedKey, targetKey) {
  if (!draggedKey || !targetKey || draggedKey === 'all' || draggedKey === targetKey) {
    return keys;
  }
  if (!keys.includes(draggedKey) || !keys.includes(targetKey)) {
    return keys;
  }

  const nextKeys = keys.filter((key) => key !== draggedKey);
  const targetIndex = nextKeys.indexOf(targetKey);
  nextKeys.splice(targetIndex + 1, 0, draggedKey);
  return nextKeys;
}

export function getTypeLabel(type) {
  const labels = {
    text: '文本',
    html: 'HTML',
    image: '图片',
    files: '文件',
  };
  return labels[type] ?? '未知';
}
