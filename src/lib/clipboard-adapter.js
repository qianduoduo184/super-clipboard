const SUPPORTED_TYPES = new Set(['text', 'html', 'image', 'files']);

export function formatBytes(value) {
  if (value < 1024) {
    return `${value} B`;
  }
  if (value < 1024 * 1024) {
    return `${(value / 1024).toFixed(1)} KB`;
  }
  return `${(value / 1024 / 1024).toFixed(1)} MB`;
}

export function mapBackendItemToViewItem(item) {
  const type = SUPPORTED_TYPES.has(item.item_type) ? item.item_type : 'text';

  return {
    id: item.id,
    type,
    preview: item.preview || '(空内容)',
    contentPath: item.content_path ?? null,
    favorite: item.favorite,
    pinned: item.pinned || false,
    updatedAt: Math.floor(item.updated_at / 1000),
    size: type === 'files' ? `${item.size_bytes} 个文件` : formatBytes(item.size_bytes),
    source: item.source_app || '未知来源',
  };
}
