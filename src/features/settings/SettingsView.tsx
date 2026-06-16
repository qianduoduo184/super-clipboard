import { useEffect, useState, type KeyboardEvent } from 'react';
import { ArrowLeft, Database, Download, Eye, FileText, Filter, FolderOpen, HardDrive, Keyboard, Moon, Power, Shield, Sun, Trash2, Upload } from 'lucide-react';
import {
  checkForUpdates,
  clearHistory,
  getDiagnostics,
  getSettings,
  installUpdate,
  setGlobalShortcut,
  updateSettings,
  selectDirectory,
  migrateDirectory,
  updateStorageSettings,
  exportBackup,
  selectBackupFile,
  parseBackupInfo,
  importBackup,
  type BackupInfo,
} from '../history/api';
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
  const [pendingDataDir, setPendingDataDir] = useState<string | null>(null);
  const [pendingLogDir, setPendingLogDir] = useState<string | null>(null);
  const [pendingBackup, setPendingBackup] = useState<{ path: string; info: BackupInfo } | null>(null);

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

  async function handleManualUpdateCheck() {
    try {
      setStatus('正在检查更新...');
      const update = await checkForUpdates();
      if (!update.available) {
        setStatus('当前已是最新版本');
        window.alert('当前已是最新版本');
        return;
      }
      const confirmed = window.confirm(`发现新版本 ${update.version ?? ''}，是否现在更新？`);
      if (confirmed) {
        setStatus('正在下载并安装更新...');
        await installUpdate();
        setStatus('更新已安装完成，应用即将重启');
        window.alert('更新已安装完成，应用将在 2 秒后自动重启');
      } else {
        setStatus('已取消更新');
      }
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      let userMessage = '检查更新失败';

      if (message.includes('Could not fetch') || message.includes('404') || message.includes('Not Found')) {
        userMessage = '暂无可用更新。首次 Release 将在下次代码推送后自动构建。';
      } else if (message.includes('Network') || message.includes('timeout')) {
        userMessage = '网络连接失败，请检查网络后重试';
      } else {
        userMessage = `检查更新失败: ${message}`;
      }

      setStatus(userMessage);
      window.alert(userMessage);
    }
  }

  function handleKeyboard(event: React.KeyboardEvent<HTMLElement>) {
    if (event.key === 'Escape') {
      event.preventDefault();
      if (capturingShortcut) {
        setCapturingShortcut(false);
        setShortcutDraft('');
        setShortcutError('');
      } else {
        onBack();
      }
    }
  }

  async function handleSelectDataDir() {
    try {
      const selected = await selectDirectory();
      if (selected) {
        setPendingDataDir(selected);
      }
    } catch (error) {
      setStatus(`选择目录失败: ${error}`);
    }
  }

  async function handleSelectLogDir() {
    try {
      const selected = await selectDirectory();
      if (selected) {
        setPendingLogDir(selected);
      }
    } catch (error) {
      setStatus(`选择目录失败: ${error}`);
    }
  }

  async function handleConfirmDataDirChange() {
    if (!pendingDataDir || !diagnostics) return;

    const oldPath = diagnostics.app_data_dir;
    const newPath = pendingDataDir;

    const choice = window.confirm(
      `即将更改数据目录：\n\n` +
      `当前目录: ${oldPath}\n` +
      `新目录: ${newPath}\n\n` +
      `是否将现有数据迁移到新目录？\n\n` +
      `[确定] = 迁移并切换\n` +
      `[取消] = 放弃更改`
    );

    if (!choice) {
      setPendingDataDir(null);
      setStatus('已取消数据目录更改');
      return;
    }

    try {
      setStatus('正在迁移数据...');
      await migrateDirectory(oldPath, newPath, true);

      const updatedSettings = await updateStorageSettings(newPath, settings.custom_log_dir ?? null);
      setSettings(mergeSettings(updatedSettings));
      onSettingsChanged(updatedSettings);

      setPendingDataDir(null);
      setStatus('数据目录已更改，需要重启应用生效');
      window.alert('数据目录已更改。请重启应用以应用新设置。');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setStatus(`数据迁移失败: ${message}`);
      window.alert(`数据迁移失败: ${message}\n\n数据目录保持不变。`);
      setPendingDataDir(null);
    }
  }

  async function handleConfirmLogDirChange() {
    if (!pendingLogDir || !diagnostics) return;

    const oldPath = diagnostics.log_path.substring(0, diagnostics.log_path.lastIndexOf('\\'));
    const newPath = pendingLogDir;

    const choice = window.confirm(
      `即将更改日志目录：\n\n` +
      `当前目录: ${oldPath}\n` +
      `新目录: ${newPath}\n\n` +
      `是否将现有日志迁移到新目录？\n\n` +
      `[确定] = 迁移并切换\n` +
      `[取消] = 放弃更改`
    );

    if (!choice) {
      setPendingLogDir(null);
      setStatus('已取消日志目录更改');
      return;
    }

    try {
      setStatus('正在迁移日志...');
      await migrateDirectory(oldPath, newPath, true);

      const updatedSettings = await updateStorageSettings(settings.custom_data_dir ?? null, newPath);
      setSettings(mergeSettings(updatedSettings));
      onSettingsChanged(updatedSettings);

      setPendingLogDir(null);
      setStatus('日志目录已更改，需要重启应用生效');
      window.alert('日志目录已更改。请重启应用以应用新设置。');
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setStatus(`日志迁移失败: ${message}`);
      window.alert(`日志迁移失败: ${message}\n\n日志目录保持不变。`);
      setPendingLogDir(null);
    }
  }

  function handleCancelDataDirChange() {
    setPendingDataDir(null);
    setStatus('已取消数据目录更改');
  }

  function handleCancelLogDirChange() {
    setPendingLogDir(null);
    setStatus('已取消日志目录更改');
  }

  async function handleExportBackup() {
    try {
      setStatus('正在导出备份...');
      const savedPath = await exportBackup();
      setStatus(`备份已导出到: ${savedPath}`);
      window.alert(`备份导出成功！\n\n保存路径: ${savedPath}\n\n可以在文件管理器中打开该目录。`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (message.includes('取消')) {
        setStatus('已取消导出');
      } else {
        setStatus(`导出失败: ${message}`);
        window.alert(`导出失败: ${message}`);
      }
    }
  }

  async function handleSelectBackupForImport() {
    try {
      const selected = await selectBackupFile();
      if (!selected) {
        setStatus('已取消导入');
        return;
      }

      setStatus('正在解析备份文件...');
      const info = await parseBackupInfo(selected);
      setPendingBackup({ path: selected, info });
      setStatus(`已选择备份: ${info.item_count} 条记录`);
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setStatus(`解析备份文件失败: ${message}`);
      window.alert(`解析备份文件失败: ${message}`);
    }
  }

  async function handleConfirmImport(merge: boolean) {
    if (!pendingBackup) return;

    const action = merge ? '合并' : '覆盖';
    const warning = merge
      ? '将导入备份数据并与现有数据合并（跳过重复项）。'
      : '将清空现有数据并导入备份（会自动创建临时备份）。';

    const confirmed = window.confirm(
      `即将${action}导入备份：\n\n` +
      `创建时间: ${new Date(pendingBackup.info.created_at).toLocaleString()}\n` +
      `数据条数: ${pendingBackup.info.item_count}\n` +
      `备份版本: ${pendingBackup.info.version}\n\n` +
      `${warning}\n\n` +
      `是否继续？`
    );

    if (!confirmed) {
      setStatus('已取消导入');
      return;
    }

    try {
      setStatus('正在导入备份...');
      const importedCount = await importBackup(pendingBackup.path, merge);

      setPendingBackup(null);
      setStatus(`导入成功，已导入 ${importedCount} 条记录`);
      window.alert(`导入成功！\n\n已导入 ${importedCount} 条记录。\n\n请刷新历史列表以查看导入的数据。`);

      // 触发历史记录刷新
      onHistoryCleared();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setStatus(`导入失败: ${message}`);
      window.alert(`导入失败: ${message}\n\n数据已回滚，请检查备份文件格式。`);
    }
  }

  function handleCancelImport() {
    setPendingBackup(null);
    setStatus('已取消导入');
  }

  return (
    <main className="app-shell settings-shell" onKeyDown={handleKeyboard}>
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

        <label className="setting-row">
          <span className="setting-icon"><Download size={18} /></span>
          <span>
            <strong>自动检查更新</strong>
            <small>每天第一次启动时检查 GitHub Release 更新</small>
          </span>
          <input
            type="checkbox"
            checked={settings.auto_update_enabled}
            onChange={(event) =>
              void saveSettings(updateSettingValue(settings, 'auto_update_enabled', event.target.checked))
            }
          />
        </label>

        <div className="setting-row">
          <span className="setting-icon"><Download size={18} /></span>
          <span>
            <strong>手动检查更新</strong>
            <small>发现新版本后确认再下载并安装</small>
          </span>
          <button onClick={() => void handleManualUpdateCheck()}>
            检查
          </button>
        </div>

        <div className="setting-section-divider">备份管理</div>

        <div className="setting-row">
          <span className="setting-icon"><Download size={18} /></span>
          <span>
            <strong>导出备份</strong>
            <small>将剪贴板历史导出为 JSON 文件</small>
          </span>
          <button onClick={handleExportBackup}>
            <Download size={14} style={{ marginRight: '4px' }} />
            导出
          </button>
        </div>

        <div className="setting-row">
          <span className="setting-icon"><Upload size={18} /></span>
          <span>
            <strong>导入备份</strong>
            <small>
              {pendingBackup
                ? `${pendingBackup.info.item_count} 条记录 (${new Date(pendingBackup.info.created_at).toLocaleDateString()})`
                : '从备份文件恢复剪贴板历史'}
            </small>
          </span>
          <div style={{ display: 'flex', gap: '6px' }}>
            {pendingBackup ? (
              <>
                <button onClick={() => handleConfirmImport(true)}>合并</button>
                <button onClick={() => handleConfirmImport(false)}>覆盖</button>
                <button onClick={handleCancelImport}>取消</button>
              </>
            ) : (
              <button onClick={handleSelectBackupForImport}>
                <Upload size={14} style={{ marginRight: '4px' }} />
                选择
              </button>
            )}
          </div>
        </div>

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

        <div className="setting-section-divider">存储设置</div>

        <div className="setting-row">
          <span className="setting-icon"><Database size={18} /></span>
          <span>
            <strong>数据目录</strong>
            <small>{pendingDataDir || settings.custom_data_dir || diagnostics?.app_data_dir || '默认路径'}</small>
          </span>
          <div style={{ display: 'flex', gap: '6px' }}>
            {pendingDataDir ? (
              <>
                <button onClick={handleConfirmDataDirChange}>确认</button>
                <button onClick={handleCancelDataDirChange}>取消</button>
              </>
            ) : (
              <button onClick={handleSelectDataDir}>
                <FolderOpen size={14} style={{ marginRight: '4px' }} />
                更改
              </button>
            )}
          </div>
        </div>

        <div className="setting-row">
          <span className="setting-icon"><FileText size={18} /></span>
          <span>
            <strong>日志目录</strong>
            <small>{pendingLogDir || settings.custom_log_dir || (diagnostics?.log_path ? diagnostics.log_path.substring(0, diagnostics.log_path.lastIndexOf('\\')) : '默认路径')}</small>
          </span>
          <div style={{ display: 'flex', gap: '6px' }}>
            {pendingLogDir ? (
              <>
                <button onClick={handleConfirmLogDirChange}>确认</button>
                <button onClick={handleCancelLogDirChange}>取消</button>
              </>
            ) : (
              <button onClick={handleSelectLogDir}>
                <FolderOpen size={14} style={{ marginRight: '4px' }} />
                更改
              </button>
            )}
          </div>
        </div>

        <div className="setting-section-divider">诊断信息</div>

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

        <div className="setting-section-divider">快捷键说明</div>

        <div className="setting-row shortcut-help">
          <span className="setting-icon"><Keyboard size={18} /></span>
          <span>
            <strong>全局快捷键</strong>
            <small>在任意应用中唤出剪贴板面板</small>
          </span>
          <div className="shortcut-display">
            <kbd>Ctrl</kbd>
            <kbd>Shift</kbd>
            <kbd>V</kbd>
          </div>
        </div>

        <div className="setting-row shortcut-help">
          <span className="setting-icon"><Keyboard size={18} /></span>
          <span>
            <strong>选择并粘贴</strong>
            <small>在历史列表中选择条目后按下</small>
          </span>
          <div className="shortcut-display">
            <kbd>Enter</kbd>
          </div>
        </div>

        <div className="setting-row shortcut-help">
          <span className="setting-icon"><Keyboard size={18} /></span>
          <span>
            <strong>隐藏窗口</strong>
            <small>关闭剪贴板面板返回工作区</small>
          </span>
          <div className="shortcut-display">
            <kbd>ESC</kbd>
          </div>
        </div>

        <div className="setting-row shortcut-help">
          <span className="setting-icon"><Keyboard size={18} /></span>
          <span>
            <strong>上下导航</strong>
            <small>在历史列表中移动选择</small>
          </span>
          <div className="shortcut-display">
            <kbd>↑</kbd>
            <kbd>↓</kbd>
          </div>
        </div>
      </section>
    </main>
  );
}
