# 小米手表 AI 指令助手 v1.2.0

新增 Kimi Code、ZCode 与 Antigravity 的原生审批和完成通知接入，改善账号授权、设备通信与桌面状态显示。

- **Kimi Code**：通过官方本地 API 接收手表的“允许一次 / 拒绝”，并在同一轮任务结束后发送完整的 AI 最终文字回复。电脑已处理、取消或过期的审批不会被旧手表卡片再次批准。
- **ZCode**：支持 MCP 选择题、原生审批与本轮完成提醒；接入、修复和停用时保留其他配置。
- **Antigravity**：支持原生工具审批，正常结束且后台任务完成后发送最终文字回复；异常结束不发送完成提醒。
- **小米账号与设备**：修复授权过期识别，保留临时网络错误的重试入口，改善设备端原生网络请求处理。
- **桌面界面**：明确区分 Agent 接入、审批和通知状态；更新说明支持 Markdown 标题、列表、链接和表格。

升级后，请在 **Agent → 审批与完成通知** 修复已有接入。Kimi Code 需要退出并重新打开应用，再开始新会话；其他 Agent 请重新打开会话。详细配置与支持范围见 [Agent 接入说明](https://github.com/YoungWWan/mi-watch-agent-command-pilot/blob/v1.2.0/docs/AGENT_INTEGRATIONS.md)。

桌面版本为 `1.2.0`，内置手表指令助手为 `v1.1.6`。已安装并配置更新源的桌面端可在“关于 → 软件更新”检查新版本。

| 系统 | 首次安装或手动升级 |
| --- | --- |
| Mac Apple Silicon（M 系列） | `mi-watch-agent-command-pilot_1.2.0_darwin-aarch64.dmg` |
| Mac Intel | `mi-watch-agent-command-pilot_1.2.0_darwin-x86_64.dmg` |
| Windows x64 | `mi-watch-agent-command-pilot_1.2.0_windows-x86_64.exe` |

其余 `.app.tar.gz`、`.sig` 和 `latest.json` 文件用于应用内在线更新。

自动化检查覆盖配置迁移、审批失效、完整回复、账号授权与更新流程。Kimi Code 桌面版 1.0.4 已完成本地 API 联调，手表端实际点击仍需真机验证。

Mac 临时签名不包含 Apple 公证，首次打开仍可能需要在「系统设置 → 隐私与安全性」允许打开此应用。Windows 发布者代码签名尚未配置；在线更新签名已启用，用于校验更新包。
