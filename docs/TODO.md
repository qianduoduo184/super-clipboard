# super-clipboard TODO

## 阶段 1：项目基础

- [x] 初始化 Tauri 2 + React + TypeScript 项目。
- [x] 配置 Windows 应用基础信息。
- [x] 配置基础应用窗口。
- [x] 配置快速启动面板窗口。
- [x] 创建 README。
- [x] 验证前端开发/构建配置可用。

## 阶段 2：后端核心

- [x] 添加 SQLite 存储源码。
- [x] 启用 SQLite WAL 模式源码。
- [x] 添加剪贴板历史表。
- [x] 添加 FTS5 搜索表。
- [x] 添加按 hash 去重逻辑。
- [x] 修复软删除后同 hash 内容无法再次记录的索引设计。
- [x] 添加分页查询逻辑。
- [x] 添加收藏逻辑。
- [x] 添加软删除逻辑。
- [x] 添加最大历史条数/最长保存时间清理策略源码。
- [x] 添加 Windows 剪贴板监听接口占位。
- [x] 实现 Windows 原生剪贴板通知和内容读取源码。
- [x] 添加文本捕获源码。
- [x] 添加 HTML 捕获源码。
- [x] 添加图片捕获源码。
- [x] 添加文件列表捕获源码。
- [x] 添加 blob 存储。
- [x] 添加图片缩略图生成源码。

## 阶段 3：应用集成

- [x] 添加 Tauri 命令：`search_items`。
- [x] 添加 Tauri 命令：`get_item_detail`。
- [x] 添加 Tauri 命令：`copy_item`。
- [x] 添加 Tauri 命令：`paste_item`。
- [x] 添加 Tauri 命令：`toggle_favorite`。
- [x] 添加 Tauri 命令：`delete_item`。
- [x] 添加 Tauri 命令：`set_recording_enabled`。
- [x] 添加托盘菜单源码。
- [x] 添加全局快捷键 `Ctrl+Shift+V` 源码。
- [x] 添加开机启动插件配置。
- [x] 添加设置持久化结构。
- [x] 保存设置后触发历史清理策略。

## 阶段 4：前端界面

- [x] 构建主启动面板。
- [x] 添加搜索输入框。
- [x] 添加搜索 debounce。
- [x] 添加虚拟历史列表。
- [x] 添加类型过滤。
- [x] 添加收藏过滤。
- [x] 添加详情预览。
- [x] 添加完整键盘导航。
- [x] 添加复制操作入口。
- [x] 添加粘贴操作入口。
- [x] 添加删除操作。
- [x] 添加收藏/取消收藏操作。
- [x] 保留 Tauri 字符串错误原文，避免复制/粘贴失败原因丢失。
- [x] 构建设置页面。
- [x] 清空历史前添加确认。

## 阶段 5：质量与性能

- [x] 添加 Rust 存储层单元测试源码。
- [x] 添加 Rust 软删除重录和保留策略测试源码。
- [x] 添加前端剪贴板模型归一化测试。
- [x] 添加前端搜索/过滤测试。
- [x] 添加前端错误文案和清空确认测试。
- [x] 添加 Rust blob 路径/缩略图测试源码。
- [x] 添加 1,000 条文本记录前端过滤性能自动化测试。
- [x] 添加 10,000 条历史记录虚拟列表窗口自动化测试。
- [ ] 实机测试连续复制 1,000 条文本。
- [ ] 实机测试 10,000 条历史记录 SQLite 查询与界面滚动。
- [ ] 测试大图片捕获。
- [ ] 测试文件列表捕获。
- [ ] 测试暂停/恢复记录。
- [ ] 测试托盘生命周期。
- [ ] 测试全局快捷键生命周期。
- [x] 运行前端生产构建。
- [ ] 安装 Rust 工具链后运行 Tauri/Rust 构建和测试。

## 阶段 6：文档

- [x] 编写架构文档。
- [x] 编写隐私行为文档。
- [x] 编写本地数据路径文档。
- [x] 编写快捷键文档。
- [x] 编写 Windows 打包文档。

## 阶段 7：代码审查发现（2026-06-10，分支 codex-clipboard-behavior-ui-updates）

> 来源：对「自动更新 / 图片复制 / 移除 HTML 捕获 / 拖拽排序 / 粘贴后隐藏窗口」五块改动的代码审查。
>
> 修复状态（2026-06-10）：🔴+🟡 五项已修复。前端经 `npm run ci:frontend`（31 测试 + typecheck）通过；后端改动（capabilities/`commands.rs`）本地无 Rust 工具链，待装链后跑 `cargo test` 验证（含新增 `html_to_plain_text_keeps_text_between_bare_ampersands`）。🟢 三项为设计权衡，暂不处理。

### 🔴 必须修复

- [x] **窗口隐藏缺权限**（`src/App.tsx:290`，`src-tauri/capabilities/default.json`）：前端 `getCurrentWindow().hide()` 缺 `core:window:allow-hide`，会抛权限错误；因调用在 `pasteItem` 之后的同一 try 内，错误被 catch 误报「粘贴失败」且窗口不隐藏（粘贴其实成功）。修复：capabilities 加 `"core:window:allow-hide"` 并实机验证。

### 🟡 应当修复

- [x] **拖拽后吞掉一次点击**（`src/App.tsx:303-323` `handleDropItem` / `:455` `onClick`）：drop 时无条件置 `suppressNextItemClick=true`，但 HTML5 拖放结束不触发 click，该标志挂起到下次真实点击才被消费，导致拖拽后第一次粘贴点击失效。修复：去掉该标志，或在 `onDragEnd` 用微任务重置。
- [x] **html_to_plain_text 丢字**（`src-tauri/src/commands.rs:270`）：多个裸 `&` 之间的正文丢失（如 `"A & B & C"` → `"A & C"`），因第二个 `&` 清空了 entity 缓冲。仅影响复制遗留 HTML 条目。修复：遇到非 `;` 结尾的 `&` 串时把缓冲原样 flush 回输出。
- [x] **last_update_check_date 被前端旧值覆盖**（`src-tauri/src/commands.rs:136` `update_settings` ↔ `:170` `check_update`）：`check_update` 写入今日值但不回传前端；用户改任意设置时 `update_settings` 用旧值覆盖，破坏「每天一次」。修复：`update_settings` 保留现有 `last_update_check_date`（仿 `global_shortcut`），或 `check_update` 回传新日期供前端同步。
- [x] **每日检查日期 UTC/本地不一致**（`src/App.tsx:146` 用 `toISOString()`(UTC) ↔ `src-tauri/src/commands.rs:170` 用 `chrono::Local`）：跨午夜边界可能误判是否已检查。修复：两侧统一日期来源。

### 🟢 可选

- [ ] `capabilities/default.json:11` `updater:default` 可能多余（更新经 Rust 侧 `app.updater()` 调用，不走 JS→插件 IPC），确认后可移除。
- [ ] `src/App.tsx:93-95` `getVisibleFilters()` 结果用 `as Array<{ key: FilterType; ... }>` 强转，丢失类型安全；可让 `.d.ts` 直接返回 `FilterType`。
- [ ] 筛选/搜索激活时拖拽排序会把可见子集的 `sort_rank` 抬到全局最前，清空筛选后顺序可能与预期不符（设计权衡，留意即可）。
