# super-clipboard

super-clipboard 是一个 Windows 优先、高性能、独立运行的剪贴板管理应用。项目不依赖 uTools 插件体系，目标是用原生桌面能力提供常驻剪贴板历史、快速检索和快捷粘贴体验。

## 功能

- 文本、HTML、图片、文件列表历史记录捕获源码。
- SQLite 持久化，启用 WAL 和 FTS5 搜索。
- 按 hash 去重，支持软删除、收藏、分页查询。
- 大图片和二进制内容使用本地 blob 文件存储，数据库只保存元数据和引用路径。
- 快速启动面板：搜索、类型过滤、收藏过滤、虚拟列表、详情预览。
- 操作入口：复制、粘贴、删除、收藏/取消收藏。
- 系统集成源码：托盘、全局快捷键、开机启动配置、设置持久化。

## 技术栈

- Tauri 2
- Rust
- React 18
- TypeScript
- Vite
- SQLite / FTS5
- Windows Clipboard API

## 项目结构

```text
.
├─ src/                  # React/TypeScript 前端
├─ src-tauri/            # Tauri/Rust 后端
├─ docs/                 # 架构、隐私、打包和 TODO 文档
├─ package.json
└─ README.md
```

## 开发环境

需要安装：

- Node.js 20+
- npm
- Rust stable toolchain
- Windows WebView2 Runtime

安装依赖：

```powershell
npm install
```

前端开发：

```powershell
npm run dev
```

前端测试：

```powershell
npm test
```

类型检查：

```powershell
npx tsc --noEmit
```

生产构建：

```powershell
npm run build
```

Tauri 开发运行：

```powershell
npm run tauri dev
```

Rust 后端测试：

```powershell
cd src-tauri
cargo test
```

## 当前状态

- 前端测试已覆盖剪贴板模型、列表虚拟窗口、后端数据映射、设置模型和错误提示。
- 前端构建已验证通过。
- Rust/Tauri 后端源码已实现核心模块，但仍需要在安装 Rust 工具链的 Windows 环境中执行 `cargo test` 和实机剪贴板验证。
- 图片/文件写回剪贴板、托盘生命周期、全局快捷键生命周期、大数据量实测仍在 TODO 中跟踪。
- 推送到 `main` 分支后会通过 GitHub Actions 自动生成 GitHub prerelease 并上传 Windows 成品。

## 文档

- [架构说明](docs/architecture.md)
- [隐私说明](docs/privacy.md)
- [Windows 打包说明](docs/windows-packaging.md)
- [Release 说明](docs/release.md)
- [TODO](docs/TODO.md)

## 隐私说明摘要

应用默认记录全部剪贴板内容，并将历史数据保存在本机应用数据目录。敏感内容不会上传到云端；但因为剪贴板可能包含密码、密钥、文档片段等敏感数据，正式使用前应阅读 [隐私说明](docs/privacy.md) 并按需要调整记录策略。

## License

当前仓库尚未声明开源许可证。
