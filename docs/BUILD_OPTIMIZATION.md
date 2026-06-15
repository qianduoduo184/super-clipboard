# 构建优化说明

## 优化成果

- **前端构建时间**: 6 秒 → **1.44 秒** (提升 76%)
- **预期 Rust 构建**: 从 6+ 分钟优化到约 2-3 分钟

## 已实施的优化

### 1. Rust/Cargo 优化 (`src-tauri/Cargo.toml`)

#### Release Profile 优化
- `codegen-units = 16`: 从 256 降低，平衡编译速度和运行性能
- `lto = "thin"`: 启用轻量级链接时优化，减少二进制大小
- `opt-level = "z"`: 优化二进制大小，更快的编译
- `strip = true`: 去除调试符号，减小 40-50% 体积
- `panic = "abort"`: 移除 unwinding，减小体积
- `incremental = true`: 增量编译，后续构建更快

#### Dev Profile 优化
- `opt-level = 1`: 最小优化，加快开发构建
- `incremental = true`: 启用增量编译

#### 依赖项优化
- 移除不需要的特性：
  - `chrono`: 移除不必要的默认特性，只保留 `clock` 和 `serde`
  - `image`: 移除 `jpeg` 和 `webp` 支持（项目只需 BMP/PNG）
  - `sha2`, `uuid`: 禁用默认特性减少编译单元
  - `rusqlite`: 禁用默认特性，只保留 `bundled`

### 2. Cargo 配置 (`.cargo/config.toml`)

- 并行编译: Cargo 默认使用所有 CPU 核心（不需要显式设置 `jobs`）
- 增量链接: Windows MSVC 启用 `/INCREMENTAL`
- 依赖包优化: dev 模式依赖使用 `opt-level = 1`

**注意**: 不要设置 `jobs = 0`，这在某些 Cargo 版本中会导致构建失败。Cargo 会自动使用所有可用核心。

### 3. Vite 前端优化 (`vite.config.ts`)

- `target: 'es2020'`: 现代目标，减少转换
- `minify: 'esbuild'`: 使用更快的 esbuild 压缩
- `reportCompressedSize: false`: 跳过 gzip 报告，节省时间
- **代码分割**:
  - `react`: React/ReactDOM 单独打包
  - `icons`: lucide-react 图标单独打包
  - 更好的缓存利用率

## 构建时间对比

| 阶段 | 优化前 | 优化后 | 提升 |
|------|--------|--------|------|
| 前端构建 (npm run build) | 6.04s | 1.44s | **76%** |
| Rust 首次构建 (估算) | 6-8 分钟 | 2-3 分钟 | **50-60%** |
| Rust 增量构建 (估算) | 3-4 分钟 | 30-60 秒 | **85%** |

## 构建命令

```bash
# 仅前端构建 (生产环境)
npm run build

# 仅前端类型检查
npm run typecheck

# 完整 Tauri 构建 (需要 Rust 工具链)
npm run tauri build

# 开发模式
npm run tauri dev
```

## 后续优化建议

### 如果构建仍然很慢：

1. **使用 sccache 缓存编译结果**
   ```bash
   cargo install sccache
   # 在 .cargo/config.toml 添加
   [build]
   rustc-wrapper = "sccache"
   ```

2. **使用 mold 链接器 (Linux) 或 lld (Windows)**
   - 链接速度提升 5-10 倍
   - Windows: 安装 LLVM，使用 lld-link

3. **CI/CD 优化**
   - 缓存 `target/` 和 `node_modules/`
   - 使用 Rust 缓存 Action
   - 并行运行前端和后端测试

4. **减少依赖**
   - 审查 `Cargo.toml` 中不必要的依赖
   - 使用 `cargo tree` 查看依赖树
   - 考虑更轻量的替代品

## 二进制体积优化

优化后的 release 构建预期体积：
- 优化前: ~15-20 MB
- 优化后: ~8-12 MB (减少 40-50%)

主要通过：
- `strip = true`: 去除符号表
- `opt-level = "z"`: 优化大小
- `lto = "thin"`: 链接时优化
- 移除不需要的图片格式支持

## 注意事项

- 首次构建仍需下载和编译所有依赖 (5-8 分钟)
- 后续增量构建会快得多 (30-90 秒)
- 修改依赖后需要重新编译受影响的 crate
- CI 环境建议启用缓存以加速构建
