# 快速参考：如何高效使用 Claude 开发

> 本指南提取了 `.github/DEV_CHECKLIST.md` 和 `OPTIMIZATION_SUMMARY.md` 的核心内容

## 📝 发指令前的 5 秒自检

在输入框发送前，快速问自己：

1. ✅ **指定了文件/模块吗？** (如 `src/App.tsx`, `clipboard/win.rs`)
2. ✅ **说清楚期望结果了吗？** (做什么，不做什么)
3. ✅ **提到验证方式了吗？** (如何知道改对了)
4. ✅ **有参考示例吗？** (类似功能、其他项目)
5. ✅ **标明优先级了吗？** (多需求时)

## 🎯 高效指令速查表

### ❌ 低效 → ✅ 高效

| 场景 | 低效指令 | 高效指令 |
|------|---------|---------|
| **新功能** | "添加搜索" | "在 SearchBar 组件添加实时全文搜索，使用现有 FTS5 索引，300ms debounce，参考 VS Code 交互" |
| **Bug** | "修复崩溃" | "修复 clipboard/win.rs 读取失败：快速复制时丢失内容，添加 40/80/120ms 重试处理占用" |
| **优化** | "优化性能" | "优化 search_items SQL 查询：添加 (deleted, updated_at) 复合索引，10 万条记录从 2s 降至 <100ms" |
| **文档** | "更新文档" | "在 CLAUDE.md Commands 章节补充 cargo test 单独运行方式和输出说明" |

## 🚀 常用指令模板

### 新功能（复制改名直接用）

```
在 [文件/模块] 实现 [功能名]：
- 核心逻辑：[1-2 句话]
- 交互方式：[用户怎么触发]
- 边界情况：[错误、空值、极端输入如何处理]
- 测试要求：[单元测试覆盖什么]
- 参考实现：[类似功能或项目]
```

### Bug 修复

```
修复 [模块] 的 [问题]：
- 当前行为：[错误现象]
- 期望行为：[正确结果]
- 复现步骤：[1、2、3...]
- 可能原因：[你的初步分析]
- 修复范围：[只改这里 / 需同步其他地方]
```

### 批量任务

```
批量处理以下任务（按顺序）：
1. [任务 A] - 验证: npm test
2. [任务 B] - 依赖任务 A 的输出
3. [任务 C] - 独立，可并行
完成后统一提交: feat(ui): ...
```

## 💡 充分利用 AI 能力

### 1. 技术选型
```
"对比 windows-rs / clipboard-win / arboard 三种剪贴板库，
给出性能、维护活跃度、API 易用性的评分表，推荐最适合的"
```

### 2. Code Review
```
"Review 最近 3 次涉及 unsafe 的提交，检查：
- 内存安全隐患
- 能否用 safe 替代
- SAFETY 注释是否充分"
```

### 3. 性能分析
```
"分析 search_items 在 10 万条记录下的瓶颈，
给出 Profiling 解读、索引优化、查询重写方案"
```

### 4. 批量测试生成
```
"为 clipboard/win.rs 生成单元测试覆盖：
- 文本（ASCII/Unicode/Emoji）
- 图片（PNG/BMP/JPEG）
- 文件列表（单个/多个/路径含空格）
- 剪贴板被占用时的重试"
```

## 📦 提交规范速查

### 标准格式
```
<type>(<scope>): <subject>

<body>

Closes #123
```

### Type 类型
- `feat`: 新功能
- `fix`: Bug 修复
- `perf`: 性能优化
- `refactor`: 重构
- `docs`: 文档
- `test`: 测试
- `build/ci`: 构建/CI

### Scope（本项目）
- `clipboard`, `storage`, `ui`, `tray`, `shortcut`, `settings`, `updater`, `blobs`

### 示例
```bash
# 简单
fix(clipboard): retry when locked

# 中等
feat(ui): add drag-and-drop reordering

Users can now reorder history by dragging.

Closes #45

# Breaking
feat(storage)!: migrate to WAL mode

BREAKING CHANGE: Databases auto-migrate on first launch.
```

## 🔄 阶段性开发（大功能）

```markdown
### 阶段 1：调研（20%）
"分析 [方案 A] vs [方案 B]，给出推荐"

### 阶段 2：骨架（40%）
"实现核心逻辑，用 TODO 占位错误处理"

### 阶段 3：完善（70%）
"补充错误处理、边界情况、单元测试"

### 阶段 4：集成（90%）
"集成到 UI，更新文档"

### 阶段 5：Review（100%）
"Review 所有改动，检查安全性和性能"
```

## ✅ 提交前检查（3 秒扫一眼）

- [ ] `npm test` + `npm run typecheck` 通过
- [ ] 提交信息遵循 `type(scope): subject` 格式
- [ ] 改动是原子的（一个提交只做一件事）
- [ ] 关联了 Issue（如果有）：`Closes #N`
- [ ] 更新了 CHANGELOG（面向用户的改动）

## 📚 更多细节

- **完整 Checklist**: `.github/DEV_CHECKLIST.md`
- **提交模板**: `.github/COMMIT_TEMPLATE.md`
- **贡献指南**: `CONTRIBUTING.md`
- **优化总结**: `.github/OPTIMIZATION_SUMMARY.md`

---

**记住**：清晰的指令 = 更快的迭代 = 更少的返工 ⚡
