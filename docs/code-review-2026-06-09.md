# 代码审查报告 — 2026-06-09

审查范围：`src-tauri/`（Rust 后端）与 `src/`（React/TS 前端）全量源码。
审查维度：安全、性能、代码质量、测试。

---

## 🔴 严重（必须修复）

### 1. FTS5 搜索遇到特殊字符会报错，导致核心搜索功能失效
- **位置：** `src-tauri/src/storage/repository.rs:197-201`
- **现象：** 用户输入的 `query` 被直接作为 `clipboard_items_fts MATCH :query` 的匹配表达式。虽然走了参数绑定（**没有 SQL 注入风险**），但 FTS5 的 MATCH 有自己的查询语法。当查询包含 FTS5 保留字符时会抛出语法错误：
  - 含 `:` 的内容（如搜索网址 `http://...`）→ FTS5 把 `http` 当成列名 → `no such column: http`。
  - 未配对的引号 `"`、括号 `(` `)`、`AND` / `OR` / `NOT` / `*` 等 → `fts5: syntax error`。
- **连锁影响：** 该错误以 `Err(String)` 返回前端，`App.tsx:179` 的 `.catch` 会把它当作"后端不可用"，于是 `setBackendAvailable(false)` 并回退到**示例数据**（`seedItems`）。用户只是搜了个带冒号的词，界面却切换成演示数据，体验严重错乱。
- **修复建议：** 在传入 MATCH 前对查询做转义/包裹。最简单稳妥的做法是把用户输入按空白切分后，每个 token 用双引号包裹并转义内部双引号，拼成 FTS5 短语查询：
  ```rust
  fn to_fts_query(raw: &str) -> String {
      raw.split_whitespace()
          .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
          .collect::<Vec<_>>()
          .join(" ")
  }
  ```
  并为典型特殊字符输入补充单元测试。

### 2. “开机启动”设置项完全不生效
- **位置：** `src-tauri/src/system/settings.rs:10`、`src-tauri/src/commands.rs:102-125`、`src-tauri/src/main.rs:33-36`
- **现象：** `autostart_enabled` 被持久化并随 `update_settings` 保存，但**没有任何代码调用 `tauri_plugin_autostart` 的 `enable()` / `disable()`**。`main.rs` 仅做了 `tauri_plugin_autostart::init(...)`，`update_settings` 只做了保存设置 + 历史清理。
- **影响：** 设置页 `SettingsView.tsx:267-280` 的“开机启动”开关是个空操作；UI 文案承诺“登录 Windows 后自动启动”，实际不会写入任何自启动项。这是功能性缺陷。
- **修复建议：** 在 `update_settings`（或新增专用命令）中，根据 `next_settings.autostart_enabled` 调用 `app.autolaunch().enable()/disable()`，并处理错误。补充对应测试或至少在日志中记录结果。

---

## 🟡 建议（应当考虑）

### 3. 删除 / 清理历史时不回收 blob 文件，磁盘无限增长
- **位置：** `src-tauri/src/storage/repository.rs:233-278`（`soft_delete` / `prune_history`）
- **现象：** 两者都只设置 `deleted_at` 并从 FTS 移除，**从不删除图片对应的 `content_path`（.bmp）及其 `.thumb.png` 缩略图**。只有 `clear_history`（`commands.rs:149`）会清空整个 blob 目录。
- **影响：** 单条删除、按条数/天数自动清理后，图片 blob 与缩略图持续堆积。设置页文案“删除历史记录和未引用 blob 文件”暗示存在 GC，但实际没有。
- **修复建议：** 在软删除/清理选中行之前，先查出这些行的 `content_path`，物理删除对应 blob 及 `thumbnail_path_for(...)`（已有 `remove_blob_if_exists` 可复用）。

### 4. 一次复制会生成多条历史记录
- **位置：** `src-tauri/src/clipboard/win.rs:60-128`
- **现象：** `read_current_clipboard` 对每种可用格式各 push 一个 draft。复制富文本时剪贴板通常同时含 `CF_UNICODETEXT` 和 `HTML Format`，于是一次 Ctrl+C 会产生 2 条记录（文本 + HTML）；复制图片时也可能同时落库 DIB 和文本。
- **影响：** 历史列表出现看似重复的条目。
- **修复建议：** 明确捕获优先级（如图片 > 文件 > HTML > 文本，命中一种即停止），或将同源多格式合并为一条记录。

### 5. 文件列表的 `size_bytes` 存的是文件数量而非字节数
- **位置：** `src-tauri/src/clipboard/win.rs:119`（`size_bytes: files.len() as i64`）
- **现象：** 对 Files 类型，`size_bytes` 被赋成文件个数。前端 `clipboard-adapter.js` 的 `formatBytes` 会把它当字节渲染，于是“2 个文件”显示成 `2 B`。
- **修复建议：** 要么累加各文件真实大小，要么为 Files 类型在前端单独渲染为“N 个文件”。

### 6. 缩略图已生成但前端从未使用
- **位置：** `src-tauri/src/blobs/mod.rs:62-67` 生成 `.thumb.png`；前端 `App.tsx`/`clipboard-adapter.js` 完全忽略 `content_path`，图片仅以文件名文本显示在 `<pre>` 中。
- **影响：** 架构文档承诺“缩略图预览”未兑现，且每张图片都在做无人消费的缩略图计算与磁盘写入。
- **修复建议：** 前端按 `content_path` 渲染图片/缩略图预览；在此之前可视为已知 TODO（`docs/TODO.md` 阶段 5 实机测试项相关）。

### 7. 写入剪贴板无重试，粘贴可能偶发失败
- **位置：** `src-tauri/src/clipboard/win.rs:130-157`（`write_text_to_clipboard`）
- **现象：** 读取路径有 40→300ms 的退避重试（`clipboard/mod.rs:58-86`），但写入路径的 `OpenClipboard` 失败即直接返回错误。当其他应用短暂占用剪贴板时，复制/粘贴会失败。
- **修复建议：** 为 `ClipboardGuard::open()` 增加短重试，或在 `copy_item`/`paste_item` 命令层包裹重试。

---

## 🟢 细节（可选）

### 8. `search()` 混用命名占位符与位置绑定
- **位置：** `repository.rs:171-209`
- SQL 用了 `:item_type` / `:cursor` / `:query` / `:limit`，却通过 `params_from_iter`（按位置 ?1、?2… 绑定）传参。当前因压入 vec 的顺序与出现顺序一致而能正常工作，但命名占位符却走位置绑定容易误导后续维护者；调整任一处顺序就会出错。建议统一改用 `named_params!` 或全部改为 `?`。

### 9. `if is_some() { ... unwrap_or_default() }` 可简化
- **位置：** `repository.rs:189-195`
- 用 `if let Some(item_type) = filters.item_type` 一次取出更清晰，避免 `is_some()` + `unwrap_or_default()` 的冗余。

### 10. 监听窗口 `WM_DESTROY` 未 `PostQuitMessage`
- **位置：** `win.rs:232`
- 当前消息窗口生命周期内不会被销毁，影响有限；若将来需要优雅退出监听线程，`WM_DESTROY` 分支应调用 `PostQuitMessage(0)` 以让 `GetMessageW` 循环退出。

---

## 测试缺口

- **缺少 FTS 特殊字符搜索测试**（对应严重问题 #1）：现有 `search_returns_matching_items` 只覆盖了普通词，未覆盖 `:`、引号、括号等会触发 FTS5 语法错误的输入。
- **缺少自启动开关行为的覆盖**（对应严重问题 #2，目前根本未接线）。
- `win.rs` 的 Windows 原生路径无测试（平台相关，可接受）；但 `read_current_clipboard` 的多格式产出逻辑（#4）可抽象出纯函数后做单元测试。

---

## ✅ 做得好的地方

- **全程参数化 SQL**（`params!` / `params_from_iter`），不存在 SQL 注入。
- **DIB→BMP 解析的边界与溢出处理严谨**：`blobs/mod.rs` 使用 `slice.get(..)`、`checked_add`、对 `u32::MAX` 与 header 大小做了校验，避免越界与整型溢出。
- **`ClipboardGuard` 用 RAII 保证 `OpenClipboard`/`CloseClipboard` 配对**，避免泄漏剪贴板句柄。
- **剪贴板读取退避重试**，应对源应用尚未释放剪贴板的竞态。
- **快捷键替换失败可回滚**到旧快捷键（`shortcuts.rs:65-74`），不会让用户陷入无快捷键状态。
- **前端优雅降级**：后端不可用时回退示例数据；纯逻辑层（`src/lib/*.js`）测试覆盖良好，含 1,000/10,000 条规模的性能与虚拟窗口测试。
- **软删除唯一索引设计正确**：`idx_clipboard_items_active_hash ... WHERE deleted_at IS NULL` 允许同内容在软删除后重新入库（已有针对性测试）。
