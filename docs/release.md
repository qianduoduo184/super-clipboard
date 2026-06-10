# Release 说明

super-clipboard 使用 GitHub Actions 自动构建 Windows 成品并上传到 GitHub Release。

## 自动发版策略

每次推送到 `main` 分支后会触发 `.github/workflows/release.yml`。如果仓库还没有配置 updater signing Secret，workflow 只执行前端检查并跳过签名发布构建；配置好 Secret 后才会构建并上传正式 Release。

完整发布流程：

1. 检出仓库。
2. 安装 Node.js 22。
3. 安装 Rust stable。
4. 执行 `npm ci`。
5. 执行 `npm test`。
6. 执行 `npx tsc --noEmit`。
7. 检查 updater signing Secret 是否存在。
8. Secret 存在时，预检 updater 私钥格式和密码。
9. 使用 `tauri-apps/tauri-action@v0` 构建 Tauri Windows 应用。
10. 创建 GitHub Release，并上传 Tauri 生成的安装包。

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
- Release 构建被跳过：配置 GitHub Actions Secrets `TAURI_SIGNING_PRIVATE_KEY` 后重新运行 workflow。
- Release 上传失败：检查 GitHub Actions `permissions.contents: write` 是否存在。
- Action 解析失败：确认 `.github/workflows/release.yml` 使用的是存在的 `tauri-apps/tauri-action` tag，例如 `v0`。当前不要使用 `v1`，因为上游仓库没有对应 tag。
- Cargo 入口失败：当前应用使用 `src-tauri/src/main.rs` 作为二进制入口，不声明额外 `[lib]` crate。

## 当前约定

- 每次任务产生代码或文档修改后，完成验证即提交并推送。
- 每次推送到 `main` 后，由 GitHub Actions 自动生成正式 Release。
- 自动更新依赖 GitHub latest release endpoint；如果改回 prerelease，需要同步调整 updater endpoint。

## 自动更新签名

Tauri updater 需要签名校验。当前 `src-tauri/tauri.conf.json` 已配置 GitHub Release endpoint 和 updater 公钥。

发布前需要在 GitHub Secrets 配置：

```text
TAURI_SIGNING_PRIVATE_KEY
TAURI_SIGNING_PRIVATE_KEY_PASSWORD
```

`TAURI_SIGNING_PRIVATE_KEY` 必须填写 updater 私钥文件的完整内容，也就是 `npx tauri signer generate` 生成的 `.key` 文件内容。不要填写 `.key.pub` 公钥内容，也不要填写本地文件路径。

当前私钥内容是 base64 文本，解码后应以 `untrusted comment:` 开头并包含 `secret key`。如果 GitHub Actions 报错 `Missing comment in secret key`，通常表示 `TAURI_SIGNING_PRIVATE_KEY` 填错、被截断，或填成了公钥。如果报错 `Wrong password for that key`，则表示 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 与生成私钥时输入的密码不一致。

如果生成密钥时未设置密码，`TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 可以留空；如果不确定密码，重新生成密钥对并同步更新 Secret 和 `src-tauri/tauri.conf.json` 中的公钥。

本地可用以下命令生成密钥对：

```powershell
npx tauri signer generate -w .tmp\super-clipboard-updater.key
```

将生成的公钥写入 `plugins.updater.pubkey`，私钥只保存到 GitHub Secrets 或安全的本地密钥库，不能提交到仓库。

提交前可用下面的命令在本地验证私钥格式和密码是否可用：

```powershell
"updater signing preflight" | Set-Content .tmp\updater-signing-preflight.txt
$env:TAURI_PRIVATE_KEY = (Get-Content .tmp\super-clipboard-updater.key -Raw).Trim()
$env:TAURI_PRIVATE_KEY_PASSWORD = ""
npx tauri signer sign .tmp\updater-signing-preflight.txt
```
