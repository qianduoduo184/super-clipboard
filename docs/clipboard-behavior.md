# 剪贴板行为说明

## 自动粘贴功能

super-clipboard 实现了「点击即粘贴」的交互模式，模拟真实软件使用场景。

### 工作原理

1. **列表项点击**：点击历史列表中的任意条目，应用会：
   - 将该条目内容写入系统剪贴板
   - 模拟 `Ctrl+V` 键盘输入
   - 隐藏 super-clipboard 窗口
   - 内容自动粘贴到当前激活的应用输入框

2. **Enter 键**：在历史列表中按 Enter 键也会触发相同的粘贴流程

3. **详情面板粘贴按钮**：点击右侧详情面板的「粘贴」按钮触发粘贴

### 技术实现

- **后端**（`src-tauri/src/clipboard/win.rs`）：
  - `simulate_paste_shortcut()` 通过 Windows `SendInput` API 发送 `Ctrl+V` 按键序列
  - 使用 `KEYEVENTF_KEYUP` 确保完整的按下/释放循环

- **前端**（`src/App.tsx`）：
  - 调用 `pasteItem(id)` Tauri 命令
  - 成功后调用 `getCurrentWindow().hide()` 隐藏窗口
  - 权限：需要 `core:window:allow-hide`（已在 `capabilities/default.json` 配置）

### 适配范围

自动粘贴适用于接受标准剪贴板输入的 Windows 应用：

- ✅ **微信**：聊天输入框、朋友圈编辑器
- ✅ **浏览器**：Edge、Chrome、Firefox 的网页输入框
- ✅ **Office 套件**：Word、Excel、PowerPoint
- ✅ **代码编辑器**：VS Code、Notepad++、Sublime Text
- ✅ **终端**：Windows Terminal、PowerShell
- ✅ **记事本**：Windows 记事本、写字板

### 失效场景

以下场景可能无法自动粘贴：

- 🚫 禁用剪贴板的安全应用（如部分企业聊天工具）
- 🚫 自定义输入法拦截了 `Ctrl+V`
- 🚫 当前焦点不在可编辑区域（需要用户先点击输入框）
- 🚫 应用正在全屏独占模式（游戏、视频播放器）

### 降级方案

如果自动粘贴失败，用户可以：

1. 点击详情面板的「仅复制」按钮，内容已在系统剪贴板
2. 手动切换到目标应用并按 `Ctrl+V`

## 内容类型支持

### 已实现

- **文本**：纯文本直接粘贴
- **HTML**：转换为纯文本后粘贴（去除标签，保留内容）
- **图片**：从 blob 读取 DIB 格式并写入剪贴板，支持粘贴到支持图片的应用

### 待实现

- **文件列表**：当前只记录文件路径，不支持回写到剪贴板（tracked in `docs/TODO.md`）

## 隐私与安全

- super-clipboard **不会**主动粘贴到后台应用
- **必须**用户主动点击或按 Enter 才触发粘贴
- 粘贴前内容已显示在界面，用户可预览
- 所有数据存储在本地 SQLite，不联网上传
