# 贡献指南

感谢你考虑为 super-clipboard 做出贡献！

## 行为准则

- 尊重所有贡献者
- 建设性地提供反馈
- 专注于对项目最有利的事情

## 如何贡献

### 报告 Bug

1. 检查 [Issues](https://github.com/qianduoduo184/super-clipboard/issues) 确保问题未被报告
2. 使用 Bug 报告模板创建新 Issue
3. 提供详细的复现步骤和环境信息
4. 如果可能，附上日志文件（路径在设置页面可见）

### 建议新功能

1. 检查 [Issues](https://github.com/qianduoduo184/super-clipboard/issues) 确保功能未被建议
2. 使用功能请求模板创建新 Issue
3. 清晰描述使用场景和期望收益
4. 如果有参考实现，请提供链接

### 提交代码

#### 准备工作

1. **Fork 仓库**并克隆到本地
   ```bash
   git clone https://github.com/YOUR_USERNAME/super-clipboard.git
   cd super-clipboard
   ```

2. **安装依赖**
   ```bash
   # 前端依赖
   npm install
   
   # Rust 工具链（如果还没有）
   # 访问 https://rustup.rs/ 安装
   ```

3. **创建分支**
   ```bash
   git checkout -b feat/your-feature-name
   # 或
   git checkout -b fix/your-bug-fix
   ```

#### 开发流程

1. **编写代码**
   - 遵循现有代码风格
   - 添加必要的注释
   - 保持提交原子性（一个提交只做一件事）

2. **编写测试**
   - 前端: 在 `src/lib/*.test.js` 添加测试
   - Rust: 在对应模块添加 `#[cfg(test)]` 测试

3. **运行测试**
   ```bash
   # 前端测试
   npm test
   npm run typecheck
   
   # Rust 测试
   cargo test --manifest-path src-tauri/Cargo.toml
   ```

4. **手动测试**
   ```bash
   npm run tauri dev
   ```

#### 提交规范

遵循 [Conventional Commits](https://www.conventionalcommits.org/) 规范：

```
<type>(<scope>): <subject>

<body>

<footer>
```

**Type:**
- `feat`: 新功能
- `fix`: Bug 修复
- `docs`: 文档更新
- `style`: 代码格式
- `refactor`: 重构
- `perf`: 性能优化
- `test`: 测试
- `build`: 构建系统
- `ci`: CI 配置
- `chore`: 其他

**Scope:** (可选)
- `clipboard`, `storage`, `ui`, `tray`, `shortcut`, `settings`, `updater`, `blobs`

**示例:**
```bash
git commit -m "feat(ui): add drag-and-drop reordering

Users can now reorder clipboard history by dragging items.

Closes #45"
```

#### 提交 Pull Request

1. **推送到你的 Fork**
   ```bash
   git push origin feat/your-feature-name
   ```

2. **创建 PR**
   - 访问原仓库页面
   - 点击 "New Pull Request"
   - 选择你的分支
   - 填写 PR 模板

3. **PR 检查清单**
   - [ ] 所有测试通过
   - [ ] 代码遵循项目规范
   - [ ] 添加了必要的文档
   - [ ] 更新了 CHANGELOG.md（如果是面向用户的改动）
   - [ ] PR 标题遵循 Conventional Commits
   - [ ] 关联了相关 Issue

4. **等待 Review**
   - 维护者会尽快 review
   - 根据反馈修改代码
   - 讨论技术方案

## 开发环境

### 必需工具

- **Node.js**: 18+ (推荐 22 LTS)
- **Rust**: 1.96+
- **Windows**: 10/11 (项目目前仅支持 Windows)
- **WebView2**: 通常随 Windows 11 预装

### 可选工具

- **VS Code**: 推荐 IDE
  - 扩展: rust-analyzer, Tauri, ESLint
- **Windows Terminal**: 更好的终端体验

### 项目结构

```
super-clipboard/
├── src/                    # React 前端
│   ├── lib/               # 纯逻辑（带测试）
│   ├── features/          # 功能模块
│   └── App.tsx            # 主组件
├── src-tauri/             # Rust 后端
│   ├── src/
│   │   ├── clipboard/     # 剪贴板监听
│   │   ├── storage/       # 数据库
│   │   ├── system/        # 托盘/快捷键/设置
│   │   ├── blobs/         # 文件存储
│   │   └── commands.rs    # Tauri IPC 命令
│   └── Cargo.toml
├── docs/                  # 文档
├── .github/               # GitHub 配置
└── CLAUDE.md              # AI 助手指南

详细架构说明见 CLAUDE.md
```

## 代码规范

### TypeScript/React

- 使用函数组件和 Hooks
- 优先使用 `.tsx` (组件) 和 `.js` (纯逻辑)
- 纯逻辑模块配套 `.d.ts` 和 `.test.js`
- 使用 `const` 和箭头函数
- 避免 `any`，充分利用类型推导

### Rust

- 遵循 Rust 官方风格指南
- 使用 `cargo fmt` 格式化
- 使用 `cargo clippy` 检查
- `unsafe` 代码必须有 `SAFETY` 注释
- 错误处理使用 `Result` 和 `?`

### 提交粒度

- 一个提交只做一件事
- 提交信息清晰描述改动
- 大功能拆分成多个小提交
- 每个提交都应该能通过测试

## 代码审查标准

维护者会从以下角度 review：

1. **正确性**: 代码是否解决了问题？
2. **测试**: 是否有足够的测试覆盖？
3. **性能**: 是否有性能隐患？
4. **安全**: 是否有安全风险？
5. **可维护性**: 代码是否易读易维护？
6. **向后兼容**: 是否破坏现有 API？

## 发布流程

由维护者负责：

1. 更新版本号（`package.json`, `Cargo.toml`, `tauri.conf.json`）
2. 更新 `CHANGELOG.md`
3. 创建 git tag
4. 推送到 main 触发 CI 构建
5. GitHub Release 自动创建

### 版本管理规则

#### 版本号格式
采用语义化版本（Semantic Versioning）：`MAJOR.MINOR.PATCH`

| 字段 | 含义 | 触发场景 |
|------|------|---------|
| MAJOR | 主版本 | 不兼容的重大架构变更或功能重写 |
| MINOR | 次版本 | 新增功能，向后兼容 |
| PATCH | 修订版本 | Bug 修复、构建发布、小优化 |

#### 当前基准版本
`1.0.0`（正式发布于首次生产发版）

#### 发版规则
- **正式功能发版**：递增 `MINOR`，如 `1.0.0` → `1.1.0`
- **构建发布 / Bug 修复 / 小版本迭代**：递增 `PATCH`，如 `1.0.0` → `1.0.1`
- **重大重构或破坏性变更**：递增 `MAJOR`，如 `1.x.x` → `2.0.0`，需提前告知

#### 版本号同步要求
每次发版必须确保以下所有位置的版本号保持一致：
- `package.json` → `version`
- `src-tauri/tauri.conf.json` → `version`
- `src-tauri/Cargo.toml` → `version`

#### AI 协作约定
在与 AI（如 Claude）协作开发时：
- 每次发版前，告知 AI 本次发版类型（修复 / 新功能 / 重大变更），由 AI 自动推导正确的版本号
- AI 在执行发版任务时，必须同步更新上述所有版本号字段，不允许只更新部分文件
- AI 不得在未明确告知发版类型的情况下自行递增版本号

## 获取帮助

- **问题**: 在 [Issues](https://github.com/qianduoduo184/super-clipboard/issues) 提问
- **讨论**: 使用 GitHub Discussions（如果启用）
- **安全问题**: 请私下联系维护者（不要公开 Issue）

## 许可证

通过提交代码，你同意你的贡献将使用与项目相同的许可证。

---

再次感谢你的贡献！🎉
