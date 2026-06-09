import { useEffect, useState, type KeyboardEvent } from 'react';
import { ArrowLeft, Database, Eye, FileText, Keyboard, Moon, Power, Shield, Sun, Trash2 } from 'lucide-react';
import { clearHistory, getDiagnostics, getSettings, setGlobalShortcut, updateSettings } from '../history/api';
import {
  applyThemeMode,
  createDefaultSettings,
  formatShortcutFromEvent,
  mergeSettings,
  shouldClearHistory,
  updateSettingValue,
  validateShortcut,
  type AppSettings,
} from '../../lib/settings-model';

type SettingsViewProps = {
  recording: boolean;
  onRecordingChange: (value: boolean) => void;
  onRecordingLoaded: (value: boolean) => void;
  onSettingsChanged: (settings: AppSettings) => void;
  onHistoryCleared: () => void;
  onBack: () => void;
};

export default function SettingsView({
  recording,
  onRecordingChange,
  onRecordingLoaded,
  onSettingsChanged,
  onHistoryCleared,
  onBack,
}: SettingsViewProps) {
  const [settings, setSettings] = useState<AppSettings>(() =>
    updateSettingValue(createDefaultSettings(), 'recording_enabled', recording),
  );
  const [status, setStatus] = useState('设置将在 Tauri 后端可用时自动同步');
  const [diagnostics, setDiagnostics] = useState<{ app_data_dir: string; log_path: string } | null>(null);
  const [capturingShortcut, setCapturingShortcut] = useState(false);
  const [shortcutDraft, setShortcutDraft] = useState('');
  const [shortcutError, setShortcutError] = useState('');

  useEffect(() => {
    let ignore = false;
    getSettings()
      .then((backendSettings) => {
        if (ignore) return;
        const nextSettings = mergeSettings(backendSettings);
        setSettings(nextSettings);
        applyThemeMode(nextSettings.theme_mode);
        onRecordingLoaded(nextSettings.recording_enabled);
        onSettingsChanged(nextSettings);
        setStatus('设置已从本地配置载入');
      })
      .catch(() => {
        if (ignore) return;
        setStatus('未连接 Tauri 后端，当前为本地预览设置');
      });

    getDiagnostics()
      .then((backendDiagnostics) => {
        if (ignore) return;
        setDiagnostics(backendDiagnostics);
      })
      .catch(() => {
        if (ignore) return;
        setDiagnostics(null);
      });

    return () => {
      ignore = true;
    };
  }, [onRecordingLoaded]);

  async function saveSettings(nextSettings: AppSettings) {
    setSettings(nextSettings);
    applyThemeMode(nextSettings.theme_mode);
    onRecordingChange(nextSettings.recording_enabled);
    onSettingsChanged(nextSettings);
    try {
      const savedSettings = await updateSettings(nextSettings);
      const mergedSettings = mergeSettings(savedSettings);
      setSettings(mergedSettings);
      applyThemeMode(mergedSettings.theme_mode);
      onSettingsChanged(mergedSettings);
      setStatus('设置已保存');
    } catch {
      setStatus('后端不可用，设置仅在当前界面生效');
    }
  }

  async function saveShortcut(shortcut: string) {
    if (!validateShortcut(shortcut)) {
      setShortcutError('快捷键必须包含修饰键和主键');
      return;
    }

    setShortcutError('');
    try {
      const savedSettings = await setGlobalShortcut(shortcut);
      const mergedSettings = mergeSettings(savedSettings);
      setSettings(mergedSettings);
      applyThemeMode(mergedSettings.theme_mode);
      onSettingsChanged(mergedSettings);
      setCapturingShortcut(false);
      setShortcutDraft('');
      setStatus('快捷键已更新');
    } catch (error) {
      setShortcutError(error instanceof Error ? error.message : String(error));
      setStatus('快捷键注册失败，已保留原快捷键');
    }
  }

  function handleShortcutCapture(event: KeyboardEvent<HTMLDivElement>) {
    if (!capturingShortcut) return;
    event.preventDefault();
    event.stopPropagation();

    if (event.key === 'Escape') {
      setCapturingShortcut(false);
      setShortcutDraft('');
      setShortcutError('');
      return;
    }

    if (event.key === 'Enter') {
      if (shortcutDraft) {
        void saveShortcut(shortcutDraft);
      }
      return;
    }

    const nextShortcut = formatShortcutFromEvent(event.nativeEvent);
    setShortcutDraft(nextShortcut);
    setShortcutError(nextShortcut && !validateShortcut(nextShortcut) ? '继续按下一个主键' : '');
  }

  async function handleClearHistory() {
    if (!shouldClearHistory(window.confirm('确定要清空全部剪贴板历史和 blob 文件吗？'))) {
      return;
    }

    try {
      await clearHistory();
      onHistoryCleared();
      setStatus('历史记录已清空');
    } catch {
      setStatus('后端不可用，无法清空真实历史记录');
    }
  }

  return (
    <main className="app-shell settings-shell">
      <section className="toolbar">
        <div className="title-group">
          <button className="icon-button" title="返回" onClick={onBack}>
            <ArrowLeft size={18} />
          </button>
          <div>
            <h1>设置</h1>
            <p>{status}</p>
          </div>
        </div>
      </section>

      <section className="settings-list">
        <div className="setting-row">
          <span className="setting-icon"><Sun size={18} /></span>
          <span>
            <strong>主题</strong>
            <small>切换亮色或暗色显示模式</small>
          </span>
          <div className="segmented-control" aria-label="主题模式">
            <button
              className={settings.theme_mode === 'light' ? 'active' : ''}
              onClick={() => void saveSettings(updateSettingValue(settings, 'theme_mode', 'light'))}
            >
              <Sun size={15} />
              亮色
            </button>
            <button
              className={settings.theme_mode === 'dark' ? 'active' : ''}
              onClick={() => void saveSettings(updateSettingValue(settings, 'theme_mode', 'dark'))}
            >
              <Moon size={15} />
              暗色
            </button>
          </div>
        </div>

        <label className="setting-row">
          <span className="setting-icon"><Power size={18} /></span>
          <span>
            <strong>记录剪贴板</strong>
            <small>关闭后不会写入新的历史记录</small>
          </span>
          <input
            type="checkbox"
            checked={settings.recording_enabled}
            onChange={(event) =>
              void saveSettings(updateSettingValue(settings, 'recording_enabled', event.target.checked))
            }
          />
        </label>

        <label className="setting-row">
          <span className="setting-icon"><Eye size={18} /></span>
          <span>
            <strong>详情预览</strong>
            <small>关闭后主界面只显示历史列表</small>
          </span>
          <input
            type="checkbox"
            checked={settings.preview_enabled}
            onChange={(event) =>
              void saveSettings(updateSettingValue(settings, 'preview_enabled', event.target.checked))
            }
          />
        </label>

        <label className="setting-row">
          <span className="setting-icon"><Database size={18} /></span>
          <span>
            <strong>最大历史条数</strong>
            <small>超过限制后自动清理非收藏记录</small>
          </span>
          <select
            value={settings.max_history_items}
            onChange={(event) =>
              void saveSettings(updateSettingValue(settings, 'max_history_items', Number(event.target.value)))
            }
          >
            <option value="1000">1,000</option>
            <option value="5000">5,000</option>
            <option value="10000">10,000</option>
            <option value="50000">50,000</option>
          </select>
        </label>

        <label className="setting-row">
          <span className="setting-icon"><Shield size={18} /></span>
          <span>
            <strong>最长保存时间</strong>
            <small>默认记录全部内容，按保留时间清理</small>
          </span>
          <select
            value={settings.retention_days}
            onChange={(event) =>
              void saveSettings(updateSettingValue(settings, 'retention_days', Number(event.target.value)))
            }
          >
            <option value="7">7 天</option>
            <option value="14">14 天</option>
            <option value="30">30 天</option>
            <option value="0">永久</option>
          </select>
        </label>

        <div className="setting-row">
          <span className="setting-icon"><Keyboard size={18} /></span>
          <span>
            <strong>全局快捷键</strong>
            <small>{shortcutError || (capturingShortcut ? '按组合键，Enter 保存，Esc 取消' : '打开快速历史面板')}</small>
          </span>
          <div
            className={capturingShortcut ? 'shortcut-capture capturing' : 'shortcut-capture'}
            tabIndex={0}
            onKeyDown={handleShortcutCapture}
          >
            <span>{shortcutDraft || settings.global_shortcut}</span>
            {capturingShortcut ? (
              <button onClick={() => void saveShortcut(shortcutDraft)} disabled={!validateShortcut(shortcutDraft)}>
                保存
              </button>
            ) : (
              <button
                onClick={(event) => {
                  setCapturingShortcut(true);
                  setShortcutDraft('');
                  setShortcutError('');
                  event.currentTarget.parentElement?.focus();
                }}
              >
                修改
              </button>
            )}
          </div>
        </div>

        <label className="setting-row">
          <span className="setting-icon"><Power size={18} /></span>
          <span>
            <strong>开机启动</strong>
            <small>登录 Windows 后自动启动后台监听</small>
          </span>
          <input
            type="checkbox"
            checked={settings.autostart_enabled}
            onChange={(event) =>
              void saveSettings(updateSettingValue(settings, 'autostart_enabled', event.target.checked))
            }
          />
        </label>

        <div className="setting-row">
          <span className="setting-icon"><Trash2 size={18} /></span>
          <span>
            <strong>清空历史</strong>
            <small>删除历史记录和未引用 blob 文件</small>
          </span>
          <button className="danger-button" onClick={() => void handleClearHistory()}>
            清空
          </button>
        </div>

        <div className="setting-row">
          <span className="setting-icon"><FileText size={18} /></span>
          <span>
            <strong>运行日志</strong>
            <small>{diagnostics?.log_path ?? 'Tauri 后端可用后显示日志路径'}</small>
          </span>
        </div>

        <div className="setting-row">
          <span className="setting-icon"><Database size={18} /></span>
          <span>
            <strong>数据目录</strong>
            <small>{diagnostics?.app_data_dir ?? 'Tauri 后端可用后显示数据目录'}</small>
          </span>
        </div>
      </section>
    </main>
  );
}
