# Release 说明

super-clipboard 使用 GitHub Actions 自动构建 Windows 成品并上传到 GitHub Release。

## 自动发版策略

每次推送到 `main` 分支后会触发 `.github/workflows/release.yml`：

1. 检出仓库。
2. 安装 Node.js 22。
3. 安装 Rust stable。
4. 执行 `npm ci`。
5. 执行 `npm test`。
6. 执行 `npx tsc --noEmit`。
7. 使用 `tauri-apps/tauri-action` 构建 Tauri Windows 应用。
8. 创建 GitHub prerelease，并上传 Tauri 生成的安装包。

自动发版 tag 格式：

```text
super-clipboard-v<应用版本>-<GitHub Actions run number>
```

示例：

```text
super-clipboard-v0.1.0-12
```

## 手动发版

也可以在 GitHub 仓库页面手动触发：

1. 打开 GitHub 仓库。
2. 进入 `Actions`。
3. 选择 `Release` workflow。
4. 点击 `Run workflow`。

## 本地验证

推送前建议至少运行：

```powershell
npm test
npx tsc --noEmit
npm run build
```

本地 Tauri 构建需要 Rust 工具链：

```powershell
npm run tauri build
```

## 成品位置

GitHub Actions 构建完成后，安装包会出现在 GitHub Release 资产中。

本地构建时，Tauri 默认输出目录为：

```text
src-tauri/target/release/bundle/
```

## 失败排查

- `npm ci` 失败：检查 `package-lock.json` 是否与 `package.json` 同步。
- `npm test` 失败：先修复前端单元测试。
- `npx tsc --noEmit` 失败：先修复 TypeScript 类型错误。
- Tauri 构建失败：优先检查 Rust 依赖、Windows API 调用、`tauri.conf.json` 和图标/打包配置。
- Release 上传失败：检查 GitHub Actions `permissions.contents: write` 是否存在。

## 当前约定

- 每次任务产生代码或文档修改后，完成验证即提交并推送。
- 每次推送到 `main` 后，由 GitHub Actions 自动生成 prerelease。
- 当前项目仍处于早期版本，Release 默认标记为 prerelease。
