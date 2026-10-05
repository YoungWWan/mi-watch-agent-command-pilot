use super::{
    configurator::{backup_config, get_binary_path},
    paths::{ConfigPaths, Platform},
    permission_rules::write_text_private,
};
use anyhow::{Context, Result};
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table};

const ANTIGRAVITY_HOOK: &str = "redmi_watch_hooks";
const ANTIGRAVITY_MATCHER: &str =
    "^(run_command|manage_task|write_to_file|replace_file_content|multi_replace_file_content)$";

#[derive(Serialize)]
pub struct HookStatus {
    key: String,
    name: String,
    configured: bool,
    approval_installed: bool,
    approval_supported: bool,
    attention_installed: bool,
    completion_installed: bool,
    needs_repair: bool,
    message: String,
    config_path: String,
}

fn configs(paths: &ConfigPaths) -> Vec<(&'static str, &'static str, PathBuf)> {
    vec![
        ("codex", "Codex", paths.codex.join("hooks.json")),
        (
            "claude_code",
            "Claude Code",
            paths.claude_code.join("settings.json"),
        ),
        ("cursor", "Cursor", paths.home.join(".cursor/hooks.json")),
        ("kimi_code", "Kimi Code", paths.kimi.join("config.toml")),
        ("zcode", "ZCode", paths.home.join(".zcode/cli/config.json")),
        (
            "antigravity",
            "Antigravity",
            paths.home.join(".gemini/config/hooks.json"),
        ),
    ]
}

fn read_config(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = std::fs::read_to_string(path)?;
    if path
        .extension()
        .is_some_and(|extension| extension == "toml")
    {
        Ok(toml_edit::de::from_str(
            text.trim_start_matches('\u{feff}'),
        )?)
    } else {
        Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
    }
}

fn normalized(config: &Value, key: &str) -> Value {
    match key {
        "zcode" => json!({"hooks": config["hooks"]["events"]}),
        "antigravity" => json!({"hooks": config[ANTIGRAVITY_HOOK]}),
        "kimi_code" => {
            let mut events = serde_json::Map::new();
            for hook in config["hooks"].as_array().into_iter().flatten() {
                if let Some(event) = hook["event"].as_str() {
                    events
                        .entry(event)
                        .or_insert(json!([]))
                        .as_array_mut()
                        .unwrap()
                        .push(hook.clone());
                }
            }
            json!({"hooks": events})
        }
        _ => config.clone(),
    }
}

fn events(key: &str) -> [(&'static str, &'static str, u64, bool); 2] {
    match key {
        "cursor" => [
            ("beforeShellExecution", "permission", 70, true),
            ("stop", "stop", 5, true),
        ],
        "kimi_code" => [
            ("PermissionRequest", "permission", 5, true),
            ("Stop", "stop", 15, true),
        ],
        "antigravity" => [
            ("PreToolUse", "permission", 70, false),
            ("Stop", "stop", 5, true),
        ],
        _ => [
            ("PermissionRequest", "permission", 70, false),
            ("Stop", "stop", 5, false),
        ],
    }
}

fn decoded_windows_command(command: &str) -> Option<String> {
    let encoded = command
        .strip_prefix("powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand ")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    if bytes.len() % 2 != 0 {
        return None;
    }
    let words = bytes
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    String::from_utf16(&words).ok()
}

fn owned(handler: &Value) -> bool {
    ["command", "commandWindows", "command_windows"]
        .iter()
        .any(|key| {
            let Some(command) = handler[*key].as_str() else {
                return false;
            };
            if decoded_windows_command(command)
                .is_some_and(|script| script.starts_with("# redmi-watch-command-v1\n"))
            {
                return true;
            }
            let native =
                command.contains("agent-command-pilot") || command.contains("Agent 指令助手");
            (native
                && (command.contains("--hook ")
                    || handler["args"]
                        .as_array()
                        .is_some_and(|args| args.first().is_some_and(|arg| arg == "--hook"))))
                || command.contains("permission_request_hook.py")
                || command.contains("completion_notification_hook.py")
        })
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn powershell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn windows_command(binary: &str, arguments: &str) -> String {
    // cmd.exe and PowerShell tokenize quoted executable paths differently. The outer
    // command contains only ASCII tokens; paths and arguments are UTF-16 literals in
    // the encoded script. PowerShell 5.1 feeds redirected stdin into $input; reading
    // the OS handle again loses buffered input. Encode the JSON explicitly for the
    // native child, and forward its output streams without PowerShell formatting.
    let script = format!(
        "# redmi-watch-command-v1\n$ErrorActionPreference = 'Stop'; \
         [Console]::InputEncoding = New-Object System.Text.UTF8Encoding; \
         $json = [string]::Join([Environment]::NewLine, @($input)); \
         $bytes = [System.Text.Encoding]::UTF8.GetBytes($json); \
         $start = New-Object System.Diagnostics.ProcessStartInfo; \
         $start.FileName = {}; $start.Arguments = {}; \
         $start.UseShellExecute = $false; $start.CreateNoWindow = $true; \
         $start.RedirectStandardInput = $true; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true; \
         $process = New-Object System.Diagnostics.Process; $process.StartInfo = $start; [void]$process.Start(); \
         $output = $process.StandardOutput.BaseStream.CopyToAsync([Console]::OpenStandardOutput()); \
         $errors = $process.StandardError.BaseStream.CopyToAsync([Console]::OpenStandardError()); \
         $process.StandardInput.BaseStream.Write($bytes, 0, $bytes.Length); $process.StandardInput.Close(); \
         $process.WaitForExit(); [void]$output.GetAwaiter().GetResult(); [void]$errors.GetAwaiter().GetResult(); exit $process.ExitCode",
        powershell_quote(binary), powershell_quote(arguments)
    );
    let bytes = script
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect::<Vec<_>>();
    format!(
        "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

fn handler(
    key: &str,
    name: &str,
    mode: &str,
    timeout: u64,
    binary: &str,
    platform: Platform,
) -> Value {
    if key == "zcode" {
        return json!({"type":"process", "command":binary, "args":["--hook",mode,"--app",name], "timeoutMs":timeout * 1000, "enabled":true});
    }
    if key == "claude_code" && platform == Platform::Windows {
        // Claude Code's exec form bypasses both Git Bash and PowerShell quoting.
        return json!({"type":"command", "command":binary, "args":["--hook",mode,"--app",name], "timeout":timeout});
    }
    let command = if platform == Platform::Windows {
        windows_command(binary, &format!("--hook {mode} --app \"{name}\""))
    } else {
        format!(
            "{} --hook {} --app {}",
            shell_quote(binary),
            mode,
            shell_quote(name)
        )
    };
    let mut result = json!({"type":"command", "command":command, "timeout":timeout});
    if key == "kimi_code" {
        result.as_object_mut().unwrap().remove("type");
    }
    if key == "codex" && platform == Platform::Windows {
        result["commandWindows"] = result["command"].clone();
    }
    result
}

fn event_handlers<'a>(config: &'a Value, event: &str, flat: bool) -> Vec<&'a Value> {
    let Some(entries) = config["hooks"][event].as_array() else {
        return vec![];
    };
    if flat {
        return entries.iter().collect();
    }
    entries
        .iter()
        .flat_map(|entry| entry["hooks"].as_array().into_iter().flatten())
        .collect()
}

fn installed(config: &Value, event: &str, flat: bool, expected: &Value) -> bool {
    event_handlers(config, event, flat).iter().any(|entry| {
        ["type", "command", "args", "commandWindows", "shell", "timeout", "timeoutMs"]
            .iter()
            .all(|key| entry[*key] == expected[*key])
            && entry["enabled"] != false
            // A filtered handler does not cover the event as configured by us.
            && (flat || config["hooks"][event].as_array().is_some_and(|entries| entries.iter().any(|group| {
                let matcher = group["matcher"].as_str().unwrap_or("");
                (matcher.is_empty() || matcher == "*" || (event == "PreToolUse" && matcher == ANTIGRAVITY_MATCHER))
                    && group["hooks"].as_array().is_some_and(|handlers| handlers.iter().any(|handler| handler == *entry))
            })))
            && (!flat || entry["matcher"].as_str().is_none_or(|matcher| matcher.is_empty() || matcher == ".*" || matcher == "*"))
    })
}

fn status(
    key: &str,
    name: &str,
    path: &Path,
    config: &Value,
    binary: &Path,
    platform: Platform,
) -> HookStatus {
    let [(approval_event, approval_mode, approval_timeout, approval_flat), (completion_event, _, completion_timeout, completion_flat)] =
        events(key);
    let enabled = match key {
        "zcode" => config["hooks"]["enabled"] == true,
        "antigravity" => config[ANTIGRAVITY_HOOK]["enabled"] != false,
        _ => true,
    };
    let config = normalized(config, key);
    let binary_text = binary.to_string_lossy();
    let approval = enabled
        && binary.is_file()
        && installed(
            &config,
            approval_event,
            approval_flat,
            &handler(
                key,
                name,
                approval_mode,
                approval_timeout,
                &binary_text,
                platform,
            ),
        );
    let completion = enabled
        && binary.is_file()
        && installed(
            &config,
            completion_event,
            completion_flat,
            &handler(
                key,
                name,
                "stop",
                completion_timeout,
                &binary_text,
                platform,
            ),
        );
    let kimi_auth = key != "kimi_code"
        || path
            .parent()
            .is_some_and(|home| super::kimi::read_token(home).is_ok());
    let configured = approval && completion && kimi_auth;
    let has_owned = events(key).iter().any(|(event, _, _, flat)| {
        event_handlers(&config, event, *flat)
            .iter()
            .any(|entry| owned(entry))
    });
    let needs_repair = has_owned && !configured;
    HookStatus {
        key: key.into(),
        name: name.into(),
        configured,
        approval_installed: approval && kimi_auth,
        approval_supported: true,
        attention_installed: false,
        completion_installed: completion && kimi_auth,
        needs_repair,
        message: if key == "kimi_code" && has_owned && !kimi_auth {
            "Kimi 本地认证尚未配置或文件权限无效，请点击修复；已有认证文件会保留。"
        } else if needs_repair {
            "钩子路径、启用状态或参数需要更新，请点击修复。"
        } else if key == "kimi_code" {
            "支持允许一次、拒绝及完成回复；启用后请重启 Kimi Code，并保持指令服务运行。超时仍在电脑审批。"
        } else if key == "antigravity" {
            "通过 PreToolUse 审批终端执行、后台任务控制和文件写入；需要支持生命周期钩子的 Antigravity 版本。"
        } else if configured {
            "已配置应用内置钩子。"
        } else {
            "尚未启用钩子。"
        }
        .into(),
        config_path: path.to_string_lossy().into(),
    }
}

pub fn statuses() -> Result<Vec<HookStatus>> {
    let paths = ConfigPaths::current()?;
    let binary = get_binary_path();
    Ok(configs(&paths)
        .into_iter()
        .map(|(key, name, path)| {
            match read_config(&path).with_context(|| format!("无法读取 {name} 钩子配置")) {
                Ok(config) => status(key, name, &path, &config, &binary, paths.platform),
                Err(error) => HookStatus {
                    key: key.into(),
                    name: name.into(),
                    configured: false,
                    approval_installed: false,
                    approval_supported: true,
                    attention_installed: false,
                    completion_installed: false,
                    needs_repair: false,
                    message: format!("{error}；原文件已保留。"),
                    config_path: path.to_string_lossy().into(),
                },
            }
        })
        .collect())
}

fn update(
    config: &mut Value,
    key: &str,
    name: &str,
    enable: bool,
    binary: &str,
    platform: Platform,
) -> Result<()> {
    if key == "zcode" || key == "antigravity" {
        config.as_object().context("钩子配置必须是 JSON 对象")?;
        let mut inner = normalized(config, key);
        if inner["hooks"].is_null() {
            inner = json!({"hooks":{}});
        }
        update_events(&mut inner, key, name, enable, binary, platform)?;
        if key == "zcode" {
            let root = config.as_object_mut().unwrap();
            let hooks = root
                .entry("hooks")
                .or_insert(json!({}))
                .as_object_mut()
                .context("hooks 必须是对象")?;
            hooks.insert("events".into(), inner["hooks"].clone());
            if enable {
                hooks.insert("enabled".into(), json!(true));
            }
        } else {
            let mut definition = config_definition(config, enable)?;
            let root = config.as_object_mut().unwrap();
            definition.insert("PreToolUse".into(), inner["hooks"]["PreToolUse"].clone());
            definition.insert("Stop".into(), inner["hooks"]["Stop"].clone());
            definition.retain(|_, value| !value.is_null());
            if enable || definition.keys().any(|key| key != "enabled") {
                root.insert(ANTIGRAVITY_HOOK.into(), Value::Object(definition));
            } else {
                root.remove(ANTIGRAVITY_HOOK);
            }
        }
        return Ok(());
    }
    update_events(config, key, name, enable, binary, platform)
}

fn config_definition(config: &Value, enable: bool) -> Result<serde_json::Map<String, Value>> {
    let mut definition = match config.get(ANTIGRAVITY_HOOK) {
        Some(value) => value
            .as_object()
            .context("Antigravity 钩子必须是对象")?
            .clone(),
        None => serde_json::Map::new(),
    };
    if enable {
        definition.insert("enabled".into(), json!(true));
    }
    Ok(definition)
}

fn update_events(
    config: &mut Value,
    key: &str,
    name: &str,
    enable: bool,
    binary: &str,
    platform: Platform,
) -> Result<()> {
    let flat = key == "cursor";
    let root = config.as_object_mut().context("钩子配置必须是 JSON 对象")?;
    if flat {
        root.entry("version").or_insert(json!(1));
    }
    let hooks = root
        .entry("hooks")
        .or_insert(json!({}))
        .as_object_mut()
        .context("hooks 必须是对象")?;
    for (event, mode, timeout, flat) in events(key) {
        let entries = hooks
            .entry(event)
            .or_insert(json!([]))
            .as_array_mut()
            .context("钩子事件必须是数组")?;
        if flat {
            entries.retain(|entry| !owned(entry));
        } else {
            for entry in entries.iter_mut() {
                if let Some(handlers) = entry["hooks"].as_array_mut() {
                    handlers.retain(|entry| !owned(entry));
                }
            }
            entries.retain(|entry| {
                !entry["hooks"]
                    .as_array()
                    .is_some_and(|handlers| handlers.is_empty())
            });
        }
        if enable {
            let handler = handler(key, name, mode, timeout, binary, platform);
            entries.push(if flat {
                handler
            } else {
                if key == "antigravity" {
                    json!({"matcher":ANTIGRAVITY_MATCHER, "hooks":[handler]})
                } else {
                    json!({"hooks":[handler]})
                }
            });
        }
        if entries.is_empty() {
            hooks.remove(event);
        }
    }
    Ok(())
}

fn update_kimi(content: &str, enable: bool, binary: &str, platform: Platform) -> Result<String> {
    let mut document = content
        .trim_start_matches('\u{feff}')
        .parse::<DocumentMut>()
        .context("Kimi 配置不是有效 TOML，已保留原文件")?;
    if document.get("hooks").is_none() {
        if !enable {
            return Ok(content.into());
        }
        document["hooks"] = Item::ArrayOfTables(ArrayOfTables::new());
    }
    if let Some(array) = document["hooks"].as_array() {
        let mut tables = ArrayOfTables::new();
        for entry in array.iter() {
            tables.push(
                entry
                    .as_inline_table()
                    .context("Kimi hooks 条目必须是对象")?
                    .clone()
                    .into_table(),
            );
        }
        document["hooks"] = Item::ArrayOfTables(tables);
    }
    let hooks = document["hooks"]
        .as_array_of_tables_mut()
        .context("Kimi hooks 必须是 [[hooks]] 数组")?;
    hooks.retain(|table| {
        let hook: Value = toml_edit::de::from_str(&table.to_string()).unwrap_or(json!({}));
        !owned(&hook)
    });
    if enable {
        for (event, mode, timeout, _) in events("kimi_code") {
            let handler = handler("kimi_code", "Kimi Code", mode, timeout, binary, platform);
            let mut table = Table::new();
            table["event"] = value(event);
            table["command"] = value(handler["command"].as_str().unwrap());
            table["timeout"] = value(timeout as i64);
            hooks.push(table);
        }
    }
    if hooks.is_empty() {
        document.remove("hooks");
    }
    Ok(document.to_string())
}

pub fn toggle(key: &str, enable: bool) -> Result<()> {
    let paths = ConfigPaths::current()?;
    let (_, name, path) = configs(&paths)
        .into_iter()
        .find(|(candidate, _, _)| *candidate == key)
        .context("不支持此 Agent 的钩子")?;
    if !enable && !path.exists() {
        return Ok(());
    }
    if key == "kimi_code" {
        let content = if path.exists() {
            std::fs::read_to_string(&path)?
        } else {
            String::new()
        };
        let updated = update_kimi(
            &content,
            enable,
            &get_binary_path().to_string_lossy(),
            paths.platform,
        )?;
        if enable {
            super::kimi::ensure_token(&paths.kimi)?;
        }
        if updated != content {
            backup_config(&path)?;
            write_text_private(&path, &updated)?;
        }
        return Ok(());
    }
    let mut config = read_config(&path)?;
    let previous = config.clone();
    update(
        &mut config,
        key,
        name,
        enable,
        &get_binary_path().to_string_lossy(),
        paths.platform,
    )?;
    if config != previous {
        backup_config(&path)?;
        write_text_private(&path, &(serde_json::to_string_pretty(&config)? + "\n"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zcode_and_antigravity_use_native_schemas_and_preserve_foreign_hooks() {
        let root =
            std::env::temp_dir().join(format!("pilot-hook-new-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let binary = root.join("agent-command-pilot.exe");
        std::fs::write(&binary, "fixture").unwrap();
        for platform in [Platform::MacOs, Platform::Windows] {
            for (key, name, original) in [
                (
                    "zcode",
                    "ZCode",
                    json!({"mcp":{"servers":{"other":{"command":"other"}}},"hooks":{"enabled":false,"events":{"Stop":[{"hooks":[{"type":"process","command":"other"}]}]}}}),
                ),
                (
                    "antigravity",
                    "Antigravity",
                    json!({"other-hook":{"Stop":[{"command":"other"}]}}),
                ),
            ] {
                let mut config = original.clone();
                update(
                    &mut config,
                    key,
                    name,
                    true,
                    binary.to_str().unwrap(),
                    platform,
                )
                .unwrap();
                let first = config.clone();
                assert!(status(key, name, &binary, &config, &binary, platform).configured);
                update(
                    &mut config,
                    key,
                    name,
                    true,
                    binary.to_str().unwrap(),
                    platform,
                )
                .unwrap();
                assert_eq!(first, config);
                if key == "zcode" {
                    let handler = &config["hooks"]["events"]["PermissionRequest"][0]["hooks"][0];
                    assert_eq!(handler["type"], "process");
                    assert_eq!(handler["timeoutMs"], 70_000);
                    assert_eq!(
                        handler["args"],
                        json!(["--hook", "permission", "--app", "ZCode"])
                    );
                    assert_eq!(config["mcp"], original["mcp"]);
                    config["hooks"]["enabled"] = json!(false);
                } else {
                    assert!(config[ANTIGRAVITY_HOOK]["Stop"][0].get("command").is_some());
                    assert_eq!(
                        config[ANTIGRAVITY_HOOK]["PreToolUse"][0]["matcher"],
                        ANTIGRAVITY_MATCHER
                    );
                    assert_eq!(config["other-hook"], original["other-hook"]);
                    config[ANTIGRAVITY_HOOK]["enabled"] = json!(false);
                }
                assert!(status(key, name, &binary, &config, &binary, platform).needs_repair);
                update(
                    &mut config,
                    key,
                    name,
                    false,
                    binary.to_str().unwrap(),
                    platform,
                )
                .unwrap();
                assert!(!status(key, name, &binary, &config, &binary, platform).configured);
                assert!(config.to_string().contains("other"));
                if key == "zcode" {
                    assert_eq!(config["mcp"], original["mcp"]);
                } else {
                    assert_eq!(config, original);
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn kimi_toml_preserves_models_comments_and_foreign_hooks_and_requires_private_auth() {
        let root =
            std::env::temp_dir().join(format!("pilot-hook-kimi-{:016x}", rand::random::<u64>()));
        std::fs::create_dir_all(&root).unwrap();
        let binary = root.join("agent-command-pilot");
        std::fs::write(&binary, "fixture").unwrap();
        super::super::kimi::ensure_token(&root).unwrap();
        for platform in [Platform::MacOs, Platform::Windows] {
            let original = "# preserve config\ndefault_model = 'kimi'\n[[hooks]]\nevent = 'Stop'\ncommand = 'other-hook'\n[models.kimi]\nmodel = 'kimi-k2'\n";
            let updated = update_kimi(original, true, binary.to_str().unwrap(), platform).unwrap();
            assert!(updated.contains("# preserve config"));
            assert_eq!(
                update_kimi(&updated, true, binary.to_str().unwrap(), platform).unwrap(),
                updated
            );
            let parsed: Value = toml_edit::de::from_str(&updated).unwrap();
            let status = status(
                "kimi_code",
                "Kimi Code",
                &binary,
                &parsed,
                &binary,
                platform,
            );
            assert!(status.configured && status.approval_installed && status.completion_installed);
            assert!(status.approval_supported && !status.attention_installed);
            for hook in parsed["hooks"].as_array().unwrap().iter().skip(1) {
                assert!(hook
                    .as_object()
                    .unwrap()
                    .keys()
                    .all(|key| ["event", "command", "timeout"].contains(&key.as_str())));
            }
            let removed: Value = toml_edit::de::from_str(
                &update_kimi(&updated, false, binary.to_str().unwrap(), platform).unwrap(),
            )
            .unwrap();
            assert_eq!(removed, toml_edit::de::from_str::<Value>(original).unwrap());
            assert!(update_kimi("hooks = []\n", true, binary.to_str().unwrap(), platform).is_ok());
            assert!(update_kimi(
                "hooks = 'invalid'\n",
                true,
                binary.to_str().unwrap(),
                platform
            )
            .is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn migration_preserves_other_handlers_and_is_idempotent_on_both_platforms() {
        for platform in [Platform::MacOs, Platform::Windows] {
            for (key, name) in [
                ("codex", "Codex"),
                ("claude_code", "Claude Code"),
                ("cursor", "Cursor"),
            ] {
                let flat = key == "cursor";
                let event = if flat {
                    "beforeShellExecution"
                } else {
                    "PermissionRequest"
                };
                let foreign = json!({"command":"other-hook"});
                let legacy = json!({"command":"python3 /old/permission_request_hook.py"});
                let entries = if flat {
                    json!([foreign, legacy])
                } else {
                    json!([{"matcher":"*", "hooks":[foreign,legacy]}])
                };
                let mut config = json!({"other":true,"hooks":{event:entries}});
                let binary = r"C:\Program Files\中文's & $ ` %\agent-command-pilot.exe";
                update(&mut config, key, name, true, binary, platform).unwrap();
                let first = config.clone();
                update(&mut config, key, name, true, binary, platform).unwrap();
                assert_eq!(first, config);
                assert!(installed(
                    &config,
                    event,
                    flat,
                    &handler(key, name, "permission", 70, binary, platform)
                ));
                update(&mut config, key, name, false, binary, platform).unwrap();
                assert!(!event_handlers(&config, event, flat)
                    .iter()
                    .any(|entry| owned(entry)));
                assert_eq!(config["other"], true);
                assert!(config.to_string().contains("other-hook"));
            }
        }
    }

    #[test]
    fn windows_claude_exec_and_encoded_shell_commands_preserve_paths_and_arguments() {
        let binary = r"D:\应用\指令 助手's & $ ` %\agent-command-pilot.exe";
        let direct = handler(
            "claude_code",
            "Claude Code",
            "permission",
            70,
            binary,
            Platform::Windows,
        );
        assert_eq!(direct["command"], binary);
        assert_eq!(
            direct["args"],
            json!(["--hook", "permission", "--app", "Claude Code"])
        );
        let encoded = handler(
            "codex",
            "Codex",
            "permission",
            70,
            binary,
            Platform::Windows,
        );
        assert_eq!(encoded["command"], encoded["commandWindows"]);
        let command = encoded["command"].as_str().unwrap();
        assert!(!command.contains(binary));
        let script = decoded_windows_command(command).unwrap();
        assert!(script.contains(&powershell_quote(binary)));
        assert!(script.contains("--hook permission --app \"Codex\""));
        assert!(script.contains("@($input)"));
        assert!(script.contains("UTF8.GetBytes($json)"));
        assert!(script.contains("OpenStandardOutput"));
        assert!(!script.contains("ExecutionPolicy"));
        assert!(owned(&encoded));
        assert!(owned(&direct));
    }

    #[test]
    fn old_missing_or_partial_hooks_are_repairable_instead_of_enabled() {
        let binary = std::env::current_exe().unwrap();
        let path = Path::new("unused-hooks.json");
        let mut config = json!({"hooks":{"PermissionRequest":[{"hooks":[{"command":"python3 /old/permission_request_hook.py"}]}]}});
        let old = status("codex", "Codex", path, &config, &binary, Platform::Windows);
        assert!(old.needs_repair);
        assert!(!old.configured);
        update(
            &mut config,
            "codex",
            "Codex",
            true,
            binary.to_str().unwrap(),
            Platform::Windows,
        )
        .unwrap();
        assert!(status("codex", "Codex", path, &config, &binary, Platform::Windows).configured);
        config["hooks"].as_object_mut().unwrap().remove("Stop");
        let partial = status("codex", "Codex", path, &config, &binary, Platform::Windows);
        assert!(partial.needs_repair);
        assert!(partial.approval_installed);
        assert!(!partial.completion_installed);
        assert!(
            !status(
                "codex",
                "Codex",
                path,
                &config,
                Path::new("missing.exe"),
                Platform::Windows
            )
            .configured
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_stdio_fixture() {
        if std::env::var_os("REDMI_WATCH_STDIO_FIXTURE").is_none() {
            return;
        }
        use std::io::Read;
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input).unwrap();
        let value: Value = serde_json::from_str(&input)
            .unwrap_or_else(|error| panic!("{error}; fixture stdin={input:?}"));
        println!("STDIO_FIXTURE:{}", value);
    }

    #[cfg(windows)]
    #[test]
    fn windows_wrapper_round_trips_utf8_json_through_cmd_and_powershell() {
        use std::{
            io::Write,
            process::{Command, Stdio},
        };
        let root = std::env::temp_dir().join(format!(
            "pilot 中文 ' & $ % `-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let binary = root.join("agent-command-pilot.exe");
        std::fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
        let command = windows_command(
            binary.to_str().unwrap(),
            "--exact integrations::hooks::tests::windows_stdio_fixture --nocapture",
        );
        let payload = json!({"message":"中文回复 💡", "path":r"C:\中文\test"}).to_string();
        for shell in ["cmd.exe", "powershell.exe"] {
            let mut process = Command::new(shell);
            if shell == "cmd.exe" {
                process.args(["/D", "/S", "/C", &command]);
            } else {
                // PowerShell does not connect its own redirected stdin to a native
                // command's pipeline. Forward the JSON explicitly, with UTF-8 at
                // both ends, instead of launching the wrapper with an empty pipe.
                let script = format!(
                    "$OutputEncoding = [Console]::InputEncoding = [Console]::OutputEncoding = \
                     New-Object System.Text.UTF8Encoding; \
                     $input | {command}; exit $LASTEXITCODE"
                );
                process.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
            }
            let mut child = process
                .env("REDMI_WATCH_STDIO_FIXTURE", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(payload.as_bytes())
                .unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(
                output.status.success(),
                "{shell}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8(output.stdout)
                .unwrap()
                .contains(&format!("STDIO_FIXTURE:{payload}")));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
