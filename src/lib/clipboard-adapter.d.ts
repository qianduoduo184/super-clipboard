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

export type BackendClipboardCapacityStatus = {
  blocked: boolean;
  message: string;
  required_additional: number;
  revision: number;
};

export type ClipboardCapacityStatus = {
  blocked: boolean;
  message: string;
  requiredAdditional: number;
  revision: number;
};

export type ViewClipboardItem = {
  id: string;
  hash: string;
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

export type ItemIdentity = {
  itemId: string;
  itemHash: string;
  updatedAt: number;
};

export type DetailSlot = {
  identity: ItemIdentity;
  detail: ViewClipboardItemDetail;
};

export type DetailRequest = {
  identity: ItemIdentity | null;
  generation: number;
};

export type DetailLoadStatus = {
  identity: ItemIdentity | null;
  loading: boolean;
  error: string | null;
};

export type ImageFallbackState = {
  thumbnailPath: string | null;
  contentPath: string | null;
  stage: 'thumbnail' | 'original' | 'none';
};

export function formatBytes(value: number): string;

export function mapBackendCapacityStatus(
  status: BackendClipboardCapacityStatus,
): ClipboardCapacityStatus;

export function reduceCapacityStatus(
  current: ClipboardCapacityStatus,
  incoming: ClipboardCapacityStatus,
): ClipboardCapacityStatus;

export function mapBackendItemToViewItem(item: BackendClipboardItemSummary): ViewClipboardItem;

export function mapBackendItemDetailToViewItem(item: BackendClipboardItemDetail): ViewClipboardItemDetail;

export function createItemIdentity(
  item: Pick<ViewClipboardItem, 'id' | 'hash' | 'updatedAt'>,
): ItemIdentity;

export function reconcileDetailSlot(
  slot: DetailSlot | null,
  selectedItem?: Pick<ViewClipboardItem, 'id' | 'hash' | 'updatedAt'> | null,
): DetailSlot | null;

export function beginDetailRequest(
  currentRequest: DetailRequest,
  identity: ItemIdentity | null,
): DetailRequest;

export function isDetailResponseCurrent(
  activeRequest: DetailRequest,
  completedRequest: DetailRequest,
  selectedIdentity: ItemIdentity | null | undefined,
): boolean;

export function resolveDetailResponse(
  activeRequest: DetailRequest,
  completedRequest: DetailRequest,
  selectedIdentity: ItemIdentity | null | undefined,
  detail: ViewClipboardItemDetail,
): DetailSlot | null;

export function getDetailDisplayContent(
  summary: Pick<ViewClipboardItem, 'id' | 'preview'>,
  detail?: Pick<ViewClipboardItemDetail, 'id' | 'content'>,
): string;

export function createImageFallbackState(
  thumbnailPath: string | null,
  contentPath: string | null,
): ImageFallbackState;

export function getImageFallbackPath(state: ImageFallbackState): string | null;

export function advanceImageFallback(state: ImageFallbackState): ImageFallbackState;

export function reconcileImageFallbackState(
  state: ImageFallbackState,
  thumbnailPath: string | null,
  contentPath: string | null,
): ImageFallbackState;

export function selectDetailLoadStatus(
  status: DetailLoadStatus,
  selectedIdentity: ItemIdentity | null | undefined,
): Pick<DetailLoadStatus, 'loading' | 'error'>;
