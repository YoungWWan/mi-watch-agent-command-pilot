# Kimi Code、ZCode 与 Antigravity 接入

在桌面端 **Agent → MCP 选择题** 点击目标工具的 **接入**，再到 **审批与完成通知** 启用所需钩子。重开 Agent 会话后生效。旧 Python 入口、移动后的应用路径或停用的配置会显示 **需要修复**。MCP 与钩子均使用桌面应用内置二进制，需要保持指令服务运行。

## 支持范围

| Agent | 手表选择题 | 手表审批 | 通知 |
| --- | --- | --- | --- |
| Kimi Code | 支持 | 官方本地 API，允许一次或拒绝 | 同一轮任务结束后的 AI 最终文字回复 |
| ZCode | 支持 | 原生 `PermissionRequest`，允许一次或拒绝；有原生权限建议时提供始终允许 | 本轮完成 |
| Antigravity | 支持 | `PreToolUse` 的终端执行、后台任务控制、文件创建及编辑，允许一次或拒绝 | 正常结束且后台任务全部完成 |

Kimi 的 `PermissionRequest` 是观察事件，钩子返回值仍为 `{}`。桌面助手根据事件中的会话、审批、工具调用、Agent 和轮次 ID 查找待审批请求，收到手表点击后通过官方审批 API 提交 `approved` 或 `rejected`。只允许一次，不修改默认权限模式，也不保存会话级允许规则。60 秒未回复时仍可在电脑审批；电脑已处理、请求已过期或身份不匹配时，旧手表消息不能批准后续请求。

Kimi 的 `Stop` 没有携带最终回复。钩子在返回前读取当前活动轮次，将等待任务交给桌面服务；服务等同一轮的 transcript 状态变为 `completed` 后，读取最后一步的 assistant 文字帧。回复保留完整文字，不截为 300 字，不包含 thinking 内容。失败、取消或其他 Stop 钩子要求继续运行时，不提前发送完成通知。最多等待 15 分钟。

### Kimi 本地认证和版本

启用或修复 Kimi 钩子时，助手使用 `~/.kimi-code/server.token`。已有文件原样保留；不存在时生成 256 位随机 token，macOS / Linux 使用 `600` 权限。Kimi Code 1.0.4 桌面服务已验证支持该认证文件，`kimi web` 也使用同一机制。停用钩子时保留认证文件，因为其他 API 客户端可能仍在使用。不要公开该文件。

助手从 `server/instances/*.json` 发现动态端口，只连接本机回环地址，禁用代理和重定向，并使用认证后的 `/api/v1/meta` 服务 ID 关联轮次（1.0.4 的注册文件 ID 与 API ID 不同）。需要支持审批、snapshot 和 transcript API 的 Kimi Code 版本，并保持 Kimi Code 与桌面指令服务运行。旧版仅有 Hooks 时，请继续在电脑审批；界面“已启用”表示配置已写入，不代表当前 Kimi 服务已连接。

从上一版升级：打开新版助手，在 **Agent → 审批与完成通知 → Kimi Code** 点击 **修复**（未配置时点 **启用**），然后退出并重新打开 Kimi Code，再开始新会话。1.0.4 的钩子运行器在应用启动时加载，单独新建会话不会重新加载修改后的配置。连接失败会保留原生审批，并显示等待电脑确认或回复同步失败的提示。

Antigravity 的钩子在匹配的工具调用之前执行，即使工具已获电脑端授权也可能再次弹出手表审批；选择题、查询文件和其他未匹配工具继续采用 Agent 自身的交互。超时或指令服务不可用时返回 `ask`，由电脑端按原生权限设置处理。任务错误、达到步数限制或仍有后台任务运行时，不发送“已完成”通知。正常结束后从 `transcriptPath` 读取本轮完整文字回复，不包含思考和工具输出；读取失败时发送通用完成提示，并记录诊断错误。通知请求直接访问本机指令服务，发送失败不会阻止 Agent 结束。钩子需要使用支持生命周期 Hooks 的版本，旧版本可继续使用 MCP。

Antigravity 最后一次 `Stop` 的诊断保存在 `~/.agent-command-pilot/antigravity-last-stop.json`（本机私有文件，不保存回复正文、工具参数或错误原文）。`sent` 表示指令服务已接受通知，`skipped` 表示结束条件未满足，`send_failed` 表示通知请求失败。文件时间未更新则说明结束处理器没有运行；可在 Antigravity 的 Settings → Customizations → Hooks 检查启用状态，并重开会话复测。

## 用户配置路径

这些路径在 macOS、Windows 和 Linux 上均相对于当前用户目录。

| Agent | MCP | Hooks |
| --- | --- | --- |
| Kimi Code | `~/.kimi-code/mcp.json`，键 `mcpServers` | `~/.kimi-code/config.toml`，`[[hooks]]` 数组 |
| ZCode | `~/.zcode/cli/config.json`，键 `mcp.servers` | 同一文件中的 `hooks.events`，并启用 `hooks.enabled` |
| Antigravity | `~/.gemini/config/mcp_config.json`，键 `mcpServers` | `~/.gemini/config/hooks.json`，独立命名组 `redmi_watch_hooks` |

Kimi 设置了 `KIMI_CODE_HOME` 时使用该目录；没有覆盖路径、新目录不存在且旧 `~/.kimi` 已存在时，使用旧目录。Kimi 的 MCP 单次调用超时设为 330 秒，以覆盖手表最长 300 秒的答题时间。

ZCode 的 MCP 与钩子修改会保留同一文件中的模型、已有 MCP 和其他钩子。用户级钩子使用 `process` 与参数数组，避免 shell 路径转义；停用本助手只移除本助手的处理器。Antigravity 的其他命名钩子组也会保留。修改前保存 `.redmi-watch.bak` 备份；无法解析的配置会保留原文件并报告错误。

配置检测代表入口文件与参数有效，不代表 Agent 已实际加载它。若工具没有出现，请新建会话并查看对应 Agent 的 MCP 状态。项目级同名 MCP 可能覆盖用户配置；自定义启动参数指定其他配置文件时，请在该 Agent 中检查实际读取路径。

## 让 Agent 优先询问手表

在 Agent 的项目规则中添加下面的说明，或直接在对话里要求使用 `ask_watch_question`：

> 遇到 2–6 项选择题，优先调用 redmi_watch_questions 的 ask_watch_question。status 为 answered 时采用 selected_options；computer_input、timeout 或 error 时回到电脑询问。自由文本和工具权限确认使用对应原生流程。

MCP 本身的首次调用可能需要 Agent 原生确认。接入配置不会修改 Agent 的权限规则。

## 独立 Python 开发入口

项目根目录的 `server/configure_mcp.py` 支持三者的 MCP 配置；它启动独立 Python 服务入口，不安装上述桌面原生钩子：

```bash
python3 server/configure_mcp.py kimi_code
python3 server/configure_mcp.py zcode
python3 server/configure_mcp.py antigravity
python3 server/configure_mcp.py --status
```

需要审批与通知时使用桌面端安装钩子。不要把开发入口配置与桌面内置入口混用；桌面端可以将旧 Python 配置修复为当前应用入口。

## 协议参考与验证

适配依据：[Kimi Code MCP](https://github.com/MoonshotAI/kimi-code/blob/main/docs/en/customization/mcp.md)、[Kimi Code Hooks](https://www.kimi.com/code/docs/kimi-code-cli/customization/hooks.html)、[Kimi Server API](https://www.kimi.com/code/docs/en/kimi-code-cli/reference/server-api.html)、[ZCode MCP](https://www.zcode.network/cn/docs/mcp-services/)、[ZCode Hooks](https://www.zcode.network/cn/docs/hooks/)、[Antigravity MCP](https://www.antigravity.google/docs/mcp/) 与 [Antigravity Hooks](https://www.antigravity.google/docs/hooks/)。旧 Kimi CLI 路径依据 [历史 MCP 文档](https://github.com/MoonshotAI/kimi-cli/blob/main/docs/en/customization/mcp.md)。

自动化验证覆盖原生配置结构、已有配置保留、重复安装、停用、认证文件保留和权限、Kimi 真实 HTTP 协议下的手表回复与审批提交、电脑抢先审批、超时、重复事件、完整回复及失败轮次。另验证 Antigravity 输入输出协议。

2026-10-04 已在 Kimi Code 桌面版 1.0.4（API 服务 2.1.1）完成本地联调：官方 Stop 事件生成一条与最终回复完全一致的通知；原生 Bash 权限请求生成“拒绝 / 允许一次”卡片；取消测试轮次后旧卡片失效。测试会话已归档，测试工作区已移除注册。手表当时处于等待连接状态，设备端实际点击仍需连接后验证。
