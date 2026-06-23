# super-clipboard 架构说明

super-clipboard 是 Windows 优先的独立剪贴板管理器。应用使用 Tauri 2 承载桌面窗口，Rust 负责常驻系统能力，React/TypeScript 负责用户界面。

## 模块边界

- `clipboard`：监听 Windows 剪贴板变化，归一化文本、HTML、图片、文件列表。
- `storage`：初始化 SQLite，维护历史表、FTS5 搜索表、分页查询、收藏和软删除。
- `blobs`：保存图片和大二进制内容，避免把大 DataURL 写入数据库。
- `commands`：暴露窄 Tauri IPC，前端不直接访问数据库或文件系统。
- `system`：管理托盘、全局快捷键、开机启动和用户设置。
- `src/`：React 快速面板和设置页。

## 性能原则

- 监听使用系统通知，不以轮询作为主路径。
- SQLite 使用 WAL 模式。
- 搜索走 FTS5 混合策略（短查询用 LIKE，长查询用 trigram）。
- 列表分页加载，前端只渲染当前可见数据。
- 大内容异步落盘并使用缩略图预览。

## 搜索功能

### 混合搜索策略

为了支持各种长度的查询和不同语言，搜索使用了混合策略：

**短查询（< 3 字符）**:
- 使用 SQL `LIKE '%query%'` 进行子字符串匹配
- 同时搜索 `preview` 和 `content` 字段
- 适用场景：
  - 单字符搜索（如 "a", "中"）
  - 两字符搜索（如 "ab", "测试"）
  - 英文缩写（如 "JS", "UI"）

**长查询（≥ 3 字符）**:
- 使用 FTS5 trigram 分词器进行全文搜索
- 支持 CJK（中日韩）子字符串匹配
- 性能优异，即使在 10,000+ 条记录中也能在 50ms 内完成
- 适用场景：
  - 完整词语搜索（如 "clipboard", "剪贴板"）
  - 短语搜索（如 "test content", "测试内容"）
  - URL 关键词（如 "example", "github"）

**示例**:
```rust
// 搜索 "ab" - 使用 LIKE，能匹配 "123ab332sddsdf"
search("ab", filters)  // 找到包含 "ab" 的所有记录

// 搜索 "云之家" - 使用 trigram，能匹配 "同步组织排序码到云之家"
search("云之家", filters)  // 找到包含这 3 个字的记录

// 搜索 "3ab" - 使用 trigram，精确子串匹配
search("3ab", filters)  // 找到 "123ab332sddsdf"
```

### FTS5 配置

```sql
CREATE VIRTUAL TABLE clipboard_items_fts
USING fts5(id UNINDEXED, preview, content, tokenize='trigram');
```

- **tokenize='trigram'**: 创建 3 字符的 n-gram token
- **id UNINDEXED**: id 字段不参与搜索，仅用于关联
- **preview, content**: 两个字段都参与全文搜索

### 性能特征

| 场景 | 策略 | 性能 | 说明 |
|------|------|------|------|
| 1-2 字符查询 | LIKE | ~10ms (10k 条) | 全表扫描，但数据量小时可接受 |
| 3+ 字符查询 | FTS5 trigram | ~50ms (10k 条) | 使用索引，性能稳定 |
| CJK 子串 | FTS5 trigram | ~50ms (10k 条) | trigram 天然支持 |
| 空查询 | 索引扫描 | ~7ms (10k 条) | 只按时间排序 |
