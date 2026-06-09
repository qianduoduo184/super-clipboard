# super-clipboard Windows 应用实现计划

> **给 agentic workers：** REQUIRED SUB-SKILL: 使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 按任务执行。本计划使用 checkbox（`- [ ]`）跟踪进度。

**目标：** 构建一个 Windows 优先、高性能、独立运行的剪贴板管理应用，不再依赖 uTools 插件体系。

**架构：** 使用 Tauri 2 作为桌面应用壳，Rust 负责剪贴板监听、存储、搜索、托盘、快捷键等系统能力，React/TypeScript 负责界面。历史数据使用 SQLite 持久化，大图片和大文件内容存储为外部 blob，数据库只保存元数据和引用路径。

**技术栈：** Tauri 2、Rust、TypeScript、React、Vite、SQLite、SQLite FTS5、Windows Clipboard API、Tauri global-shortcut/autostart/tray 能力。

---

## 方案概述

首版实现一个高性能核心版剪贴板管理器：

- 支持文本、HTML、图片、文件列表历史记录。
- 支持快速搜索、类型过滤、收藏、删除、快捷粘贴。
- 支持托盘常驻、全局快捷键、开机启动。
- 使用 SQLite 替代 JSON 大文件存储。
- 避开原“超级剪切板”项目的旧依赖、轮询监听、大图片 DataURL 入库、权限暴露过宽等问题。

## 核心决策

- 采用 `Tauri 2 + Rust + React + TypeScript`。
- v1 只面向 Windows 优化。
- 默认记录全部剪贴板内容。
- 首版不做云同步、OCR、插件系统、脚本系统、多设备同步。
- 文本内容进入 SQLite 和 FTS5 搜索索引；大图片、大二进制内容写入本地文件，SQLite 保存引用。

## 执行任务

### 任务 1：项目初始化

**文件：**
- 创建：`package.json`
- 创建：`src-tauri/`
- 创建：`src/`
- 创建：`README.md`

- [ ] 创建 Tauri 2 + React + TypeScript 项目。
- [ ] 设置应用名为 `super-clipboard`。
- [ ] 配置基础 Windows 应用窗口。
- [ ] 配置快速启动面板窗口尺寸和行为。
- [ ] 运行 `npm run tauri dev`，确认空应用可启动。
- [ ] 提交：`chore: scaffold tauri app`

### 任务 2：SQLite 存储层

**文件：**
- 创建：`src-tauri/src/storage/mod.rs`
- 创建：`src-tauri/src/storage/schema.rs`
- 创建：`src-tauri/src/storage/repository.rs`

- [ ] 初始化 SQLite 数据库。
- [ ] 启用 WAL 模式。
- [ ] 创建剪贴板历史表。
- [ ] 创建 FTS5 搜索表。
- [ ] 实现按内容 hash 去重写入。
- [ ] 实现分页查询历史记录。
- [ ] 实现收藏、删除、软删除字段更新。
- [ ] 为插入、去重、分页、搜索写 Rust 测试。
- [ ] 提交：`feat: add sqlite clipboard storage`

### 任务 3：Windows 剪贴板监听

**文件：**
- 创建：`src-tauri/src/clipboard/mod.rs`
- 创建：`src-tauri/src/clipboard/types.rs`
- 创建：`src-tauri/src/clipboard/win.rs`

- [ ] 使用 Windows 原生剪贴板通知机制监听变化。
- [ ] 解析文本内容。
- [ ] 解析 HTML 内容。
- [ ] 解析图片内容。
- [ ] 解析文件列表内容。
- [ ] 将剪贴板内容统一转换为 `ClipboardItemDraft`。
- [ ] 为每条内容计算稳定 hash。
- [ ] 将捕获事件发送到存储写入队列。
- [ ] 为类型识别和 hash 生成写测试。
- [ ] 提交：`feat: capture windows clipboard changes`

### 任务 4：大内容与 blob 存储

**文件：**
- 创建：`src-tauri/src/blobs/mod.rs`

- [ ] 为图片和大二进制内容创建本地 blob 存储目录。
- [ ] 将大图片写入文件系统。
- [ ] 在 SQLite 中保存 blob 路径、大小、hash、预览信息。
- [ ] 异步生成图片缩略图。
- [ ] 实现删除历史后的 blob 清理逻辑。
- [ ] 为 blob 路径生成和清理逻辑写测试。
- [ ] 提交：`feat: store large clipboard payloads as blobs`

### 任务 5：Tauri 命令接口

**文件：**
- 修改：`src-tauri/src/main.rs`
- 创建：`src-tauri/src/commands.rs`

- [ ] 添加 `search_items(query, filters, limit, cursor)`。
- [ ] 添加 `get_item_detail(id)`。
- [ ] 添加 `copy_item(id)`。
- [ ] 添加 `paste_item(id)`。
- [ ] 添加 `toggle_favorite(id)`。
- [ ] 添加 `delete_item(id)`。
- [ ] 添加 `set_recording_enabled(enabled)`。
- [ ] 保持 IPC 参数和返回值窄而明确。
- [ ] 提交：`feat: expose clipboard commands to frontend`

### 任务 6：系统集成

**文件：**
- 创建：`src-tauri/src/system/tray.rs`
- 创建：`src-tauri/src/system/shortcuts.rs`
- 创建：`src-tauri/src/system/settings.rs`

- [ ] 添加托盘菜单：显示窗口、暂停/恢复记录、清空历史、设置、退出。
- [ ] 添加全局快捷键 `Ctrl+Shift+V`。
- [ ] 添加开机启动设置。
- [ ] 持久化用户设置。
- [ ] 应用启动时恢复快捷键、托盘、监听状态。
- [ ] 提交：`feat: add tray shortcuts and settings`

### 任务 7：主界面

**文件：**
- 创建：`src/App.tsx`
- 创建：`src/features/history/`
- 创建：`src/features/settings/`

- [ ] 构建快速启动面板布局。
- [ ] 添加搜索框。
- [ ] 搜索输入添加 100ms debounce。
- [ ] 添加虚拟列表展示历史记录。
- [ ] 添加过滤器：全部、收藏、文本、图片、文件。
- [ ] 添加键盘导航。
- [ ] 添加复制、粘贴、删除、收藏操作。
- [ ] 添加详情预览面板。
- [ ] 提交：`feat: build clipboard history UI`

### 任务 8：设置界面

**文件：**
- 创建：`src/features/settings/SettingsView.tsx`

- [ ] 添加最大历史条数设置。
- [ ] 添加最大保存天数设置。
- [ ] 添加快捷键展示与修改入口。
- [ ] 添加开机启动开关。
- [ ] 添加暂停记录开关。
- [ ] 添加清空历史操作。
- [ ] 添加数据目录展示。
- [ ] 提交：`feat: add settings view`

### 任务 9：性能优化

**文件：**
- 修改：存储、剪贴板、UI 相关模块。

- [ ] 连续复制 1,000 条文本，确认无明显卡顿。
- [ ] 构造 10,000 条历史数据，确认列表滚动稳定。
- [ ] 验证搜索首屏返回速度。
- [ ] 验证大图片复制不会阻塞 UI。
- [ ] 验证托盘常驻内存符合桌面工具预期。
- [ ] 优化数据库查询索引。
- [ ] 优化图片缩略图加载策略。
- [ ] 提交：`perf: optimize clipboard history hot paths`

### 任务 10：发布准备

**文件：**
- 修改：`README.md`
- 创建：`docs/architecture.md`
- 创建：`docs/privacy.md`

- [ ] 编写架构说明。
- [ ] 编写隐私说明：默认记录全部剪贴板内容。
- [ ] 编写本地数据存储路径说明。
- [ ] 编写快捷键和设置说明。
- [ ] 编写 Windows 打包说明。
- [ ] 运行生产构建。
- [ ] 提交：`docs: add release documentation`
