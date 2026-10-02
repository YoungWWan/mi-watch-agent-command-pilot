# Windows 配置与构建

桌面端的 MCP、审批钩子和完成通知使用本机安装的 `agent-command-pilot.exe`。运行桌面安装包无需 Python，也无需克隆项目。配置按当前用户目录生成，支持中文、空格及特殊字符路径；不能把 Mac 的配置文件直接复制到 Windows 后继续使用。

## 安装后接入 Agent

1. 安装 Windows x64 安装包，启动“小米手表 AI 指令助手”，在“指令”页确认服务已启动，完成手表配对和连接测试。
2. 打开“Agent → MCP 选择题”，在目标 Agent 上点击“接入”。已有旧 Python 入口、其他电脑路径或应用移动后的入口会显示“修复”，点击即可更新。
3. 在“审批与完成通知”页启用需要的钩子。配置前会在同目录保存一次 `<文件名>.redmi-watch.bak`，保留其他 MCP、钩子和 Agent 设置；无效 JSON/TOML 会报错并保留原文件。
4. 重启对应 Agent 或重新打开会话，确认能看到 `ask_watch_question`。Agent 自身要求信任钩子时按其正常流程确认。

应用更新后若安装目录不变，原入口可以继续使用；换电脑、移动应用或更换安装目录后，在新位置运行应用并点击“修复”。配置状态验证的是入口和参数，不代表 Agent 已加载配置或手表已经在线。

手表通过局域网访问电脑。Windows 防火墙如提示网络访问权限，请允许应用在正在使用的专用网络通信。电脑和手表需要能互相访问；更换电脑后重新配对，不复制旧电脑的配对令牌。

## Windows 配置位置

`%USERPROFILE%`、`%APPDATA%` 表示当前 Windows 用户的系统目录，并非写入配置的字面字符串。

| Agent | MCP | 钩子 |
|---|---|---|
| Codex | `%USERPROFILE%\.codex\config.toml` | `%USERPROFILE%\.codex\hooks.json` |
| Claude Code | `%USERPROFILE%\.claude.json` | `%USERPROFILE%\.claude\settings.json` |
| Cursor | `%USERPROFILE%\.cursor\mcp.json` | `%USERPROFILE%\.cursor\hooks.json` |
| Claude Desktop | `%APPDATA%\Claude\claude_desktop_config.json` | 不提供 |
| Antigravity | `%USERPROFILE%\.gemini\config\mcp_config.json` | 不提供 |
| Windsurf | `%USERPROFILE%\.codeium\windsurf\mcp_config.json` | 不提供 |

设置了 `CODEX_HOME` 时，Codex 的 MCP、钩子和本项目审批规则一起使用该目录。设置了 `CLAUDE_CONFIG_DIR` 时，Claude Code 的 `settings.json` 与 `.claude.json` 使用该目录。环境变量应对桌面应用和目标 Agent 同时生效；从资源管理器启动的应用不会自动继承另一个终端临时设置的变量。

Claude Desktop 优先使用标准 AppData 配置；仅在标准文件不存在、且检测到 Microsoft Store 安装的现有 `Claude_*\LocalCache\Roaming\Claude` 配置时使用已有文件。桌面页的“配置详情”显示本次选中的实际路径。

MCP 以 JSON/TOML 的独立 `command` 和 `args` 启动。Windows Claude Code 钩子采用直接执行形式；Codex、Cursor 钩子使用系统 PowerShell 的 UTF-16 编码启动命令，从 PowerShell 输入管道接收 JSON、以 UTF-8 写入原生入口，并直接转发输出流。不要手动把这些命令改成 Bash 的单引号形式。

手动用 `powershell.exe -Command` 再启动钩子时，外层 PowerShell 不会自动把自己的 stdin 传给本机命令。需要显式通过 `$input` 管道传入 JSON，并将 `$OutputEncoding`、`[Console]::InputEncoding` 和 `[Console]::OutputEncoding` 设为 UTF-8。参见 [PowerShell 输入流说明](https://learn.microsoft.com/en-us/powershell/module/microsoft.powershell.core/about/about_redirection?view=powershell-7.5)。

本次适配面向 Windows 原生 Agent。WSL 使用自己的 Linux 用户目录、可执行文件和网络环境，需要单独接入；当前 Windows GUI 不会自动改写 WSL 内的配置。Windows 日常使用以桌面页生成的内置入口为准。

## 在 Windows 构建

准备 Node.js 22、pnpm 11.23.0、Rust stable MSVC 工具链、Microsoft C++ Build Tools 的“使用 C++ 的桌面开发”工作负载及 WebView2。详细前置要求见 [Tauri 官方文档](https://v2.tauri.app/start/prerequisites/)。内置 RPK 已在资源目录，无需 Windows 重新构建手表包。

在项目根目录打开 PowerShell：

```powershell
cd apps/desktop
pnpm install --frozen-lockfile
pnpm run build
$testFiles = Get-ChildItem tests -Filter *.test.mjs | ForEach-Object { $_.FullName }
node --test $testFiles
cd src-tauri
cargo test -p agent-command-pilot --lib --locked
cd ..
pnpm tauri build --bundles nsis
node ../../scripts/check-native-integrations.mjs src-tauri/target/release/agent-command-pilot.exe
```

安装包位于 `apps/desktop/src-tauri/target/release/bundle/nsis/`。当前流程输出未签名安装包；用于发布时需另行配置 Windows 签名。

仓库根目录的 `.github/workflows/windows-build.yml` 在 Windows runner 上执行前端测试、Rust 测试、NSIS 打包及发布版 MCP/钩子标准输入输出检查，并上传安装包 artifact。Rust 测试还会复制可执行文件到含中文、空格和特殊字符的目录，分别通过 `cmd.exe` 与 PowerShell 验证 UTF-8 JSON 往返。它不发布 GitHub Release。

`src-tauri/build.rs` 同时为 MSVC 应用和测试可执行文件嵌入 `windows-app-manifest.xml`，启用 Common Controls v6。缺少该清单时，Tauri 的测试程序可能在执行测试前以 `STATUS_ENTRYPOINT_NOT_FOUND` 退出；处理方式参考 [Tauri 官方构建示例](https://github.com/tauri-apps/tauri/blob/dev/examples/api/src-tauri/build.rs)。

## 验证范围

已在 macOS 通过前端构建、前端测试、Rust 测试和原生入口检查，并对配置模块及 Windows 蓝牙实现执行 Windows MSVC 目标的编译检查。完整 Windows 安装包、Windows shell 测试和蓝牙真机连接仍需在 Windows runner / Windows 电脑实际执行；macOS 没有 Windows SDK，不能替代这部分验证。

配置位置与协议参考：[MCP 的 Claude Desktop 示例](https://github.com/modelcontextprotocol/docs/blob/main/quickstart/user.mdx)、[Claude Code 设置](https://code.claude.com/docs/en/settings)、[Claude Code 钩子](https://code.claude.com/docs/en/hooks)、[Codex 钩子](https://developers.openai.com/codex/hooks/)、[Cursor 钩子](https://cursor.com/docs/hooks)。
