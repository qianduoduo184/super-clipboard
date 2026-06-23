# super-clipboard

super-clipboard 是一个 Windows 优先、高性能、独立运行的剪贴板管理应用。通过「点击即粘贴」的交互模式，实现快速访问剪贴板历史并自动粘贴到当前应用。

## 核心特性

### 🚀 一键粘贴
- 按 `Ctrl+Shift+V` 打开历史面板
- 点击任意记录自动粘贴到当前应用（微信、浏览器、Office 等）
- Enter 键快速粘贴选中项
- 窗口自动隐藏，无需手动切换

### 📋 全类型支持
- **文本**：纯文本和 HTML 内容（自动转换为纯文本）
- **图片**：支持截图、复制的图片，保留原始质量
- **文件列表**：记录复制的文件路径（回写功能开发中）

### 🔍 强大搜索
- SQLite FTS5 全文搜索，秒级响应
- 按类型过滤：文本、图片、文件
- 收藏夹快速访问
- 虚拟列表支持万级历史记录流畅滚动

### 🎨 系统集成
- 托盘常驻，最小化到后台
- 全局快捷键唤醒（可自定义）
- 开机自启动配置
- 主题切换（亮色/暗色）
- GitHub Release 自动更新

## 功能

- 文本、HTML、图片、文件列表历史记录捕获源码。
- SQLite 持久化，启用 WAL 和 FTS5 搜索。
- 按 hash 去重，支持软删除、收藏、分页查询。
- 大图片和二进制内容使用本地 blob 文件存储，数据库只保存元数据和引用路径。
- 快速启动面板：搜索、类型过滤、收藏过滤、虚拟列表、详情预览。
- 操作入口：粘贴、仅复制、删除、收藏/取消收藏、拖拽排序。
- 系统集成源码：托盘图标、全局快捷键、开机启动配置、设置持久化。
- 自动更新：每日检查 GitHub Release，发现新版本后一键下载安装。

## 快速开始

### 安装

1. 从 [GitHub Releases](https://github.com/qianduoduo184/super-clipboard/releases) 下载最新的 `.exe` 安装包
2. 运行安装程序，按提示完成安装
3. 启动后会在系统托盘显示图标

### 使用

1. **打开历史面板**：按 `Ctrl+Shift+V`（可在设置中自定义）
2. **快速粘贴**：
   - 点击任意历史记录，内容自动粘贴到当前应用
   - 或使用方向键选择 + `Enter` 粘贴
3. **搜索**：输入关键词筛选历史记录
4. **收藏**：点击心形图标收藏常用内容
5. **管理**：右键菜单或详情面板可删除、仅复制

### 适配应用

✅ 已测试支持：微信、浏览器（Edge/Chrome/Firefox）、Office（Word/Excel/PowerPoint）、VS Code、记事本、终端

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

### ✅ 已完成
- 剪贴板监听和历史记录（文本、HTML、图片、文件列表）
- SQLite FTS5 全文搜索，支持万级历史记录
- 点击即粘贴，自动模拟 `Ctrl+V` 快捷键
- 托盘图标、全局快捷键、开机启动
- 自动更新（GitHub Release）
- 暗色/亮色主题切换
- 前端测试覆盖（33 个测试 + TypeScript 类型检查）
- GitHub Actions 自动构建和发布
- 文件列表复制粘贴支持
- 动态高度虚拟滚动（图片行 3 倍高度）
- 拖拽排序和导航过滤器配置

### 🔧 待完善
详见 [docs/TODO.md](docs/TODO.md)

## 贡献

欢迎贡献！请查看 [CONTRIBUTING.md](CONTRIBUTING.md) 了解：
- 如何报告 Bug 和建议新功能
- 开发环境搭建
- 代码规范和提交流程
- Pull Request 指南

### 快速开始贡献

1. **Fork 仓库**并克隆
2. **创建分支**: `git checkout -b feat/your-feature`
3. **开发并测试**: `npm run ci:frontend && cargo test`
4. **提交**: 遵循 [Conventional Commits](https://www.conventionalcommits.org/) 规范
5. **推送并创建 PR**

### 文档资源

- **[QUICK_REFERENCE.md](.github/QUICK_REFERENCE.md)** - 高效指令速查表（推荐先看）
- **[DEV_CHECKLIST.md](.github/DEV_CHECKLIST.md)** - 开发指令最佳实践
- **[COMMIT_TEMPLATE.md](.github/COMMIT_TEMPLATE.md)** - Git 提交规范
- **[CHANGELOG.md](CHANGELOG.md)** - 版本变更历史
- **[CLAUDE.md](CLAUDE.md)** - AI 助手开发指南

## 许可证

[许可证类型] - 详见 LICENSE 文件

## 致谢

- [Tauri](https://tauri.app/) - 跨平台应用框架
- [React](https://react.dev/) - UI 框架
- [rusqlite](https://github.com/rusqlite/rusqlite) - SQLite Rust 绑定

### 🚧 进行中
- 文件列表写回剪贴板（当前只支持读取）
- 大数据量实机测试（10,000+ 条历史记录）
- Rust 后端测试（需要 Windows + Rust 工具链环境）

### 📋 计划
- macOS 和 Linux 支持（当前仅 Windows）
- 云同步（可选）
- 更多剪贴板格式支持

## 故障排查

### 应用无法启动
1. 检查日志文件：`%APPDATA%\app.superclipboard.desktop\logs\super-clipboard.log`
2. 也可以在应用「设置」页查看「运行日志」路径
3. 查看日志末尾的 `ERROR` 或 `panic` 记录

### 粘贴功能不工作
1. 确保目标应用的输入框已获得焦点（点击输入框）
2. 检查目标应用是否禁用了剪贴板（部分企业安全软件）
3. 尝试使用「仅复制」按钮，然后手动 `Ctrl+V`

### 托盘图标不显示
- 托盘初始化失败会记录到日志，但不会阻止主窗口启动
- 可通过全局快捷键 `Ctrl+Shift+V` 唤醒窗口

### 自动更新检查失败
- 首次安装时，GitHub Release 可能尚未创建，属于正常现象
- 推送代码到 main 分支后，CI 会自动构建并发布 Release
- 之后每日首次启动会自动检查更新
- **中国大陆用户**：应用已配置 GitHub 镜像加速，如仍然超时请查看 [更新镜像说明](docs/UPDATE_MIRRORS.md)

## 文档

- [架构说明](docs/architecture.md) - 系统设计和模块边界
- [隐私说明](docs/privacy.md) - 数据存储和安全策略
- [剪贴板行为](docs/clipboard-behavior.md) - 自动粘贴工作原理和适配场景
- [配置审计](docs/configuration-audit.md) - 自动更新、粘贴、托盘配置检查
- [Windows 打包说明](docs/windows-packaging.md) - 本地构建指南
- [Release 说明](docs/release.md) - CI/CD 和自动发版流程
- [更新镜像说明](docs/UPDATE_MIRRORS.md) - 中国大陆用户更新加速方案
- [测试报告](docs/TEST_REPORT.md) - 自动化测试详细报告
- [TODO](docs/TODO.md) - 开发进度和待办事项

## 隐私说明摘要

应用默认记录全部剪贴板内容，并将历史数据保存在本机应用数据目录。敏感内容不会上传到云端；但因为剪贴板可能包含密码、密钥、文档片段等敏感数据，正式使用前应阅读 [隐私说明](docs/privacy.md) 并按需要调整记录策略。

## License

当前仓库尚未声明开源许可证。
