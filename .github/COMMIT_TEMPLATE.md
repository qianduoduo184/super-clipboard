# Git 提交信息模板

遵循 [Conventional Commits](https://www.conventionalcommits.org/) 规范

## 基本格式

```
<type>(<scope>): <subject>

<body>

<footer>
```

## Type 类型

- `feat`: 新功能
- `fix`: Bug 修复
- `docs`: 文档更新
- `style`: 代码格式（不影响功能，如空格、格式化）
- `refactor`: 重构（既不是新功能也不是 bug 修复）
- `perf`: 性能优化
- `test`: 添加或修改测试
- `build`: 构建系统或外部依赖变更
- `ci`: CI 配置文件和脚本变更
- `chore`: 其他不修改 src 或测试文件的变更
- `revert`: 回滚之前的提交

## Scope 范围（可选）

- `clipboard`: 剪贴板监听和内容读取
- `storage`: 数据库和持久化
- `ui`: 前端界面
- `tray`: 系统托盘
- `shortcut`: 全局快捷键
- `settings`: 设置管理
- `updater`: 自动更新
- `blobs`: 文件和图片存储

## Subject 主题

- 使用祈使句，现在时："add" 而不是 "added" 或 "adds"
- 首字母小写
- 结尾不加句号
- 限制在 72 个字符以内

## Body 正文（可选）

- 解释 **为什么** 做这个改动，而不是 **怎么做的**
- 可以包含与之前行为的对比
- 使用多行，每行不超过 72 个字符

## Footer 页脚（可选）

- 关联 Issue: `Closes #123` 或 `Fixes #123` 或 `Relates to #123`
- Breaking Changes: `BREAKING CHANGE: <description>`

## 示例

### 简单修复
```
fix(clipboard): retry when clipboard is locked by another app
```

### 新功能
```
feat(ui): add drag-and-drop reordering for history items

Users can now reorder clipboard history by dragging items.
Visual feedback is provided during drag operation.

Closes #45
```

### Breaking Change
```
feat(storage)!: migrate to WAL mode for better concurrency

BREAKING CHANGE: Existing databases will be automatically migrated
to WAL mode on first launch. Backup your data before upgrading.

Relates to #67
```

### 性能优化
```
perf(storage): add composite index on (deleted, updated_at)

Reduces search query time from 120ms to 8ms on 10k records.
```

### 回滚
```
revert: feat(ui): add drag-and-drop reordering

This reverts commit abc123def because it causes crashes on
Windows 11 with touch screens.

Relates to #89
```

## 配置 Git 使用此模板

```bash
git config commit.template .github/COMMIT_TEMPLATE.md
```
