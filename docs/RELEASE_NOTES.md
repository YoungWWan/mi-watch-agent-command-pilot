# 小米手表 AI 指令助手 v1.2.1

修复包含 6 个选项的 MCP 选择题发送失败，并改进服务错误提示。

- **选择题发送**：支持 6 个选项和独立的“在电脑输入”按钮，单选、多选均可正常提交；普通指令仍最多支持 6 个操作按钮。
- **错误提示**：服务端返回具体的校验原因；MCP 显示 HTTP 状态和错误信息，避免将空错误响应误报为 `Invalid JSON from server`。
- **本机连接**：MCP 提问直接访问本机指令服务，避免受到系统或环境代理影响。

桌面版本为 `1.2.1`，内置手表指令助手为 `v1.1.6`。已安装并配置更新源的桌面端可在“关于 → 软件更新”升级。更新后请重新打开 Agent 会话，使 MCP 使用新版程序。

| 系统 | 首次安装或手动升级 |
| --- | --- |
| Mac Apple Silicon（M 系列） | `mi-watch-agent-command-pilot_1.2.1_darwin-aarch64.dmg` |
| Mac Intel | `mi-watch-agent-command-pilot_1.2.1_darwin-x86_64.dmg` |
| Windows x64 | `mi-watch-agent-command-pilot_1.2.1_windows-x86_64.exe` |

其余 `.app.tar.gz`、`.sig` 和 `latest.json` 文件用于应用内在线更新。

回归测试覆盖 6 个选项加电脑输入的 HTTP 提交与多选回复、按钮数量边界，以及 JSON、空正文和非 JSON 的 HTTP 错误响应。手表端页面沿用已有选项列表，实际手表点击尚未在本次发布中验证。

Mac 临时签名不包含 Apple 公证，首次打开仍可能需要在「系统设置 → 隐私与安全性」允许打开此应用。Windows 发布者代码签名尚未配置；在线更新签名已启用，用于校验更新包。
