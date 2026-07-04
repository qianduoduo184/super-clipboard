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
- [x] 添加 1,000 条文本插入和搜索性能测试（后端）。
- [x] 添加 10,000 条历史记录 SQLite 查询性能测试（后端）。
- [ ] 实机测试连续复制 1,000 条文本。
- [ ] 实机测试 10,000 条历史记录 SQLite 查询与界面滚动。
- [ ] 测试大图片捕获。
- [ ] 测试文件列表捕获。
- [ ] 测试暂停/恢复记录。
- [x] 修复托盘图标显示为空白的问题（已添加 `icon(app.default_window_icon()...)`）。
- [ ] 测试托盘生命周期。
- [ ] 测试全局快捷键生命周期。
- [x] 运行前端生产构建。
- [x] 安装 Rust 工具链后运行 Tauri/Rust 构建和测试。
- [x] 修复 FTS5 CJK 子字符串搜索（使用 trigram 分词器）。

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

## 阶段 8：用户体验改进（2026-06-25）

### 🔴 必须修复

- [x] **拖拽功能无效**：无法实现拖拽文本排序和 nav 排序，需检查拖放事件处理逻辑和相关权限配置。已修复：提升拖拽手柄默认可见度（opacity 从 0 改为 0.3），改善用户体验。
- [x] **图片内容和时间重叠**：优化复制时间的展示位置，确保不会和图片内容或其他任何内容重叠，影响可读性。已修复（2026-06-25）：使用 flexbox 垂直布局，`flex-direction: column` + `justify-content: space-between`，确保内容和时间上下分离。
- [x] **拖动排序位置不符合预期**：已修复（2026-06-25）：修改 `reorderItemsByDrag` 逻辑，从在目标之前插入改为在目标之后插入，符合用户拖放操作的直觉。
- [x] **数据丢失问题**：分析数据丢失只剩几条，且复制时间全部显示为一分钟前的原因并修复。可能涉及 SQLite 清理策略误触发、时间戳字段异常、或前端缓存/状态管理问题。已修复：后端使用微秒时间戳，前端期望毫秒时间戳，在 `mapBackendItemToViewItem` 中添加 `Math.floor(item.updated_at / 1000)` 转换。

### 🟡 应当修复

- [x] **可调整预览框大小**：允许通过拖动设置预览框的大小，提升用户对详情区域的控制灵活性。已实现：添加可拖动分隔条，支持动态调整历史列表和预览面板的宽度比例（范围 25%-75%）。
- [x] **智能 ESC 键处理**：已实现（2026-06-25）：第一次 ESC 清空搜索框，第二次 ESC 关闭窗口，提供更好的键盘交互体验。
- [x] **窗口失焦自动隐藏**：已实现（2026-06-25）：使用 `onFocusChanged` 监听窗口焦点，失焦时自动清空搜索并隐藏窗口，类似 Spotlight/Alfred 的快捷启动体验。

## 阶段 9：安全加固（2026-06-25，v1.0.7）

### 🔴 关键安全漏洞修复

- [x] **图片路径遍历攻击**（Critical）：`read_blob` 命令缺少路径验证，攻击者可通过 `../../` 读取任意文件。已修复：添加路径规范化和边界验证，确保只能读取 blob 目录内的文件。
- [x] **目录迁移 TOCTOU 竞态**（High）：`migrate_directory` 在验证和操作之间存在时间窗口，攻击者可在验证后替换为符号链接。已修复：添加符号链接检测，消除攻击窗口。
- [x] **FTS5 查询注入**（Medium）：用户输入直接拼接到 FTS5 MATCH 子句，可能导致 SQL 注入。已修复：实现严格字符白名单过滤，支持 CJK 和常见符号，防止注入攻击。
- [x] **剪贴板内存耗尽**（High）：恶意应用可构造超大剪贴板内容导致内存耗尽。已修复：限制文本 200MB，二进制 500MB，超过限制拒绝读取。
- [x] **安全测试套件**：新增 `src-tauri/src/security_tests.rs`，包含 4 个安全测试用例，验证所有修复有效。

## 阶段 10：待完成任务

### 🔧 功能完善

- [x] **文件列表写回剪贴板**（2026-07-03 复核：**已实现**，文档此前描述过时）：`copy_item` 已支持 image（BMP blob → `CF_DIB`）与 files（JSON 路径数组 → `CF_HDROP`），`commands.rs:103-145` + `clipboard/win.rs:188-269`。捕获侧 `read_current_clipboard` 也已对应存储。**待办**：`win.rs` 写回路径为 `#[cfg(windows)]`，仅有 blob round-trip 单测，尚未在真机验证实际粘贴效果。
- [ ] **实机大数据量测试**：连续复制 1,000+ 条记录，验证 10,000+ 条历史记录的查询性能和 UI 流畅度。
- [ ] **托盘生命周期测试**：验证托盘图标在各种系统状态下的稳定性（屏幕缩放、DPI 变化、系统休眠恢复等）。
- [ ] **快捷键生命周期测试**：验证全局快捷键在冲突、系统锁屏、UAC 提升等场景下的行为。

### 🎯 长期规划

- [ ] **跨平台支持**：macOS 和 Linux 支持（需要适配各平台的剪贴板 API）
- [ ] **云同步**：可选的加密云同步功能（需要设计隐私保护方案）
- [ ] **更多格式支持**：RTF、图表、音频等富媒体格式
- [ ] **剪贴板加密**：敏感内容加密存储选项
- [ ] **OCR 集成**：图片文字识别和搜索
- [ ] **智能分类**：自动识别和分类剪贴板内容（链接、代码、邮件等）
- [ ] **快捷操作**：对特定类型内容提供快捷操作（如链接直接打开、代码高亮等）

## 阶段 11：安装器与界面显示问题（2026-07-03）

### 🌐 安装器本地化与配置记忆

- [ ] **安装界面改为中文**：当前 NSIS 安装向导为英文界面，需配置为中文（简体）。定位 `src-tauri/tauri.conf.json` 的 `bundle.windows.nsis` 配置，设置 `languages: ["SimpChinese"]`（或提供 `installerLanguages`），必要时补充中文 `.nsh` 语言资源。需实机运行 `npm run tauri build` 打包后验证向导语言。
- [ ] **安装器记忆上次勾选项**：安装向导每次都以默认值呈现复选项（如"创建桌面快捷方式"），未记忆用户上次的选择。例如用户上次取消勾选"生成快捷方式"，本次安装应默认不勾选。需在 NSIS 模板中将复选状态读写到注册表（`ReadRegStr`/`WriteRegStr`，如 `HKCU\Software\super-clipboard`），并在向导页 `.onInit` 时回填。Tauri 默认 NSIS 模板不含此逻辑，需自定义 `installerHooks` 或 `template`。

### 🖥️ 界面显示问题

- [x] **滚动后退出再打开内容区域空白**（2026-07-03）：界面滚动后隐藏窗口，再次打开时内容区域偶发不显示。
  - **根因**：虚拟列表由 React `scrollTop` 状态驱动（`App.tsx:138`，`onScroll` 更新），但 `.history-list` DOM 元素的真实滚动位置从未与之同步。当搜索/刷新副作用触发 `setScrollTop(0)`（`App.tsx:265`，由 `clipboard-changed` 事件或清空搜索引发）时，只重置了 React 状态为 0，DOM 元素仍停留在旧滚动位置（如 800px）。虚拟窗口据此把条目渲染在 `translateY(0)`（长 spacer 顶部），而容器仍下滚 800px，视口落在已渲染条目下方的空白区 → 内容区空白。窗口隐藏后此错位状态保留，重开时复现。
  - **修复**：在 `setScrollTop(0)` 处同步重置 DOM 元素滚动位置 `historyListRef.current.scrollTop = 0`，使状态与 DOM 保持一致。
  - **验证**：`npm run ci:frontend`（typecheck）；实机滚动→触发刷新/隐藏→重开确认内容正常显示。

## 阶段 12：竞品对标与功能补齐（2026-07-03）

> 来源：[competitive-analysis-and-roadmap.md](./competitive-analysis-and-roadmap.md)。对照 Ditto / CopyQ / Windows 剪贴板历史 / Maccy / PasteBar 等，梳理缺陷与可借鉴方案。排序为建议值，实际排期以用户确认为准。

### 🔴 P0 核心短板（竞品普遍具备，本应用缺失，削弱核心可用性）

- [x] ~~**图片 / 文件粘贴写回**~~（2026-07-03 复核：**已在代码中实现**，此条基于过时文档，撤销）：`copy_item` 已支持 image/files 写回，详见阶段 10 对应条目。**仅剩真机验证**未做。
- [x] **富文本 HTML 粘贴写回 + "以纯文本粘贴"开关**（2026-07-03 实现，后端已 `cargo build` 通过；前端 typecheck 待跑）：新增 `write_html_to_clipboard`（`CF_HTML` + `CF_UNICODETEXT` 纯文本回退，`win.rs`），`copy_item`/`paste_item` 增加 `plain_text: Option<bool>` 参数——html 条目默认写 `CF_HTML` 保留格式，`plain_text=true` 时写纯文本。前端 `api.ts` 透传 `plainText`，右键菜单对 html 条目新增"以纯文本粘贴"。**注意**：HTML 捕获在阶段 7 已移除，故富文本路径当前仅惠及历史遗留 html 条目；如需对新内容生效，需另行评估是否重启 HTML 捕获。
- [x] **敏感来源排除**（2026-07-03 实现，后端已 `cargo build` 通过）：`read_current_clipboard` 开头调用 `is_history_excluded()`，检测到 Windows 排除格式 `ExcludeClipboardContentFromMonitorProcessing`（存在即排除）或 `CanIncludeInClipboardHistory`（DWORD=0）即跳过入库，密码管理器（KeePass/1Password/Bitwarden 等）复制的密码不再明文落库。
- [ ] **应用排除名单**（拆自上一条，待实现）：用户可配置按前台进程名/exe 的排除名单。需捕获时取前台进程（`GetForegroundWindow`→`QueryFullProcessImageName`）、`AppSettings` 增 `excluded_apps` 字段、设置页 UI。较独立，单列。

### 🟡 P1 高价值增强

- [ ] **数字键快速粘贴第 N 条**（借鉴 Ditto）：面板打开后按 `1~9` 直接粘贴对应条目，键盘流核心加速器。改动集中在前端 `handleKeyboard`。
- [ ] **片段 / 模板 / 分组（收藏板）**（借鉴 CopyQ tab、PasteBar boards）：把常用内容固定成可命名的组或带变量模板。当前只有扁平的收藏/置顶，缺分组与常驻片段库。需扩展 schema（分组表/模板表）。
- [ ] **内容感知快捷动作**（借鉴 CopyQ command）：识别 URL/邮箱/颜色/路径/代码，提供"打开链接、生成二维码、颜色预览、代码高亮"等一键动作。（与阶段 10「智能分类」「快捷操作」合并。）

### 🟢 P2 效率增强

- [ ] **条目内编辑后再粘贴**（借鉴 Ditto/CopyQ）：粘贴前微调文本。
- [ ] **合并多条粘贴 / 粘贴堆栈**（借鉴 Ditto）：多选合并为一次粘贴，或粘贴后自动指向下一条。
- [ ] **面板贴近光标定位**（借鉴 Ditto）：面板出现在鼠标/插入符附近，当前为固定位置。
- [ ] **多选 + 批量操作**：批量删除/收藏/导出，配合合并粘贴。

### ⚪ P3 长期 / 差异化（多数已在阶段 10 长期规划，此处做对标拆分，不重复）

- [ ] **局域网点对点同步**：作为「云同步」的第一步，无服务器、隐私更友好。
- [ ] **检索增强**：按来源应用、日期范围、正则过滤。
- [ ] **RTF 富文本格式**、**首次运行引导 / 快捷键与权限自检**。
- [ ] 其余（端到端加密、OCR、Emoji/GIF）见阶段 10 长期规划。
