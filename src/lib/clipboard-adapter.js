const SUPPORTED_TYPES = new Set(['text', 'html', 'image', 'files']);

export function mapBackendCapacityStatus(status) {
  const blocked = status?.blocked === true;
  return {
    blocked,
    message: blocked ? String(status?.message ?? '') : '',
    revision: Number.isSafeInteger(status?.revision) ? status.revision : 0,
  };
}

export function reduceCapacityStatus(current, incoming) {
  return incoming.revision > current.revision ? incoming : current;
}

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
    hash: item.hash,
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

export function createItemIdentity(item) {
  return {
    itemId: item.id,
    itemHash: item.hash,
    updatedAt: item.updatedAt,
  };
}

function isSameItemIdentity(left, right) {
  return (
    left?.itemId === right?.itemId &&
    left?.itemHash === right?.itemHash &&
    left?.updatedAt === right?.updatedAt
  );
}

export function reconcileDetailSlot(slot, selectedItem) {
  if (!slot || !selectedItem) return null;
  return isSameItemIdentity(slot.identity, createItemIdentity(selectedItem)) ? slot : null;
}

export function beginDetailRequest(currentRequest, identity) {
  return {
    identity,
    generation: currentRequest.generation + 1,
  };
}

export function isDetailResponseCurrent(activeRequest, completedRequest, selectedIdentity) {
  return (
    activeRequest.generation === completedRequest.generation &&
    isSameItemIdentity(activeRequest.identity, completedRequest.identity) &&
    isSameItemIdentity(completedRequest.identity, selectedIdentity)
  );
}

export function resolveDetailResponse(activeRequest, completedRequest, selectedIdentity, detail) {
  if (!isDetailResponseCurrent(activeRequest, completedRequest, selectedIdentity)) return null;
  if (!isSameItemIdentity(completedRequest.identity, createItemIdentity(detail))) return null;
  return { identity: selectedIdentity, detail };
}

export function getDetailDisplayContent(summary, detail) {
  if (detail?.id === summary.id && detail.content !== null && detail.content !== undefined) {
    return detail.content;
  }
  return summary.preview;
}

export function createImageFallbackState(thumbnailPath, contentPath) {
  return {
    thumbnailPath,
    contentPath,
    stage: thumbnailPath ? 'thumbnail' : contentPath ? 'original' : 'none',
  };
}

export function getImageFallbackPath(state) {
  if (state.stage === 'thumbnail') return state.thumbnailPath;
  if (state.stage === 'original') return state.contentPath;
  return null;
}

export function advanceImageFallback(state) {
  if (state.stage === 'thumbnail') {
    return { ...state, stage: state.contentPath ? 'original' : 'none' };
  }
  if (state.stage === 'original') {
    return { ...state, stage: 'none' };
  }
  return state;
}

export function reconcileImageFallbackState(state, thumbnailPath, contentPath) {
  if (state.thumbnailPath === thumbnailPath && state.contentPath === contentPath) return state;
  return createImageFallbackState(thumbnailPath, contentPath);
}

export function selectDetailLoadStatus(status, selectedIdentity) {
  if (!isSameItemIdentity(status.identity, selectedIdentity)) {
    return { loading: false, error: null };
  }
  return { loading: status.loading, error: status.error };
}
