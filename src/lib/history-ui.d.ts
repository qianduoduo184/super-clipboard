export type VirtualWindowInput = {
  itemCount: number;
  scrollTop: number;
  itemHeight: number;
  viewportHeight: number;
  overscan?: number;
};

export type VirtualWindow = {
  startIndex: number;
  endIndex: number;
  offsetTop: number;
};

export function calculateVirtualWindow(input: VirtualWindowInput): VirtualWindow;

export function moveSelection(
  ids: string[],
  currentId: string | undefined,
  direction: 'up' | 'down',
): string | undefined;
