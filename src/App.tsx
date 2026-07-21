import { useEffect, useMemo, useRef, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  Clipboard,
  Copy,
  FileText,
  GripVertical,
  Heart,
  Image,
  Pause,
  Pin,
  Play,
  Search,
  Settings,
  SlidersHorizontal,
  Trash2,
  X,
} from 'lucide-react';
import {
  filterItems,
  getTypeLabel,
  getVisibleFilters,
  getVisualPreview,
  reorderItemsByDrag,
  reorderNavFiltersByDrag,
} from './lib/clipboard-model';
import { calculateVirtualWindow, moveSelection } from './lib/history-ui';
import { applyThemeMode, getErrorMessage, mergeSettings, shouldCheckForUpdatesToday, toLocalDateString } from './lib/settings-model';
import SettingsView from './features/settings/SettingsView';
import {
  advanceImageFallback,
  beginDetailRequest,
  createImageFallbackState,
  createItemIdentity,
  getDetailDisplayContent,
  getImageFallbackPath,
  isDetailResponseCurrent,
  mapBackendCapacityStatus,
  reconcileDetailSlot,
  reconcileImageFallbackState,
  resolveDetailResponse,
  reduceCapacityStatus,
  selectDetailLoadStatus,
  type BackendClipboardCapacityStatus,
  type ClipboardCapacityStatus,
  type DetailLoadStatus,
  type DetailRequest,
  type DetailSlot,
  type ItemIdentity,
  type ViewClipboardItem,
} from './lib/clipboard-adapter';
import {
  copyItem,
  checkForUpdates,
  deleteItem as deleteBackendItem,
  getClipboardStatus,
  getItemDetail,
  getSettings,
  installUpdate,
  pasteItem,
  reorderItems,
  searchItems,
  setRecordingEnabled,
  toggleFavorite as toggleBackendFavorite,
  togglePin as toggleBackendPin,
  updateSettings,
} from './features/history/api';

type ClipboardType = 'text' | 'html' | 'image' | 'files';
type FilterType = 'all' | 'favorites' | ClipboardType;

type ClipboardItem = ViewClipboardItem;

const seedItems: ClipboardItem[] = [
  {
    id: '1',
    hash: 'seed-1',
    type: 'text',
    preview: 'SQLite WAL 模式 + FTS5 搜索，保证大量历史记录下仍能快速返回首屏。',
    contentPath: null,
    thumbnailPath: null,
    favorite: true,
    pinned: false,
    updatedAt: Date.now() - 1000 * 60 * 2,
    size: '112 B',
    source: 'VS Code',
  },
  {
    id: '2',
    hash: 'seed-2',
    type: 'image',
    preview: 'screenshot-2026-06-08.png',
    contentPath: null,
    thumbnailPath: null,
    favorite: false,
    pinned: false,
    updatedAt: Date.now() - 1000 * 60 * 18,
    size: '1.8 MB',
    source: 'Snipping Tool',
  },
  {
    id: '3',
    hash: 'seed-3',
    type: 'files',
    preview: '需求说明.docx, 架构草图.png',
    contentPath: null,
    thumbnailPath: null,
    favorite: false,
    pinned: false,
    updatedAt: Date.now() - 1000 * 60 * 42,
    size: '2 个文件',
    source: 'Explorer',
  },
  {
    id: '4',
    hash: 'seed-4',
    type: 'html',
    preview: '<table><tr><td>CopyQ / Ditto / PasteBar 功能对比</td></tr></table>',
    contentPath: null,
    thumbnailPath: null,
    favorite: false,
    pinned: false,
    updatedAt: Date.now() - 1000 * 60 * 85,
    size: '624 B',
    source: 'Edge',
  },
];

const HISTORY_ITEM_HEIGHT = 66;
const HISTORY_IMAGE_ITEM_HEIGHT = 198; // 3x normal height for images
const HISTORY_VIEWPORT_HEIGHT = 380;

function formatTime(value: number) {
  const minutes = Math.max(1, Math.round((Date.now() - value) / 60000));
  if (minutes < 60) return `${minutes} 分钟前`;
  return `${Math.round(minutes / 60)} 小时前`;
}

function iconForType(type: ClipboardType) {
  if (type === 'image') return <Image size={16} />;
  if (type === 'files') return <FileText size={16} />;
  return <Clipboard size={16} />;
}

function HistoryListImage({ thumbnailPath, contentPath }: { thumbnailPath: string | null; contentPath: string | null }) {
  const [fallbackState, setFallbackState] = useState(() => createImageFallbackState(thumbnailPath, contentPath));
  const currentFallbackState = reconcileImageFallbackState(fallbackState, thumbnailPath, contentPath);
  const imagePath = getImageFallbackPath(currentFallbackState);

  useEffect(() => {
    setFallbackState((current) => reconcileImageFallbackState(current, thumbnailPath, contentPath));
  }, [contentPath, thumbnailPath]);

  if (!imagePath) return <Image size={16} />;

  return (
    <img
      src={convertFileSrc(imagePath)}
      alt=""
      loading="lazy"
      decoding="async"
      onError={() => setFallbackState(advanceImageFallback(currentFallbackState))}
    />
  );
}

function useDebouncedValue<T>(value: T, delay: number) {
  const [debouncedValue, setDebouncedValue] = useState(value);

  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedValue(value), delay);
    return () => window.clearTimeout(timer);
  }, [delay, value]);

  return debouncedValue;
}

export default function App() {
  const [query, setQuery] = useState('');
  const [activeFilter, setActiveFilter] = useState<FilterType>('all');
  const [recording, setRecording] = useState(true);
  const [previewEnabled, setPreviewEnabled] = useState(true);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [items, setItems] = useState(seedItems);
  const [selectedId, setSelectedId] = useState<string | undefined>(seedItems[0]?.id);
  const [detailSlot, setDetailSlot] = useState<DetailSlot | null>(null);
  const [detailLoadStatus, setDetailLoadStatus] = useState<DetailLoadStatus>({
    identity: null,
    loading: false,
    error: null,
  });
  const [scrollTop, setScrollTop] = useState(0);
  const [backendAvailable, setBackendAvailable] = useState(true);
  const [statusMessage, setStatusMessage] = useState('正在连接本地剪贴板服务');
  const [capacityStatus, setCapacityStatus] = useState<ClipboardCapacityStatus>({
    blocked: false,
    message: '',
    requiredAdditional: 0,
    revision: -1,
  });
  const [refreshVersion, setRefreshVersion] = useState(0);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [navFiltersConfig, setNavFiltersConfig] = useState<{ visible: string[] }>({ visible: ['all', 'favorites', 'text', 'image', 'files'] });
  const [navConfigOpen, setNavConfigOpen] = useState(false);
  const [draggingFilterKey, setDraggingFilterKey] = useState<string | null>(null);
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; item: ClipboardItem } | null>(null);
  const [previewPaneWidth, setPreviewPaneWidth] = useState(0.47);
  const [isResizing, setIsResizing] = useState(false);
  const historyListRef = useRef<HTMLDivElement | null>(null);
  const detailRequestRef = useRef<DetailRequest>({ identity: null, generation: 0 });
  const selectedIdentityRef = useRef<ItemIdentity | null>(null);
  const debouncedQuery = useDebouncedValue(query, 100);

  // Track the latest settingsOpen for the blur handler (registered once with []).
  const settingsOpenRef = useRef(settingsOpen);
  settingsOpenRef.current = settingsOpen;

  const filters: Array<{ key: FilterType; label: string }> = [
    ...getVisibleFilters(navFiltersConfig),
  ] as Array<{ key: FilterType; label: string }>;

  const statusTone = capacityStatus.blocked || !backendAvailable
    ? 'warning'
    : recording
      ? 'connected'
      : 'paused';

  useEffect(() => {
    getSettings()
      .then((settings) => {
        const mergedSettings = mergeSettings(settings);
        applyThemeMode(mergedSettings.theme_mode);
        setRecording(mergedSettings.recording_enabled);
        setPreviewEnabled(mergedSettings.preview_enabled);
        setNavFiltersConfig(mergedSettings.nav_filters_config);
        const today = toLocalDateString(new Date());
        if (shouldCheckForUpdatesToday(settings.auto_update_enabled, settings.last_update_check_date, today)) {
          void checkForUpdates()
            .then((update) => {
              if (!update.available) {
                // Silently skip - no need to notify user on every startup
                return;
              }
              const confirmed = window.confirm(`发现新版本 ${update.version ?? ''}，是否现在更新？`);
              if (confirmed) {
                void installUpdate();
              }
            })
            .catch((error) => {
              // Silently ignore update check failures to avoid disrupting users
              // Users can manually check via Settings if needed
              const message = getErrorMessage(error, '');
              if (message) {
                console.info('Auto update check skipped:', message);
              }
            });
        }
      })
      .catch(() => {
        applyThemeMode('light');
      });
  }, []);

  useEffect(() => {
    let ignore = false;
    let unlistenCapacity: (() => void) | undefined;
    const applyCapacityStatus = (status: ClipboardCapacityStatus) => {
      setCapacityStatus((current) => reduceCapacityStatus(current, status));
    };

    async function subscribeThenQueryCapacityStatus() {
      try {
        const nextUnlisten = await listen<BackendClipboardCapacityStatus>(
          'clipboard-status',
          ({ payload }) => applyCapacityStatus(mapBackendCapacityStatus(payload)),
        );
        if (ignore) {
          nextUnlisten();
          return;
        }
        unlistenCapacity = nextUnlisten;
      } catch {
        // Query still provides the persisted startup state when event registration fails.
      }
      try {
        const queried = await getClipboardStatus();
        if (!ignore) applyCapacityStatus(queried);
      } catch {
        // Other backend calls own the connection fallback; keep the latest event/status here.
      }
    }

    void subscribeThenQueryCapacityStatus();
    return () => {
      ignore = true;
      unlistenCapacity?.();
    };
  }, []);

  useEffect(() => {
    let ignore = false;
    let unlistenClipboard: (() => void) | undefined;
    let unlistenSettings: (() => void) | undefined;

    listen('clipboard-changed', () => {
      setRefreshVersion((current) => current + 1);
    })
      .then((nextUnlisten) => {
        if (ignore) {
          nextUnlisten();
          return;
        }
        unlistenClipboard = nextUnlisten;
      })
      .catch(() => {
        setBackendAvailable(false);
      });

    listen('open-settings', () => {
      setSettingsOpen(true);
    })
      .then((nextUnlisten) => {
        if (ignore) {
          nextUnlisten();
          return;
        }
        unlistenSettings = nextUnlisten;
      })
      .catch(() => {});

    return () => {
      ignore = true;
      unlistenClipboard?.();
      unlistenSettings?.();
    };
  }, []);

  // Window blur/hide handler: clear search and close window on blur
  useEffect(() => {
    let unlistenBlur: (() => void) | undefined;

    getCurrentWindow()
      .onFocusChanged(({ payload: focused }) => {
        if (!focused && !settingsOpenRef.current) {
          // Window lost focus (and not on the Settings screen, whose native file/
          // folder dialogs also blur the window). Clear search and hide.
          setQuery('');
          void getCurrentWindow().hide();
        }
      })
      .then((unlisten) => {
        unlistenBlur = unlisten;
      })
      .catch((error) => {
        console.error('Failed to listen to window blur:', error);
      });

    return () => {
      unlistenBlur?.();
    };
  }, []);


  useEffect(() => {
    let ignore = false;

    searchItems(debouncedQuery, activeFilter)
      .then((backendItems) => {
        if (ignore) return;
        const nextItems = backendItems;
        setItems(nextItems);
        setBackendAvailable(true);
        setStatusMessage(nextItems.length === 0 ? '暂无剪贴板记录' : '已连接本地剪贴板服务');
        setScrollTop(0);
        // Keep the DOM scroll position in sync with the reset state. Without this the
        // container can stay scrolled (e.g. 800px) while the virtual window renders from
        // the top, leaving the viewport blank after the window is hidden and reopened.
        if (historyListRef.current) {
          historyListRef.current.scrollTop = 0;
        }
      })
      .catch(() => {
        if (ignore) return;
        setBackendAvailable(false);
        setStatusMessage('未连接 Tauri 后端，当前显示示例数据');
        setItems(filterItems(seedItems, { type: activeFilter, query: debouncedQuery }) as ClipboardItem[]);
      });

    return () => {
      ignore = true;
    };
  }, [activeFilter, debouncedQuery, refreshVersion]);

  const visibleItems = useMemo(() => filterItems(items, { type: 'all', query: '' }) as ClipboardItem[], [items]);

  const itemHeights = useMemo(() =>
    visibleItems.map(item => item.type === 'image' ? HISTORY_IMAGE_ITEM_HEIGHT : HISTORY_ITEM_HEIGHT),
    [visibleItems]
  );

  const cumulativeHeights = useMemo(() => {
    const heights = [0];
    for (let i = 0; i < itemHeights.length; i++) {
      heights.push(heights[i] + itemHeights[i]);
    }
    return heights;
  }, [itemHeights]);

  const totalHeight = cumulativeHeights[cumulativeHeights.length - 1] || 0;

  const virtualWindow = useMemo(() => {
    const viewportHeight = historyListRef.current?.clientHeight ?? HISTORY_VIEWPORT_HEIGHT;
    let startIndex = 0;
    let endIndex = visibleItems.length;

    // Find start index
    for (let i = 0; i < cumulativeHeights.length - 1; i++) {
      if (cumulativeHeights[i + 1] > scrollTop) {
        startIndex = Math.max(0, i - 3); // overscan
        break;
      }
    }

    // Find end index
    for (let i = startIndex; i < cumulativeHeights.length - 1; i++) {
      if (cumulativeHeights[i] >= scrollTop + viewportHeight) {
        endIndex = Math.min(visibleItems.length, i + 3); // overscan
        break;
      }
    }

    return {
      startIndex,
      endIndex,
      offsetTop: cumulativeHeights[startIndex],
    };
  }, [visibleItems.length, scrollTop, cumulativeHeights]);

  const virtualItems = visibleItems.slice(virtualWindow.startIndex, virtualWindow.endIndex);
  const selectedItem = visibleItems.find((item) => item.id === selectedId) ?? visibleItems[0];
  const selectedIdentity = useMemo(
    () => selectedItem ? createItemIdentity(selectedItem) : null,
    [selectedItem?.hash, selectedItem?.id, selectedItem?.updatedAt],
  );
  selectedIdentityRef.current = selectedIdentity;
  const selectedDetailSlot = reconcileDetailSlot(detailSlot, selectedItem);
  const selectedDetail = selectedDetailSlot?.detail;
  const selectedDetailLoadStatus = selectDetailLoadStatus(detailLoadStatus, selectedIdentity);
  const detailDisplayContent = selectedItem
    ? getDetailDisplayContent(selectedItem, selectedDetail)
    : '';
  const detailImagePath = selectedDetail?.contentPath ?? selectedItem?.contentPath ?? null;

  useEffect(() => {
    setDetailSlot((current) => reconcileDetailSlot(current, selectedItem));
  }, [selectedItem]);

  useEffect(() => {
    const selectedItemId = selectedItem?.id;
    if (!previewEnabled || !backendAvailable || !selectedItemId || !selectedIdentity) {
      detailRequestRef.current = beginDetailRequest(detailRequestRef.current, null);
      setDetailSlot(null);
      setDetailLoadStatus({ identity: null, loading: false, error: null });
      return;
    }
    if (selectedDetail) {
      setDetailLoadStatus({ identity: selectedIdentity, loading: false, error: null });
      return;
    }

    const request = beginDetailRequest(detailRequestRef.current, selectedIdentity);
    detailRequestRef.current = request;
    setDetailLoadStatus({ identity: selectedIdentity, loading: true, error: null });

    getItemDetail(selectedItemId)
      .then((detail) => {
        if (!detail) {
          if (!isDetailResponseCurrent(detailRequestRef.current, request, selectedIdentityRef.current)) return;
          setDetailLoadStatus({ identity: selectedIdentity, loading: false, error: '详情加载失败' });
          return;
        }
        const nextSlot = resolveDetailResponse(
          detailRequestRef.current,
          request,
          selectedIdentityRef.current,
          detail,
        );
        if (!nextSlot) return;
        setDetailSlot(nextSlot);
        setDetailLoadStatus({ identity: nextSlot.identity, loading: false, error: null });
      })
      .catch(() => {
        if (!isDetailResponseCurrent(detailRequestRef.current, request, selectedIdentityRef.current)) return;
        setDetailLoadStatus({ identity: selectedIdentity, loading: false, error: '详情加载失败' });
      });

    return () => {
      if (detailRequestRef.current.generation === request.generation) {
        detailRequestRef.current = beginDetailRequest(detailRequestRef.current, null);
      }
    };
  }, [backendAvailable, previewEnabled, selectedDetail, selectedIdentity, selectedItem?.id]);

  useEffect(() => {
    if (!selectedItem && visibleItems[0]) {
      setSelectedId(visibleItems[0].id);
    }
  }, [selectedItem, visibleItems]);

  async function toggleFavorite(id: string) {
    if (backendAvailable) {
      await toggleBackendFavorite(id);
    }
    setItems((current) =>
      current.map((item) => (item.id === id ? { ...item, favorite: !item.favorite } : item)),
    );
  }

  async function togglePin(id: string) {
    if (backendAvailable) {
      await toggleBackendPin(id);
    }
    setItems((current) =>
      current.map((item) => (item.id === id ? { ...item, pinned: !item.pinned } : item)),
    );
  }

  async function deleteItem(id: string) {
    if (backendAvailable) {
      await deleteBackendItem(id);
    }
    if (selectedIdentityRef.current?.itemId === id) {
      detailRequestRef.current = beginDetailRequest(detailRequestRef.current, null);
      selectedIdentityRef.current = null;
      setDetailSlot(null);
      setDetailLoadStatus({ identity: null, loading: false, error: null });
    }
    setItems((current) => current.filter((item) => item.id !== id));
  }

  async function copySelectedItem() {
    if (!selectedItem) return;
    if (backendAvailable) {
      try {
        await copyItem(selectedItem.id);
        setStatusMessage('已复制到剪贴板');
      } catch (error) {
        setStatusMessage(getErrorMessage(error, '复制失败'));
      }
      return;
    }
    await navigator.clipboard?.writeText(selectedItem.preview);
  }

  async function pasteSelectedItem() {
    if (!selectedItem) return;
    await pasteAndHideItem(selectedItem);
  }

  async function pasteAndHideItem(item: ClipboardItem, plainText?: boolean) {
    setSelectedId(item.id);
    if (backendAvailable) {
      try {
        await getCurrentWindow().hide();
        await pasteItem(item.id, plainText);
        setStatusMessage('已粘贴当前记录');
      } catch (error) {
        await getCurrentWindow().show();
        await getCurrentWindow().setFocus();
        setStatusMessage(getErrorMessage(error, '粘贴失败'));
      }
      return;
    }
    await navigator.clipboard?.writeText(item.preview);
  }

  async function pasteListItem(item: ClipboardItem) {
    await pasteAndHideItem(item);
  }

  async function handleDropItem(targetId: string) {
    if (!draggingId) return;
    const currentIds = visibleItems.map((item) => item.id);
    const nextIds = reorderItemsByDrag(currentIds, draggingId, targetId);
    setDraggingId(null);
    if (nextIds.join('\0') === currentIds.join('\0')) return;

    setItems((current) => {
      const itemById = new Map(current.map((item) => [item.id, item]));
      const visibleIdSet = new Set(currentIds);
      const reorderedVisibleItems = nextIds
        .map((id) => itemById.get(id))
        .filter((item): item is ClipboardItem => Boolean(item));
      const remainingItems = current.filter((item) => !visibleIdSet.has(item.id));
      return [...reorderedVisibleItems, ...remainingItems];
    });
    if (backendAvailable) {
      try {
        await reorderItems(nextIds);
        setStatusMessage('排序已保存');
      } catch (error) {
        setStatusMessage(getErrorMessage(error, '排序保存失败'));
        setRefreshVersion((current) => current + 1);
      }
    }
  }

  function handleKeyboard(event: React.KeyboardEvent<HTMLElement>) {
    if (event.key === 'Escape') {
      event.preventDefault();
      if (contextMenu) {
        setContextMenu(null);
      } else if (navConfigOpen) {
        setNavConfigOpen(false);
      } else if (query) {
        // Clear search query first
        setQuery('');
      } else {
        // Hide window and clear search
        setQuery('');
        void getCurrentWindow().hide();
      }
      return;
    }

    if (settingsOpen || visibleItems.length === 0) return;
    const ids = visibleItems.map((item) => item.id);

    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      const direction = event.key === 'ArrowDown' ? 'down' : 'up';
      const nextId = moveSelection(ids, selectedItem?.id, direction);
      if (nextId) {
        setSelectedId(nextId);
      }
      return;
    }

    if (event.key === 'Enter') {
      event.preventDefault();
      pasteSelectedItem();
      return;
    }

    if (event.key === 'Delete' && selectedItem) {
      event.preventDefault();
      void deleteItem(selectedItem.id);
    }
  }

  function handleResizeStart() {
    setIsResizing(true);
  }

  useEffect(() => {
    if (!isResizing) return;

    function handleMouseMove(event: MouseEvent) {
      const container = document.querySelector('.content-grid');
      if (!container) return;
      const rect = container.getBoundingClientRect();
      const offsetX = event.clientX - rect.left;
      const newWidth = Math.max(0.25, Math.min(0.75, offsetX / rect.width));
      setPreviewPaneWidth(newWidth);
    }

    function handleMouseUp() {
      setIsResizing(false);
    }

    document.addEventListener('mousemove', handleMouseMove);
    document.addEventListener('mouseup', handleMouseUp);

    return () => {
      document.removeEventListener('mousemove', handleMouseMove);
      document.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isResizing]);

  async function handleRecordingChange(nextRecording: boolean) {
    if (backendAvailable) {
      await setRecordingEnabled(nextRecording);
    }
    setRecording(nextRecording);
  }

  async function handleNavFilterToggle(filterKey: string, checked: boolean) {
    const nextVisible = checked
      ? [...navFiltersConfig.visible, filterKey]
      : navFiltersConfig.visible.filter((k) => k !== filterKey);
    const nextConfig = { visible: nextVisible };
    setNavFiltersConfig(nextConfig);

    if (backendAvailable) {
      try {
        const currentSettings = await getSettings();
        const mergedSettings = mergeSettings(currentSettings);
        const updatedSettings = { ...mergedSettings, nav_filters_config: nextConfig };
        await updateSettings(updatedSettings);
      } catch (error) {
        console.error('Failed to save nav filter config:', error);
      }
    }
  }

  async function handleNavFilterReorder(targetKey: string) {
    if (!draggingFilterKey || draggingFilterKey === targetKey) {
      setDraggingFilterKey(null);
      return;
    }

    const currentOrder = navFiltersConfig.visible;
    const nextOrder = reorderNavFiltersByDrag(currentOrder, draggingFilterKey, targetKey);
    if (nextOrder.join('\0') === currentOrder.join('\0')) {
      setDraggingFilterKey(null);
      return;
    }

    const nextConfig = { visible: nextOrder };
    setNavFiltersConfig(nextConfig);
    setDraggingFilterKey(null);

    if (backendAvailable) {
      try {
        const currentSettings = await getSettings();
        const mergedSettings = mergeSettings(currentSettings);
        const updatedSettings = { ...mergedSettings, nav_filters_config: nextConfig };
        await updateSettings(updatedSettings);
      } catch (error) {
        console.error('Failed to save nav filter order:', error);
      }
    }
  }

  if (settingsOpen) {
    return (
      <SettingsView
        onBack={() => setSettingsOpen(false)}
        recording={recording}
        onRecordingChange={(value) => void handleRecordingChange(value)}
        onRecordingLoaded={setRecording}
        onSettingsChanged={(settings) => {
          setRecording(settings.recording_enabled);
          setPreviewEnabled(settings.preview_enabled);
          setNavFiltersConfig(settings.nav_filters_config);
        }}
        onHistoryCleared={() => {
          detailRequestRef.current = beginDetailRequest(detailRequestRef.current, null);
          selectedIdentityRef.current = null;
          setDetailSlot(null);
          setDetailLoadStatus({ identity: null, loading: false, error: null });
          setItems([]);
          setSelectedId(undefined);
          setRefreshVersion((current) => current + 1);
        }}
      />
    );
  }

  return (
    <main
      className="app-shell"
      onKeyDown={handleKeyboard}
      onContextMenu={(e) => {
        e.preventDefault();
        setContextMenu(null);
      }}
      onClick={() => setContextMenu(null)}
    >
      <section className="toolbar">
        <div className="title-group">
          <div className="app-mark">
            <Clipboard size={18} />
          </div>
          <div>
            <h1>super-clipboard</h1>
            <div className={`app-status ${statusTone}`}>
              <span className="status-dot" aria-hidden="true" />
              <p role="status" aria-live="polite">
                {capacityStatus.blocked
                  ? capacityStatus.message
                  : recording
                    ? statusMessage
                    : '已暂停记录'}
              </p>
            </div>
          </div>
        </div>
        <div className="toolbar-actions">
          <button
            className="icon-button"
            title={recording ? '暂停记录' : '恢复记录'}
            aria-label={recording ? '暂停记录' : '恢复记录'}
            onClick={() => void handleRecordingChange(!recording)}
          >
            {recording ? <Pause size={17} /> : <Play size={17} />}
          </button>
          <button className="icon-button" title="设置" aria-label="打开设置" onClick={() => setSettingsOpen(true)}>
            <Settings size={17} />
          </button>
        </div>
      </section>

      <section className="search-row">
        <Search size={18} />
        <input
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="搜索文本、文件名或来源"
          aria-label="搜索剪贴板记录"
          autoFocus
        />
      </section>

      <section className="filter-row" aria-label="剪贴板类型过滤">
        {filters.map((filter) => (
          <button
            key={filter.key}
            className={[
              activeFilter === filter.key ? 'filter-chip active' : 'filter-chip',
              draggingFilterKey === filter.key ? 'dragging' : '',
            ].join(' ')}
            draggable={filter.key !== 'all'}
            onDragStart={(event) => {
              if (filter.key === 'all') {
                event.preventDefault();
                return;
              }
              setDraggingFilterKey(filter.key);
              event.dataTransfer.effectAllowed = 'move';
              event.dataTransfer.setData('text/plain', filter.key);
            }}
            onDragOver={(event) => {
              if (draggingFilterKey && draggingFilterKey !== filter.key) {
                event.preventDefault();
                event.dataTransfer.dropEffect = 'move';
              }
            }}
            onDrop={(event) => {
              event.preventDefault();
              if (draggingFilterKey && draggingFilterKey !== filter.key) {
                void handleNavFilterReorder(filter.key);
              }
            }}
            onDragEnd={() => setDraggingFilterKey(null)}
            onClick={() => setActiveFilter(filter.key)}
          >
            {filter.label}
          </button>
        ))}
        <button
          className="filter-chip nav-config-btn"
          title="配置导航过滤器"
          aria-label="配置导航过滤器"
          onClick={() => setNavConfigOpen(!navConfigOpen)}
        >
          <SlidersHorizontal size={16} />
        </button>
      </section>

      {navConfigOpen && (
        <div className="nav-config-overlay" onClick={() => setNavConfigOpen(false)}>
          <div className="nav-config-panel" onClick={(e) => e.stopPropagation()}>
            <div className="nav-config-header">
              <h2>导航过滤器设置</h2>
              <button
                className="icon-button"
                title="关闭"
                aria-label="关闭导航过滤器设置"
                onClick={() => setNavConfigOpen(false)}
              >
                <X size={16} />
              </button>
            </div>
            <p className="nav-config-hint">拖动调整顺序，取消勾选隐藏（全部不可隐藏）</p>
            <div className="nav-config-list">
              {[
                { key: 'all', label: '全部' },
                { key: 'favorites', label: '收藏' },
                { key: 'text', label: '文本' },
                { key: 'image', label: '图片' },
                { key: 'files', label: '文件' },
              ]
                .filter((f) => navFiltersConfig.visible.includes(f.key))
                .sort((a, b) => navFiltersConfig.visible.indexOf(a.key) - navFiltersConfig.visible.indexOf(b.key))
                .map((filter) => {
                  const isVisible = navFiltersConfig.visible.includes(filter.key);
                  const isAll = filter.key === 'all';
                  return (
                    <div
                      key={filter.key}
                      className={`nav-config-item ${draggingFilterKey === filter.key ? 'dragging' : ''}`}
                      draggable={!isAll}
                      onDragStart={(e) => {
                        if (!isAll) {
                          setDraggingFilterKey(filter.key);
                          e.dataTransfer.effectAllowed = 'move';
                          e.dataTransfer.setData('text/plain', filter.key);
                        } else {
                          e.preventDefault();
                        }
                      }}
                      onDragOver={(e) => {
                        if (draggingFilterKey && draggingFilterKey !== filter.key) {
                          e.preventDefault();
                          e.dataTransfer.dropEffect = 'move';
                        }
                      }}
                      onDrop={(e) => {
                        e.preventDefault();
                        if (draggingFilterKey && draggingFilterKey !== filter.key) {
                          void handleNavFilterReorder(filter.key);
                        }
                      }}
                      onDragEnd={() => setDraggingFilterKey(null)}
                    >
                      <span className={`drag-handle ${isAll ? 'disabled' : ''}`} aria-hidden="true">
                        <GripVertical size={16} />
                      </span>
                      <label>
                        <input
                          type="checkbox"
                          checked={isVisible}
                          disabled={isAll}
                          onChange={(e) => void handleNavFilterToggle(filter.key, e.target.checked)}
                        />
                        <span style={{ opacity: isAll ? 0.6 : 1 }}>{filter.label}</span>
                      </label>
                    </div>
                  );
                })}
              {[
                { key: 'all', label: '全部' },
                { key: 'favorites', label: '收藏' },
                { key: 'text', label: '文本' },
                { key: 'image', label: '图片' },
                { key: 'files', label: '文件' },
              ]
                .filter((f) => !navFiltersConfig.visible.includes(f.key))
                .map((filter) => (
                  <div key={filter.key} className="nav-config-item not-draggable">
                    <span className="drag-handle" style={{ opacity: 0.3 }} aria-hidden="true">
                      <GripVertical size={16} />
                    </span>
                    <label>
                      <input
                        type="checkbox"
                        checked={false}
                        onChange={(e) => void handleNavFilterToggle(filter.key, e.target.checked)}
                      />
                      <span style={{ opacity: 0.5 }}>{filter.label}</span>
                    </label>
                  </div>
                ))}
            </div>
          </div>
        </div>
      )}

      {contextMenu && (
        <div
          className="context-menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onClick={(e) => e.stopPropagation()}
        >
          <button onClick={async () => {
            await copySelectedItem();
            setContextMenu(null);
          }}>
            <Copy size={15} />
            复制
          </button>
          <button onClick={async () => {
            await pasteAndHideItem(contextMenu.item);
            setContextMenu(null);
          }}>
            <Pin size={15} />
            粘贴
          </button>
          {contextMenu.item.type === 'html' ? (
            <button onClick={async () => {
              await pasteAndHideItem(contextMenu.item, true);
              setContextMenu(null);
            }}>
              <FileText size={15} />
              以纯文本粘贴
            </button>
          ) : null}
          <button onClick={async () => {
            await toggleFavorite(contextMenu.item.id);
            setContextMenu(null);
          }}>
            <Heart size={15} fill={contextMenu.item.favorite ? 'currentColor' : 'none'} />
            {contextMenu.item.favorite ? '取消收藏' : '收藏'}
          </button>
          <button onClick={async () => {
            await togglePin(contextMenu.item.id);
            setContextMenu(null);
          }}>
            <Pin size={15} fill={contextMenu.item.pinned ? 'currentColor' : 'none'} />
            {contextMenu.item.pinned ? '取消置顶' : '置顶'}
          </button>
          <div className="context-menu-divider" />
          <button className="danger" onClick={async () => {
            await deleteItem(contextMenu.item.id);
            setContextMenu(null);
          }}>
            <Trash2 size={15} />
            删除
          </button>
        </div>
      )}

      <section
        className={previewEnabled ? 'content-grid' : 'content-grid no-preview'}
        style={previewEnabled ? {
          gridTemplateColumns: `minmax(300px, ${(1 - previewPaneWidth) * 100}%) 4px minmax(360px, ${previewPaneWidth * 100}%)`
        } : undefined}
      >
        <div
          ref={historyListRef}
          className="history-list"
          onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        >
          <div className="history-spacer" style={{ height: totalHeight }}>
            <div style={{ transform: `translateY(${virtualWindow.offsetTop}px)` }}>
          {virtualItems.map((item, idx) => {
            const actualIndex = virtualWindow.startIndex + idx;
            const itemHeight = itemHeights[actualIndex];
            const isDraggingDisabled = false;
            const hasImageListPath = item.type === 'image' && Boolean(item.thumbnailPath ?? item.contentPath);
            return (
            <button
              key={item.id}
              className={[
                selectedItem?.id === item.id ? 'history-item selected' : 'history-item',
                draggingId === item.id ? 'dragging' : '',
                item.type === 'image' ? 'image-row' : '',
              ].join(' ')}
              style={{ minHeight: `${itemHeight}px` }}
              draggable={!isDraggingDisabled}
              onDragStart={(e) => {
                if (!isDraggingDisabled) {
                  setDraggingId(item.id);
                  // Add visual feedback
                  e.dataTransfer.effectAllowed = 'move';
                  e.dataTransfer.setData('text/plain', item.id);
                }
              }}
              onDragOver={(event) => {
                if (!isDraggingDisabled) {
                  event.preventDefault();
                  event.dataTransfer.dropEffect = 'move';
                }
              }}
              onDrop={(event) => {
                if (!isDraggingDisabled) {
                  event.preventDefault();
                  void handleDropItem(item.id);
                }
              }}
              onDragEnd={() => setDraggingId(null)}
              onClick={(e) => {
                // Only trigger click if not dragging
                if (!draggingId) {
                  void pasteListItem(item);
                }
              }}
              onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                setContextMenu({ x: e.clientX, y: e.clientY, item });
              }}
            >
              <span className="drag-handle-icon">
                <GripVertical size={16} />
              </span>
              <span className={hasImageListPath ? 'type-icon image-thumb' : 'type-icon'}>
                {item.type === 'image' ? (
                  <HistoryListImage
                    thumbnailPath={item.thumbnailPath}
                    contentPath={item.contentPath}
                  />
                ) : (
                  iconForType(item.type)
                )}
              </span>
              <span className="item-main">
                <span className="item-preview">{getVisualPreview(item)}</span>
                <span className="item-meta">
                  {[item.source, formatTime(item.updatedAt), item.size].filter(Boolean).join(' · ')}
                </span>
              </span>
              <span className="item-indicators">
                {item.pinned ? <Pin className="pinned-icon" size={15} fill="currentColor" /> : null}
                {item.favorite ? <Heart className="favorite" size={15} fill="currentColor" /> : null}
              </span>
            </button>
          );
          })}
            </div>
          </div>
          {visibleItems.length === 0 ? (
            <div className="empty-state">
              <span className="empty-state-icon" aria-hidden="true"><Clipboard size={20} /></span>
              <strong>没有匹配的剪贴板记录</strong>
              <p>尝试调整关键词或筛选条件</p>
            </div>
          ) : null}
        </div>

        {previewEnabled ? (
          <>
            <div
              className={`resize-handle ${isResizing ? 'resizing' : ''}`}
              onMouseDown={handleResizeStart}
            />
            <aside className="detail-pane">
            {selectedItem ? (
              <>
                <div className="detail-header">
                  <div className="detail-heading">
                    <span className="detail-overline">当前记录</span>
                    <strong>内容详情</strong>
                  </div>
                  <div className="detail-header-meta">
                    <span className="detail-type">{getTypeLabel(selectedItem.type)}</span>
                    <span role="status" aria-live="polite">
                    {selectedDetailLoadStatus.loading
                      ? '正在加载完整内容…'
                      : selectedDetailLoadStatus.error ?? selectedItem.size}
                    </span>
                  </div>
                </div>
                {selectedItem.type === 'image' && detailImagePath ? (
                  <div className="image-preview">
                    <img src={convertFileSrc(detailImagePath)} alt="剪贴板图片预览" />
                  </div>
                ) : (
                  <pre
                    className={selectedDetailLoadStatus.loading ? 'detail-content is-loading' : 'detail-content'}
                    aria-busy={selectedDetailLoadStatus.loading}
                  >
                    {detailDisplayContent}
                  </pre>
                )}
                <div className="detail-actions">
                  <button className="primary-action" onClick={() => void pasteSelectedItem()}>
                    <Pin size={16} />
                    粘贴
                  </button>
                  <button onClick={() => void copySelectedItem()}>
                    <Copy size={16} />
                    仅复制
                  </button>
                  <button onClick={() => void togglePin(selectedItem.id)}>
                    <Pin size={16} />
                    {selectedItem.pinned ? '取消置顶' : '置顶'}
                  </button>
                  <button onClick={() => void toggleFavorite(selectedItem.id)}>
                    <Heart size={16} />
                    {selectedItem.favorite ? '取消收藏' : '收藏'}
                  </button>
                  <button className="danger" onClick={() => void deleteItem(selectedItem.id)}>
                    <Trash2 size={16} />
                    删除
                  </button>
                </div>
              </>
            ) : (
              <div className="empty-state">
                <span className="empty-state-icon" aria-hidden="true"><Clipboard size={20} /></span>
                <strong>选择一条记录</strong>
                <p>选中后可预览并快速粘贴</p>
              </div>
            )}
          </aside>
          </>
        ) : null}
      </section>
    </main>
  );
}
