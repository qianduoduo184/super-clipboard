export type ClipboardItemLike = {
  id: string;
  type: string;
  preview: string;
  favorite: boolean;
  pinned: boolean;
  updatedAt: number;
};

export function sortItemsByUpdatedTime<T extends ClipboardItemLike>(items: T[]): T[];

export function filterItems<T extends ClipboardItemLike>(
  items: T[],
  options: { type: string; query: string },
): T[];

export function normalizePreview(value: string, maxLength?: number): string;

export function getVisibleFilters(config?: { visible: string[] }): Array<{ key: string; label: string }>;

export function getVisualPreview(item: Pick<ClipboardItemLike, 'type' | 'preview'>): string;

export function reorderItemsByDrag(ids: string[], draggedId: string, targetId: string): string[];

export function reorderNavFiltersByDrag(keys: string[], draggedKey: string, targetKey: string): string[];

export function getTypeLabel(type: string): string;
