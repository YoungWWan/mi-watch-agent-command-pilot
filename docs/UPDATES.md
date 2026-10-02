# 桌面在线更新与发布

用户启动桌面应用时自动检查新版本，应用持续打开时每 6 小时检查一次，也可在「关于 → 软件更新」手动检查。发现新版后显示提醒和更新说明，用户点击「立即更新」才下载、校验签名、安装并重启。网络失败可重试，不会误报“已是最新版本”。待回复指令和手表安装会阻止桌面更新。

发布目标为 Mac Apple Silicon、Mac Intel 和 Windows x64。手表应用独立升级：桌面更新后，连接手表并点击指令助手的升级按钮。

## 首次配置

1. 源码仓库为 [YoungWWan/mi-watch-agent-command-pilot](https://github.com/YoungWWan/mi-watch-agent-command-pilot)，根目录直接包含 `apps/`、`watch/`、`scripts/` 和 `.github/`。在自己的 fork 发布时，使用自己的仓库和签名密钥。
2. 维护者在仓库 Settings → Secrets and variables → Actions 添加 `TAURI_SIGNING_PRIVATE_KEY`，值为与应用公钥匹配的更新签名私钥完整内容；若私钥有密码，还需添加 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`。私钥仅保存在安全的本机位置和 Actions secret 中，不提交到仓库。
3. 安全备份私钥；公钥配置位于 `apps/desktop/src-tauri/tauri.conf.json`。更换或丢失私钥后，已安装客户端无法验证后续更新。后续所有版本使用同一密钥。
4. 推送第一个版本标签后，Actions 会自动使用实际 `GITHUB_REPOSITORY` 写入更新地址，并生成签名更新包和 Release 草稿。检查附件完整后在 GitHub 点击 Publish release，并设为最新稳定版本。

当前源码默认没有设置更新地址，开发构建会显示“此版本尚未开通在线更新”。CI 发布构建自动接入所属仓库，无需手写用户名。已有旧桌面端没有更新功能，用户需要手动安装这一个版本，之后即可在线升级。

## 发布下个版本

从仓库根目录执行（示例版本为 1.1.1）：

```sh
node scripts/desktop-release.mjs version 1.1.1
# 编辑 docs/RELEASE_NOTES.md，描述这个版本的变化
git switch -c codex/release-1.1.1
git add .
git commit -m "Release 1.1.1"
git push -u origin codex/release-1.1.1
# 在 GitHub 创建 PR，检查通过并合入 main 后再打标签
git switch main
git pull --ff-only origin main
git tag v1.1.1
git push origin v1.1.1
```

脚本同时更新 Tauri、Cargo、Cargo.lock 和两个 package.json 的桌面版本。手表 manifest 的版本独立管理。标签必须匹配桌面版本；发布流程会拒绝版本不一致的构建。

三个平台全部构建、测试和签名成功后，工作流才生成完整 `latest.json` 并创建 Release 草稿。草稿发布之前客户端不会收到新版；重新运行工作流可以修复草稿，已发布版本不能覆盖，请提升版本。手动运行工作流时输入已有的版本标签。

Release 附件包括两个 Mac DMG、两个 Mac `.app.tar.gz` 更新包、一个 Windows NSIS `.exe`、各自的 `.sig` 和 `latest.json`。客户端下载匹配自己系统及架构的更新包。签名还绑定桌面版本，避免清单误指向旧包。

## 本地构建

普通开发构建无需更新密钥：

```sh
cd apps/desktop
pnpm install --frozen-lockfile
pnpm tauri build --debug --bundles app
```

如需本地制作公开更新包，先在仓库根目录配置真实仓库：

```sh
node scripts/desktop-release.mjs prepare --repository 你的用户名/mi-watch-agent-command-pilot --tag v1.1.0
cd apps/desktop
export TAURI_SIGNING_PRIVATE_KEY="$HOME/.agent-command-pilot/updates/mi-watch-agent-command-pilot.key"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
pnpm tauri build --bundles app,dmg -- --locked
```

`prepare` 会修改本地 Tauri 配置，开启签名更新包并保存真实更新地址。在自己 fork 中发布时，应配置自己的密钥和公钥，不能使用原维护者的私钥。

更新包签名由 [Tauri Updater](https://v2.tauri.app/plugin/updater/) 验证。它与 Apple Developer 的代码签名、公证以及 Windows 代码签名是不同的机制；当前工作流只配置了更新包签名。Mac/Windows 安装和升级仍需在对应系统测试，Apple 公证可在后续正式分发配置中加入。
