use super::{
    configurator::{backup_config, get_binary_path},
    paths::{ConfigPaths, Platform},
    permission_rules::write_private,
};
use anyhow::{Context, Result};
use base64::Engine;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Serialize)]
pub struct HookStatus {
    key: String,
    name: String,
    configured: bool,
    approval_installed: bool,
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
    ]
}

fn read_config(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let text = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(text.trim_start_matches('\u{feff}'))?)
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
    // the encoded script. Forward raw streams to avoid PowerShell 5.1 re-encoding JSON.
    let script = format!(
        "# redmi-watch-command-v1\n$ErrorActionPreference = 'Stop'; \
         $start = New-Object System.Diagnostics.ProcessStartInfo; \
         $start.FileName = {}; $start.Arguments = {}; \
         $start.UseShellExecute = $false; $start.CreateNoWindow = $true; \
         $start.RedirectStandardInput = $true; $start.RedirectStandardOutput = $true; $start.RedirectStandardError = $true; \
         $process = New-Object System.Diagnostics.Process; $process.StartInfo = $start; [void]$process.Start(); \
         $output = $process.StandardOutput.BaseStream.CopyToAsync([Console]::OpenStandardOutput()); \
         $errors = $process.StandardError.BaseStream.CopyToAsync([Console]::OpenStandardError()); \
         [Console]::OpenStandardInput().CopyTo($process.StandardInput.BaseStream); $process.StandardInput.Close(); \
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
        ["type", "command", "args", "commandWindows", "shell"]
            .iter()
            .all(|key| entry[*key] == expected[*key])
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
    let flat = key == "cursor";
    let approval_event = if flat {
        "beforeShellExecution"
    } else {
        "PermissionRequest"
    };
    let completion_event = if flat { "stop" } else { "Stop" };
    let binary_text = binary.to_string_lossy();
    let approval = binary.is_file()
        && installed(
            config,
            approval_event,
            flat,
            &handler(key, name, "permission", 70, &binary_text, platform),
        );
    let completion = binary.is_file()
        && installed(
            config,
            completion_event,
            flat,
            &handler(key, name, "stop", 5, &binary_text, platform),
        );
    let configured = approval && completion;
    let has_owned = [approval_event, completion_event].iter().any(|event| {
        event_handlers(config, event, flat)
            .iter()
            .any(|entry| owned(entry))
    });
    let needs_repair = has_owned && !configured;
    HookStatus {
        key: key.into(),
        name: name.into(),
        configured,
        approval_installed: approval,
        completion_installed: completion,
        needs_repair,
        message: if needs_repair {
            "旧版钩子或应用路径已变化，点击修复后使用当前应用。"
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
    for (event, mode, timeout) in if flat {
        [
            ("beforeShellExecution", "permission", 70),
            ("stop", "stop", 5),
        ]
    } else {
        [("PermissionRequest", "permission", 70), ("Stop", "stop", 5)]
    } {
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
                json!({"hooks":[handler]})
            });
        }
        if entries.is_empty() {
            hooks.remove(event);
        }
    }
    Ok(())
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
        write_private(&path, &config)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(script.contains("OpenStandardInput"));
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
        let value: Value = serde_json::from_str(&input).unwrap();
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
                process.args(["-NoProfile", "-NonInteractive", "-Command", &command]);
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
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(String::from_utf8(output.stdout)
                .unwrap()
                .contains(&format!("STDIO_FIXTURE:{payload}")));
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
