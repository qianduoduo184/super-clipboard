export type BackendClipboardItem = {
  id: string;
  hash: string;
  item_type: string;
  content?: string | null;
  content_path?: string | null;
  preview: string;
  source_app?: string | null;
  favorite: boolean;
  pinned: boolean;
  size_bytes: number;
  created_at: number;
  updated_at: number;
};

export type ViewClipboardItem = {
  id: string;
  type: 'text' | 'html' | 'image' | 'files';
  preview: string;
  contentPath: string | null;
  favorite: boolean;
  pinned: boolean;
  updatedAt: number;
  size: string;
  source: string;
};

export function formatBytes(value: number): string;

export function mapBackendItemToViewItem(item: BackendClipboardItem): ViewClipboardItem;
