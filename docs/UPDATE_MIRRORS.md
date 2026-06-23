# 更新检查优化 - 中国大陆用户支持

## 问题说明

由于 GitHub 在中国大陆的网络连接不稳定，直接访问 GitHub releases 可能会遇到超时错误：

```
检查更新失败: error sending request for url (https://github.com/...)
```

## 解决方案

### 1. 多镜像源支持

应用已配置多个更新检查端点，会按以下顺序自动尝试：

1. **GitHub 官方源**（海外用户首选）
   - `https://github.com/qianduoduo184/super-clipboard/releases/latest/download/latest.json`

2. **ghproxy.com 镜像**（中国大陆加速）
   - `https://mirror.ghproxy.com/https://github.com/...`

3. **ghproxy.net 镜像**（备用加速源）
   - `https://ghproxy.net/https://github.com/...`

Tauri updater 会自动尝试这些端点，直到其中一个成功响应。

### 2. 友好的错误提示

如果所有端点都失败，应用会显示：

```
检查更新失败: 网络连接超时。如果您在中国大陆，请检查网络连接或稍后重试。
```

### 3. 用户建议

如果更新检查持续失败，可以尝试：

1. **检查网络连接** - 确保设备可以访问互联网
2. **稍后重试** - 镜像服务可能临时不可用
3. **使用代理** - 如果有可用的 HTTP/HTTPS 代理
4. **手动下载** - 访问 GitHub releases 页面手动下载最新版本：
   ```
   https://github.com/qianduoduo184/super-clipboard/releases
   ```
   或使用镜像访问：
   ```
   https://mirror.ghproxy.com/https://github.com/qianduoduo184/super-clipboard/releases
   ```

## 技术实现

### 配置文件

在 `tauri.conf.json` 中配置多个端点：

```json
{
  "plugins": {
    "updater": {
      "endpoints": [
        "https://github.com/.../latest.json",
        "https://mirror.ghproxy.com/https://github.com/.../latest.json",
        "https://ghproxy.net/https://github.com/.../latest.json"
      ]
    }
  }
}
```

### 错误处理

在 `commands.rs` 中增强错误消息：

```rust
.map_err(|error| {
    let error_msg = error.to_string();
    if error_msg.contains("error sending request") || error_msg.contains("timeout") {
        format!("检查更新失败: 网络连接超时。如果您在中国大陆，请检查网络连接或稍后重试。原始错误: {}", error_msg)
    } else {
        format!("检查更新失败: {}", error_msg)
    }
})?
```

## 镜像服务说明

### ghproxy.com / ghproxy.net

- **用途**: GitHub 文件加速服务，为中国大陆用户提供更快的访问速度
- **原理**: 代理 GitHub 请求，缓存静态资源
- **可靠性**: 社区维护的免费服务，可能存在可用性波动
- **隐私**: 请求会经过第三方代理服务器

### 添加自定义镜像

如果需要添加其他镜像源，编辑 `tauri.conf.json` 的 `endpoints` 数组：

```json
"endpoints": [
  "https://github.com/qianduoduo184/super-clipboard/releases/latest/download/latest.json",
  "https://your-custom-mirror.com/path/to/latest.json"
]
```

## 故障排查

### 1. 检查日志

查看应用日志以获取详细错误信息：

- 日志路径：设置 > 诊断信息 > 日志文件路径
- 搜索关键词：`check_update`、`error`

### 2. 验证端点可访问性

在浏览器或命令行中测试端点：

```bash
# Windows PowerShell
Invoke-WebRequest -Uri "https://mirror.ghproxy.com/https://github.com/qianduoduo184/super-clipboard/releases/latest/download/latest.json"

# 或使用 curl
curl "https://mirror.ghproxy.com/https://github.com/qianduoduo184/super-clipboard/releases/latest/download/latest.json"
```

### 3. 临时禁用自动更新

如果更新检查频繁失败影响使用，可以在设置中关闭自动更新：

设置 > 自动更新 > 关闭

## 相关资源

- GitHub Releases: https://github.com/qianduoduo184/super-clipboard/releases
- ghproxy 项目: https://ghproxy.com/
- Tauri Updater 文档: https://tauri.app/v2/plugin/updater/
