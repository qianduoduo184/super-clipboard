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

export function getTypeLabel(type) {
  const labels = {
    text: '文本',
    html: 'HTML',
    image: '图片',
    files: '文件',
  };
  return labels[type] ?? '未知';
}
