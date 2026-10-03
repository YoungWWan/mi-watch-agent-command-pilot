# 小米手表 AI 指令助手 v1.1.1

修复 macOS 安装包缺少完整应用签名的问题。Mac 构建现在对可执行文件和整个 `.app` 使用免费临时签名（ad-hoc），并发布版本为 `1.1.1` 的安装包与签名更新包。

- 提供 Windows x64、Mac Apple Silicon（M 系列）和 Mac Intel 安装包。
- 已安装 `1.1.0` 的用户可在「关于 → 软件更新」检查 `1.1.1`，下载、校验签名并安装；安装完成后自动重启。
- 内置手表指令助手仍为 `v1.1.6`。

| 系统 | 首次安装或手动升级 |
| --- | --- |
| Mac Apple Silicon（M 系列） | `mi-watch-agent-command-pilot_1.1.1_darwin-aarch64.dmg` |
| Mac Intel | `mi-watch-agent-command-pilot_1.1.1_darwin-x86_64.dmg` |
| Windows x64 | `mi-watch-agent-command-pilot_1.1.1_windows-x86_64.exe` |

其余 `.app.tar.gz`、`.sig` 和 `latest.json` 文件用于应用内在线更新。

Mac 临时签名不包含 Apple 公证，首次打开仍可能需要在「系统设置 → 隐私与安全性」允许打开此应用。Windows 发布者代码签名尚未配置；在线更新签名已启用，用于校验更新包。
