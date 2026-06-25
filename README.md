# super-clipboard

super-clipboard 是一个 Windows 优先、高性能、独立运行的剪贴板管理应用。通过「点击即粘贴」的交互模式，实现快速访问剪贴板历史并自动粘贴到当前应用。

[![Release](https://img.shields.io/github/v/release/qianduoduo184/super-clipboard)](https://github.com/qianduoduo184/super-clipboard/releases)
[![CI](https://github.com/qianduoduo184/super-clipboard/actions/workflows/ci.yml/badge.svg)](https://github.com/qianduoduo184/super-clipboard/actions)
[![License](https://img.shields.io/badge/license-未声明-lightgrey)](LICENSE)

## 核心特性

### 🚀 一键粘贴
- 按 `Ctrl+Shift+V` 打开历史面板
- 点击任意记录自动粘贴到当前应用（微信、浏览器、Office 等）
- Enter 键快速粘贴选中项
- 窗口失焦自动隐藏，ESC 智能清空搜索

### 📋 全类型支持
- **文本**：纯文本和 HTML 内容（自动转换为纯文本）
- **图片**：支持截图、复制的图片，保留原始质量，自动生成缩略图
- **文件列表**：记录复制的文件路径（回写功能开发中）

### 🔍 强大搜索
- SQLite FTS5 全文搜索，支持 CJK 子字符串匹配
- 按类型过滤：文本、图片、文件、收藏夹
- 可拖拽排序历史记录和导航过滤器
- 虚拟列表支持万级历史记录流畅滚动

### 🎨 系统集成
- 托盘常驻，最小化到后台
- 全局快捷键唤醒（可自定义）
- 开机自启动配置
- 主题切换（亮色/暗色）
- GitHub Release 自动更新（配置国内镜像加速）

### 🔒 安全与隐私
- 所有数据本地存储，不上传云端
- 已修复 4 个关键安全漏洞（v1.0.7）：
  - 图片路径遍历攻击防护
  - 目录迁移 TOCTOU 竞态修复
  - FTS5 查询注入防护
  - 剪贴板内存耗尽保护

## 快速开始

### 安装

1. 从 [GitHub Releases](https://github.com/qianduoduo184/super-clipboard/releases) 下载最新的 `.msi` 或 `.exe` 安装包
2. 运行安装程序，按提示完成安装
3. 启动后会在系统托盘显示图标

### 使用

1. **打开历史面板**：按 `Ctrl+Shift+V`（可在设置中自定义）
2. **快速粘贴**：
   - 点击任意历史记录，内容自动粘贴到当前应用
   - 或使用方向键选择 + `Enter` 粘贴
3. **搜索**：输入关键词筛选历史记录，支持中文子字符串
4. **收藏**：点击心形图标收藏常用内容
5. **管理**：右键菜单或详情面板可删除、仅复制、置顶
6. **排序**：拖拽历史记录调整顺序
7. **智能关闭**：
   - 按 ESC 先清空搜索，再次 ESC 关闭窗口
   - 点击其他应用时自动隐藏窗口

### 适配应用

✅ 已测试支持：微信、浏览器（Edge/Chrome/Firefox）、Office（Word/Excel/PowerPoint）、VS Code、记事本、终端

## 技术栈

- **前端**: React 18 + TypeScript + Vite
- **后端**: Tauri 2 + Rust
- **存储**: SQLite (WAL 模式) + FTS5 全文搜索
- **系统**: Windows Clipboard API + 全局快捷键 + 托盘

## 项目结构

```text
.
├─ src/                  # React/TypeScript 前端
│  ├─ lib/              # 纯逻辑层（带测试）
│  ├─ features/         # 功能模块
│  └─ App.tsx           # 主应用
├─ src-tauri/            # Tauri/Rust 后端
│  ├─ src/
│  │  ├─ clipboard/     # 剪贴板监听
│  │  ├─ storage/       # SQLite + FTS5
│  │  ├─ blobs/         # 图片/文件存储
│  │  ├─ system/        # 托盘/快捷键/自启
│  │  └─ commands.rs    # Tauri IPC 层
│  └─ Cargo.toml
├─ docs/                 # 文档
├─ .github/workflows/    # CI/CD
└─ package.json
```

## 开发

### 环境要求

- Node.js 20+
- npm
- Rust stable toolchain
- Windows WebView2 Runtime

### 安装依赖

```powershell
npm install
```

### 开发命令

```powershell
# 前端开发服务器
npm run dev

# 完整应用开发（Rust + WebView）
npm run tauri dev

# 前端测试
npm test

# 类型检查
npm run typecheck

# 前端 CI（测试 + 类型检查）
npm run ci:frontend

# 后端测试
cargo test --manifest-path src-tauri/Cargo.toml

# 生产构建
npm run build
```

## 当前状态

### ✅ 已完成（v1.0.7）

**核心功能**
- 剪贴板监听和历史记录（文本、HTML、图片、文件列表）
- SQLite FTS5 全文搜索，支持 CJK 子字符串和 10,000+ 条记录
- 点击即粘贴，自动模拟 `Ctrl+V` 快捷键
- 托盘图标、全局快捷键、开机启动
- 自动更新（GitHub Release + 国内镜像）
- 暗色/亮色主题切换

**用户体验**
- 拖拽排序历史记录和导航过滤器
- 可调整预览面板大小
- 动态高度虚拟滚动（图片行自适应高度）
- 智能 ESC 键处理（先清搜索，再关闭窗口）
- 窗口失焦自动隐藏并清空搜索
- 图片预览与时间戳垂直布局，避免重叠

**质量保障**
- 前端测试覆盖（33 个测试，100% 通过率）
- 后端测试覆盖（26 个测试，包含性能测试）
- TypeScript 类型检查 0 错误
- 安全测试套件（4 个关键漏洞修复）
- GitHub Actions 自动构建和发布

### 🔧 待完善

详见 [docs/TODO.md](docs/TODO.md)

**短期计划**
- [ ] 文件列表写回剪贴板（当前只支持读取）
- [ ] 实机大数据量测试（10,000+ 条历史记录）
- [ ] 托盘和快捷键生命周期稳定性测试

**长期计划**
- [ ] macOS 和 Linux 支持（当前仅 Windows）
- [ ] 云同步（可选）
- [ ] 更多剪贴板格式支持（RTF、图表等）
- [ ] 剪贴板内容加密选项

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

### 拖拽功能不工作
- 确保鼠标按住拖拽手柄图标（左侧六点图标）
- 拖拽时会显示半透明效果和虚线边框
- 释放鼠标时项目会移动到目标位置之后

## 文档

### 用户文档
- [隐私说明](docs/privacy.md) - 数据存储和安全策略
- [剪贴板行为](docs/clipboard-behavior.md) - 自动粘贴工作原理和适配场景
- [更新镜像说明](docs/UPDATE_MIRRORS.md) - 中国大陆用户更新加速方案

### 开发文档
- [架构说明](docs/architecture.md) - 系统设计和模块边界
- [配置审计](docs/configuration-audit.md) - 自动更新、粘贴、托盘配置检查
- [Windows 打包说明](docs/windows-packaging.md) - 本地构建指南
- [Release 说明](docs/release.md) - CI/CD 和自动发版流程
- [测试报告](docs/TEST_REPORT.md) - 自动化测试详细报告
- [TODO](docs/TODO.md) - 开发进度和待办事项
- [CLAUDE.md](CLAUDE.md) - AI 助手开发指南

### 安全文档
- [安全修复摘要](docs/SECURITY_FIXES_SUMMARY.md) - v1.0.7 安全漏洞修复详情
- [安全修复详情](docs/SECURITY_FIXES.md) - 技术细节和测试用例

## 贡献

欢迎贡献！请查看 [CONTRIBUTING.md](CONTRIBUTING.md) 了解：
- 如何报告 Bug 和建议新功能
- 开发环境搭建
- 代码规范和提交流程
- Pull Request 指南

### 快速开始贡献

1. **Fork 仓库**并克隆到本地
2. **创建分支**: `git checkout -b feat/your-feature`
3. **开发并测试**: `npm run ci:frontend && cargo test --manifest-path src-tauri/Cargo.toml`
4. **提交**: 遵循 [Conventional Commits](https://www.conventionalcommits.org/) 规范
5. **推送并创建 PR**

### 提交规范

```bash
feat: 新增功能
fix: Bug 修复
docs: 文档更新
style: 代码格式调整
refactor: 重构
test: 测试相关
chore: 构建/工具链相关
```

### 文档资源

- **[QUICK_REFERENCE.md](.github/QUICK_REFERENCE.md)** - 高效指令速查表（推荐先看）
- **[DEV_CHECKLIST.md](.github/DEV_CHECKLIST.md)** - 开发指令最佳实践
- **[COMMIT_TEMPLATE.md](.github/COMMIT_TEMPLATE.md)** - Git 提交规范
- **[CHANGELOG.md](CHANGELOG.md)** - 版本变更历史

## 版本历史

### v1.0.7 (2026-06-25)
- 🔒 修复 4 个关键安全漏洞
- ✨ 智能 ESC 键处理和窗口失焦自动隐藏
- 🎨 修复图片/时间戳重叠和拖拽排序问题
- 📝 详见 [docs/RELEASE_v1.0.7.md](docs/RELEASE_v1.0.7.md)

### v1.0.6
- ✨ 拖拽排序和导航过滤器配置
- 🐛 修复时间戳显示和数据丢失问题

完整版本历史见 [CHANGELOG.md](CHANGELOG.md)

## 隐私说明摘要

应用默认记录全部剪贴板内容，并将历史数据保存在本机应用数据目录（`%APPDATA%\app.superclipboard.desktop`）。

**重要说明**：
- ✅ 所有数据本地存储，不上传云端
- ✅ 不收集个人信息和使用统计
- ⚠️ 剪贴板可能包含密码、密钥、文档片段等敏感数据
- 📖 正式使用前应阅读 [隐私说明](docs/privacy.md) 并按需要调整记录策略

## 许可证

当前仓库尚未声明开源许可证。详见 LICENSE 文件。

## 致谢

- [Tauri](https://tauri.app/) - 跨平台应用框架
- [React](https://react.dev/) - UI 框架
- [rusqlite](https://github.com/rusqlite/rusqlite) - SQLite Rust 绑定
- [lucide-react](https://lucide.dev/) - 图标库

## 联系方式

- **Issues**: [GitHub Issues](https://github.com/qianduoduo184/super-clipboard/issues)
- **Discussions**: [GitHub Discussions](https://github.com/qianduoduo184/super-clipboard/discussions)

---

**⚡ 高效剪贴板管理，从 super-clipboard 开始**
