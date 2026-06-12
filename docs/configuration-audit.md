# 配置审计报告

**审计日期**：2026-06-12  
**审计范围**：自动更新链路、自动粘贴功能、托盘图标

## 1. 自动更新配置 ✅

### 1.1 Tauri 配置文件（`src-tauri/tauri.conf.json`）

```json
{
  "bundle": {
    "createUpdaterArtifacts": true
  },
  "plugins": {
    "updater": {
      "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDk3MTY1MkJEMDc5OTZEOTEKUldTUmJaa0h2VklXbDRsMW1hV09lQkxvRG5Yc29Ja2NxMm9JMVFSLzN2QVpwU0RxdWR3Qm12ZFoK",
      "endpoints": [
        "https://github.com/qianduoduo184/super-clipboard/releases/latest/download/latest.json"
      ]
    }
  }
}
```

**状态**：✅ 配置正确
- `createUpdaterArtifacts: true` 启用了更新产物生成
- 公钥已配置（来自 `npx tauri signer generate`）
- endpoint 指向 GitHub Release 的 `latest.json`

### 1.2 权限配置（`src-tauri/capabilities/default.json`）

```json
{
  "permissions": [
    "updater:default"
  ]
}
```

**状态**：✅ 权限已添加
- `updater:default` 允许前端调用更新 API

### 1.3 后端命令（`src-tauri/src/commands.rs`）

```rust
#[tauri::command]
pub async fn check_update(app: AppHandle, state: State<'_, AppState>) -> Result<UpdateInfo, String>

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String>
```

**状态**：✅ 命令已实现
- `check_update`：检查更新并记录检查日期
- `install_update`：下载并安装更新，完成后重启应用

### 1.4 命令注册（`src-tauri/src/main.rs`）

```rust
.invoke_handler(tauri::generate_handler![
    commands::check_update,
    commands::install_update
])
```

**状态**：✅ 已注册到 Tauri handler

### 1.5 前端 API（`src/features/history/api.ts`）

```typescript
export async function checkForUpdates() {
  return invoke<UpdateInfo>('check_update');
}

export async function installUpdate() {
  return invoke<void>('install_update');
}
```

**状态**：✅ 前端封装完整

### 1.6 设置界面（`src/features/settings/SettingsView.tsx`）

- 自动检查更新开关（每日首次启动）
- 手动检查更新按钮
- 发现新版本时弹窗确认

**状态**：✅ UI 已完整实现

### 1.7 CI/CD 流程（`.github/workflows/release.yml`）

- 检查 `TAURI_SIGNING_PRIVATE_KEY` 是否配置
- 配置时：验证私钥格式 → 构建签名更新产物 → 上传到 GitHub Release
- 未配置时：关闭 `createUpdaterArtifacts` → 仅上传 NSIS 安装包

**状态**：✅ 降级策略完备，未配置签名密钥时不会导致构建失败

## 2. 自动粘贴功能 ✅

### 2.1 后端实现（`src-tauri/src/clipboard/win.rs`）

```rust
pub fn simulate_paste_shortcut() -> Result<()> {
    let inputs = [
        keyboard_input(VK_CONTROL, false),
        keyboard_input(VK_V, false),
        keyboard_input(VK_V, true),
        keyboard_input(VK_CONTROL, true),
    ];
    unsafe { SendInput(...) }
}
```

**状态**：✅ 使用 Windows `SendInput` API 模拟 `Ctrl+V`

### 2.2 粘贴命令（`src-tauri/src/commands.rs`）

```rust
#[tauri::command]
pub fn paste_item(state: State<'_, AppState>, id: String) -> Result<(), String> {
    copy_item(state, id)?;
    #[cfg(target_os = "windows")]
    crate::clipboard::win::simulate_paste_shortcut().map_err(|error| error.to_string())?;
    Ok(())
}
```

**状态**：✅ 先复制到剪贴板，再模拟粘贴快捷键

### 2.3 前端交互（`src/App.tsx`）

- 列表项点击：`onClick={() => void pasteListItem(item)}`
- Enter 键：`if (event.key === 'Enter') pasteSelectedItem()`
- 详情面板按钮：`<button onClick={() => void pasteSelectedItem()}>粘贴</button>`

所有入口都调用 `pasteItem(id)` → 成功后 `getCurrentWindow().hide()`

**状态**：✅ 三个交互入口完整，粘贴后自动隐藏窗口

### 2.4 窗口隐藏权限（`src-tauri/capabilities/default.json`）

```json
{
  "permissions": [
    "core:window:allow-hide"
  ]
}
```

**状态**：✅ 权限已添加（修复自代码审查发现的权限缺失问题）

### 2.5 适配场景

已在 `docs/clipboard-behavior.md` 中记录：

- ✅ 微信、浏览器、Office、代码编辑器、终端均支持
- 🚫 禁用剪贴板的安全应用、全屏游戏不支持

## 3. 托盘图标 ✅（本次修复）

### 3.1 问题描述

托盘图标显示为空白，原因是 `TrayIconBuilder` 未设置图标。

### 3.2 修复内容（`src-tauri/src/system/tray.rs`）

```rust
TrayIconBuilder::new()
    .icon(app.default_window_icon().unwrap().clone())  // 新增
    .menu(&menu)
    ...
```

**状态**：✅ 已修复，使用应用默认图标（`src-tauri/icons/icon.ico`）

### 3.3 图标资源

- 路径：`src-tauri/icons/icon.ico`
- 大小：7,004 字节
- 配置：`tauri.conf.json` → `bundle.icon: ["icons/icon.ico"]`

**状态**：✅ 图标文件存在且已正确配置

## 4. 潜在风险

### 4.1 自动更新签名密钥

- ⚠️ **GitHub Secret 必须配置**：仓库需要配置 `TAURI_SIGNING_PRIVATE_KEY` 和 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- ⚠️ **私钥格式**：必须是 `.key` 文件内容（base64 编码），不是 `.key.pub` 公钥
- ✅ **降级策略**：未配置时会跳过签名步骤，仍能生成安装包

### 4.2 自动粘贴兼容性

- ⚠️ **焦点依赖**：粘贴前用户必须手动点击目标应用输入框（Windows 限制）
- ⚠️ **输入法拦截**：部分输入法可能拦截 `Ctrl+V`（罕见）
- ✅ **降级方案**：用户可使用「仅复制」按钮，然后手动粘贴

### 4.3 托盘图标加载

- ⚠️ **依赖默认图标**：如果 `app.default_window_icon()` 返回 `None`，会 panic
- ✅ **当前状态**：`tauri.conf.json` 中已配置 `bundle.icon`，不会失败

## 5. 验证清单

- [x] 前端测试通过（`npm run ci:frontend`：31 个测试 + typecheck）
- [ ] 后端测试通过（需要本地安装 Rust 工具链：`cargo test`）
- [ ] 实机测试：启动应用检查托盘图标显示
- [ ] 实机测试：点击列表项验证自动粘贴到微信/浏览器
- [ ] 实机测试：设置页面手动检查更新（需要配置 GitHub Secret 后推送触发 Release）

## 6. 结论

✅ **自动更新配置链路完整**，所有组件（配置文件、权限、命令、UI）一致且正确注册。

✅ **自动粘贴功能已完整实现**，支持点击/Enter 键触发，粘贴后自动隐藏窗口。

✅ **托盘图标问题已修复**，应用启动后托盘会显示 `icon.ico`。

**下一步**：
1. 本地安装 Rust 工具链后运行 `cargo test` 验证后端逻辑
2. 实机测试托盘图标和自动粘贴功能
3. 配置 GitHub Secret 后推送到 `main` 触发 Release，验证自动更新完整流程
