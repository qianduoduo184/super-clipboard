export type ClipboardItemLike = {
  id: string;
  type: string;
  preview: string;
  favorite: boolean;
  updatedAt: number;
};

export function sortItemsByUpdatedTime<T extends ClipboardItemLike>(items: T[]): T[];

export function filterItems<T extends ClipboardItemLike>(
  items: T[],
  options: { type: string; query: string },
): T[];

export function normalizePreview(value: string, maxLength?: number): string;

export function getTypeLabel(type: string): string;
