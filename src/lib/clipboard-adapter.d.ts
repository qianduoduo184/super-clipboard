export type BackendClipboardItemSummary = {
  id: string;
  hash: string;
  item_type: string;
  content_path?: string | null;
  thumbnail_path?: string | null;
  preview: string;
  source_app?: string | null;
  favorite: boolean;
  pinned: boolean;
  size_bytes: number;
  created_at: number;
  updated_at: number;
};

export type BackendClipboardItemDetail = BackendClipboardItemSummary & {
  content?: string | null;
};

export type BackendClipboardItem = BackendClipboardItemDetail;

export type ViewClipboardItem = {
  id: string;
  type: 'text' | 'html' | 'image' | 'files';
  preview: string;
  contentPath: string | null;
  thumbnailPath: string | null;
  favorite: boolean;
  pinned: boolean;
  updatedAt: number;
  size: string;
  source: string;
};

export type ViewClipboardItemDetail = ViewClipboardItem & {
  content: string | null;
};

export type DetailRequest = {
  itemId: string | null;
  generation: number;
};

export function formatBytes(value: number): string;

export function mapBackendItemToViewItem(item: BackendClipboardItemSummary): ViewClipboardItem;

export function mapBackendItemDetailToViewItem(item: BackendClipboardItemDetail): ViewClipboardItemDetail;

export function cacheItemDetailById(
  detailsById: Record<string, ViewClipboardItemDetail>,
  detail: ViewClipboardItemDetail,
): Record<string, ViewClipboardItemDetail>;

export function beginDetailRequest(currentRequest: DetailRequest, itemId: string | null): DetailRequest;

export function isDetailResponseCurrent(
  activeRequest: DetailRequest,
  completedRequest: DetailRequest,
  selectedId: string | null | undefined,
): boolean;

export function getDetailDisplayContent(
  summary: Pick<ViewClipboardItem, 'id' | 'preview'>,
  detail?: Pick<ViewClipboardItemDetail, 'id' | 'content'>,
): string;
