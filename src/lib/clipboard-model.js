export function sortItemsByUpdatedTime(items) {
  return [...items].sort((a, b) => b.updatedAt - a.updatedAt);
}

export function filterItems(items, { type, query }) {
  const normalizedQuery = query.trim().toLowerCase();

  return sortItemsByUpdatedTime(items).filter((item) => {
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
    return '图片内容';
  }
  return normalizePreview(item.preview, 88);
}

export function reorderItemsByDrag(ids, draggedId, targetId) {
  if (!draggedId || !targetId || draggedId === targetId) {
    return ids;
  }

  const nextIds = ids.filter((id) => id !== draggedId);
  const targetIndex = nextIds.indexOf(targetId);
  if (targetIndex === -1 || ids.indexOf(draggedId) === -1) {
    return ids;
  }

  nextIds.splice(targetIndex, 0, draggedId);
  return nextIds;
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
