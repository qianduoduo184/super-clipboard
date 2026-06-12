import { useEffect, useMemo, useRef, useState } from 'react';
import { convertFileSrc } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  Clipboard,
  Copy,
  FileText,
  Heart,
  Image,
  Pause,
  Pin,
  Play,
  Search,
  Settings,
  Trash2,
} from 'lucide-react';
import { filterItems, getTypeLabel, getVisibleFilters, getVisualPreview, reorderItemsByDrag } from './lib/clipboard-model';
import { calculateVirtualWindow, moveSelection } from './lib/history-ui';
import { applyThemeMode, getErrorMessage, mergeSettings, shouldCheckForUpdatesToday, toLocalDateString } from './lib/settings-model';
import SettingsView from './features/settings/SettingsView';
import { mapBackendItemToViewItem } from './lib/clipboard-adapter';
import {
  copyItem,
  checkForUpdates,
  deleteItem as deleteBackendItem,
  getSettings,
  installUpdate,
  pasteItem,
  reorderItems,
  searchItems,
  setRecordingEnabled,
  toggleFavorite as toggleBackendFavorite,
} from './features/history/api';

type ClipboardType = 'text' | 'html' | 'image' | 'files';
type FilterType = 'all' | 'favorites' | ClipboardType;

type ClipboardItem = {
  id: string;
  type: ClipboardType;
  preview: string;
  contentPath: string | null;
  favorite: boolean;
  updatedAt: number;
  size: string;
  source: string;
};

const seedItems: ClipboardItem[] = [
  {
    id: '1',
    type: 'text',
    preview: 'SQLite WAL 模式 + FTS5 搜索，保证大量历史记录下仍能快速返回首屏。',
    contentPath: null,
    favorite: true,
    updatedAt: Date.now() - 1000 * 60 * 2,
    size: '112 B',
    source: 'VS Code',
  },
  {
    id: '2',
    type: 'image',
    preview: 'screenshot-2026-06-08.png',
    contentPath: null,
    favorite: false,
    updatedAt: Date.now() - 1000 * 60 * 18,
    size: '1.8 MB',
    source: 'Snipping Tool',
  },
  {
    id: '3',
    type: 'files',
    preview: '需求说明.docx, 架构草图.png',
    contentPath: null,
    favorite: false,
    updatedAt: Date.now() - 1000 * 60 * 42,
    size: '2 个文件',
    source: 'Explorer',
  },
  {
    id: '4',
    type: 'html',
    preview: '<table><tr><td>CopyQ / Ditto / PasteBar 功能对比</td></tr></table>',
    contentPath: null,
    favorite: false,
    updatedAt: Date.now() - 1000 * 60 * 85,
    size: '624 B',
    source: 'Edge',
  },
];

const HISTORY_ITEM_HEIGHT = 66;
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
  const [scrollTop, setScrollTop] = useState(0);
  const [backendAvailable, setBackendAvailable] = useState(true);
  const [statusMessage, setStatusMessage] = useState('正在连接本地剪贴板服务');
  const [refreshVersion, setRefreshVersion] = useState(0);
  const [draggingId, setDraggingId] = useState<string | null>(null);
  const [navFiltersConfig, setNavFiltersConfig] = useState<{ visible: string[] }>({ visible: ['all', 'favorites', 'text', 'image', 'files'] });
  const historyListRef = useRef<HTMLDivElement | null>(null);
  const debouncedQuery = useDebouncedValue(query, 100);

  const filters: Array<{ key: FilterType; label: string }> = [
    ...getVisibleFilters(navFiltersConfig),
  ] as Array<{ key: FilterType; label: string }>;

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
                setStatusMessage('当前已是最新版本');
                return;
              }
              const confirmed = window.confirm(`发现新版本 ${update.version ?? ''}，是否现在更新？`);
              if (confirmed) {
                void installUpdate();
              }
            })
            .catch((error) => {
              const message = getErrorMessage(error, '检查更新失败');
              // Silently ignore 404/endpoint not found to avoid disrupting users before first release
              if (!message.includes('GitHub Release') && !message.includes('404')) {
                setStatusMessage(message);
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

  useEffect(() => {
    let ignore = false;

    searchItems(debouncedQuery, activeFilter)
      .then((backendItems) => {
        if (ignore) return;
        const nextItems = backendItems.map(mapBackendItemToViewItem);
        setItems(nextItems);
        setBackendAvailable(true);
        setStatusMessage(nextItems.length === 0 ? '暂无剪贴板记录' : '已连接本地剪贴板服务');
        setScrollTop(0);
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

  const selectedItem = visibleItems.find((item) => item.id === selectedId) ?? visibleItems[0];
  const virtualWindow = calculateVirtualWindow({
    itemCount: visibleItems.length,
    scrollTop,
    itemHeight: HISTORY_ITEM_HEIGHT,
    viewportHeight: historyListRef.current?.clientHeight ?? HISTORY_VIEWPORT_HEIGHT,
    overscan: 3,
  });
  const virtualItems = visibleItems.slice(virtualWindow.startIndex, virtualWindow.endIndex);

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

  async function deleteItem(id: string) {
    if (backendAvailable) {
      await deleteBackendItem(id);
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

  async function pasteAndHideItem(item: ClipboardItem) {
    setSelectedId(item.id);
    if (backendAvailable) {
      try {
        await getCurrentWindow().hide();
        await pasteItem(item.id);
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

  async function handleRecordingChange(nextRecording: boolean) {
    if (backendAvailable) {
      await setRecordingEnabled(nextRecording);
    }
    setRecording(nextRecording);
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
          setItems([]);
          setSelectedId(undefined);
          setRefreshVersion((current) => current + 1);
        }}
      />
    );
  }

  return (
    <main className="app-shell" onKeyDown={handleKeyboard}>
      <section className="toolbar">
        <div className="title-group">
          <div className="app-mark">
            <Clipboard size={18} />
          </div>
          <div>
            <h1>super-clipboard</h1>
            <p>{recording ? statusMessage : '已暂停记录'}</p>
          </div>
        </div>
        <div className="toolbar-actions">
          <button className="icon-button" title={recording ? '暂停记录' : '恢复记录'} onClick={() => void handleRecordingChange(!recording)}>
            {recording ? <Pause size={17} /> : <Play size={17} />}
          </button>
          <button className="icon-button" title="设置" onClick={() => setSettingsOpen(true)}>
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
          autoFocus
        />
        <kbd>Ctrl</kbd>
        <kbd>Shift</kbd>
        <kbd>V</kbd>
      </section>

      <section className="filter-row" aria-label="剪贴板类型过滤">
        {filters.map((filter) => (
          <button
            key={filter.key}
            className={activeFilter === filter.key ? 'filter-chip active' : 'filter-chip'}
            onClick={() => setActiveFilter(filter.key)}
          >
            {filter.label}
          </button>
        ))}
      </section>

      <section className={previewEnabled ? 'content-grid' : 'content-grid no-preview'}>
        <div
          ref={historyListRef}
          className="history-list"
          onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        >
          <div className="history-spacer" style={{ height: visibleItems.length * HISTORY_ITEM_HEIGHT }}>
            <div style={{ transform: `translateY(${virtualWindow.offsetTop}px)` }}>
          {virtualItems.map((item) => (
            <button
              key={item.id}
              className={[
                selectedItem?.id === item.id ? 'history-item selected' : 'history-item',
                draggingId === item.id ? 'dragging' : '',
              ].join(' ')}
              draggable
              onDragStart={() => setDraggingId(item.id)}
              onDragOver={(event) => event.preventDefault()}
              onDrop={(event) => {
                event.preventDefault();
                void handleDropItem(item.id);
              }}
              onDragEnd={() => setDraggingId(null)}
              onClick={() => void pasteListItem(item)}
            >
              <span className={item.type === 'image' && item.contentPath ? 'type-icon image-thumb' : 'type-icon'}>
                {item.type === 'image' && item.contentPath ? (
                  <img src={convertFileSrc(item.contentPath)} alt="" />
                ) : (
                  iconForType(item.type)
                )}
              </span>
              <span className="item-main">
                <span className="item-preview">{getVisualPreview(item)}</span>
                <span className="item-meta">
                  {getTypeLabel(item.type)} · {item.source} · {formatTime(item.updatedAt)}
                </span>
              </span>
              {item.favorite ? <Heart className="favorite" size={15} fill="currentColor" /> : null}
            </button>
          ))}
            </div>
          </div>
          {visibleItems.length === 0 ? <div className="empty-state">没有匹配的剪贴板记录</div> : null}
        </div>

        {previewEnabled ? (
          <aside className="detail-pane">
            {selectedItem ? (
              <>
                <div className="detail-header">
                  <span className="detail-type">{getTypeLabel(selectedItem.type)}</span>
                  <span>{selectedItem.size}</span>
                </div>
                {selectedItem.type === 'image' && selectedItem.contentPath ? (
                  <div className="image-preview">
                    <img src={convertFileSrc(selectedItem.contentPath)} alt="剪贴板图片预览" />
                  </div>
                ) : (
                  <pre>{selectedItem.preview}</pre>
                )}
                <div className="detail-actions">
                  <button onClick={() => void pasteSelectedItem()}>
                    <Pin size={16} />
                    粘贴
                  </button>
                  <button onClick={() => void copySelectedItem()}>
                    <Copy size={16} />
                    仅复制
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
              <div className="empty-state">选择一条记录查看详情</div>
            )}
          </aside>
        ) : null}
      </section>
    </main>
  );
}
