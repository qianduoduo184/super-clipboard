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
- 搜索走 FTS5。
- 列表分页加载，前端只渲染当前可见数据。
- 大内容异步落盘并使用缩略图预览。
