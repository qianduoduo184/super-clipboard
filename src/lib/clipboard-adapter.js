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
    thumbnailPath: item.thumbnail_path ?? null,
    favorite: item.favorite,
    pinned: item.pinned || false,
    updatedAt: Math.floor(item.updated_at / 1000),
    size: type === 'files' ? `${item.size_bytes} 个文件` : formatBytes(item.size_bytes),
    source: item.source_app || '未知来源',
  };
}

export function mapBackendItemDetailToViewItem(item) {
  return {
    ...mapBackendItemToViewItem(item),
    content: item.content ?? null,
  };
}

export function cacheItemDetailById(detailsById, detail) {
  return {
    ...detailsById,
    [detail.id]: detail,
  };
}

export function beginDetailRequest(currentRequest, itemId) {
  return {
    itemId,
    generation: currentRequest.generation + 1,
  };
}

export function isDetailResponseCurrent(activeRequest, completedRequest, selectedId) {
  return (
    activeRequest.generation === completedRequest.generation &&
    activeRequest.itemId === completedRequest.itemId &&
    completedRequest.itemId === selectedId
  );
}

export function getDetailDisplayContent(summary, detail) {
  if (detail?.id === summary.id && detail.content !== null && detail.content !== undefined) {
    return detail.content;
  }
  return summary.preview;
}
