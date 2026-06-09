# Windows 打包说明

super-clipboard 使用 Tauri 2 打包 Windows 桌面应用。

## 前置要求

- Node.js 22 或兼容版本。
- Rust stable 工具链，包含 `cargo` 和 `rustc`。
- Microsoft Visual Studio Build Tools，安装 C++ 桌面开发组件。
- WebView2 Runtime。

## 本地构建

```powershell
npm install
npm run build
npm run tauri build
```

## 验证清单

- 应用可以启动主窗口。
- 托盘图标可以显示窗口和退出。
- `Ctrl+Shift+V` 可以呼出窗口。
- 复制文本、HTML、图片、文件列表后能进入历史记录。
- 设置页可以暂停/恢复记录。
- 清空历史后 SQLite 记录和未引用 blob 文件被清理。

## 输出物

Tauri 默认输出目录位于：

```text
src-tauri/target/release/bundle/
```

具体安装包格式由 `src-tauri/tauri.conf.json` 的 `bundle.targets` 决定。
