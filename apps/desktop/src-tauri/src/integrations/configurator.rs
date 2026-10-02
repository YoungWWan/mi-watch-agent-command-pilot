use super::{
    paths::{ConfigPaths, Platform},
    permission_rules::write_text_private,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{value, Array, DocumentMut, Item, Table};

const SERVER_NAME: &str = "redmi_watch_questions";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct AgentStatus {
    pub key: String,
    pub name: String,
    pub installed: bool,
    pub configured: bool,
    pub needs_repair: bool,
    pub message: String,
    pub config_path: String,
}

#[derive(Clone, Copy)]
enum Format {
    Toml,
    Json,
}

struct AgentConfig {
    key: &'static str,
    name: &'static str,
    path: PathBuf,
    parent: PathBuf,
    format: Format,
}

pub fn get_binary_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| {
        PathBuf::from(if cfg!(windows) {
            "agent-command-pilot.exe"
        } else {
            "agent-command-pilot"
        })
    })
}

fn configs(paths: &ConfigPaths) -> Vec<AgentConfig> {
    let home = &paths.home;
    let mut result = vec![
        AgentConfig {
            key: "codex",
            name: "Codex",
            path: paths.codex.join("config.toml"),
            parent: paths.codex.clone(),
            format: Format::Toml,
        },
        AgentConfig {
            key: "claude_code",
            name: "Claude Code",
            path: paths.claude_mcp.clone(),
            parent: paths.claude_code.clone(),
            format: Format::Json,
        },
        AgentConfig {
            key: "cursor",
            name: "Cursor",
            path: home.join(".cursor/mcp.json"),
            parent: home.join(".cursor"),
            format: Format::Json,
        },
        AgentConfig {
            key: "antigravity",
            name: "Antigravity",
            path: home.join(".gemini/config/mcp_config.json"),
            parent: home.join(".gemini"),
            format: Format::Json,
        },
        AgentConfig {
            key: "windsurf",
            name: "Windsurf",
            path: home.join(".codeium/windsurf/mcp_config.json"),
            parent: home.join(".codeium/windsurf"),
            format: Format::Json,
        },
    ];
    if let Some(path) = &paths.claude_desktop {
        result.push(AgentConfig {
            key: "claude_desktop",
            name: "Claude Desktop",
            path: path.clone(),
            parent: path.parent().unwrap().into(),
            format: Format::Json,
        });
    }
    result
}

pub(crate) fn executable_matches(command: &str, binary: &Path, platform: Platform) -> bool {
    if !binary.is_file() {
        return false;
    }
    let normalize = |path: &Path| {
        let text = path.to_string_lossy().to_string();
        if platform == Platform::Windows {
            text.replace('\\', "/")
                .trim_start_matches("//?/")
                .to_lowercase()
        } else {
            text
        }
    };
    let candidate = Path::new(command);
    if normalize(candidate) == normalize(binary) {
        return true;
    }
    match (fs::canonicalize(candidate), fs::canonicalize(binary)) {
        (Ok(candidate), Ok(binary)) => normalize(&candidate) == normalize(&binary),
        _ => false,
    }
}

fn read_item(content: &str, format: Format) -> Result<Option<Value>> {
    let content = content.trim_start_matches('\u{feff}');
    let document: Value = match format {
        Format::Toml => toml_edit::de::from_str(content).context("MCP 配置不是有效 TOML")?,
        Format::Json => serde_json::from_str(content).context("MCP 配置不是有效 JSON")?,
    };
    let root = document.as_object().context("MCP 配置必须是对象")?;
    let section = if matches!(format, Format::Toml) {
        "mcp_servers"
    } else {
        "mcpServers"
    };
    let Some(servers) = root.get(section) else {
        return Ok(None);
    };
    Ok(servers
        .as_object()
        .context("MCP 服务列表必须是对象")?
        .get(SERVER_NAME)
        .cloned())
}

fn inspect_item(
    item: Option<&Value>,
    binary: &Path,
    name: &str,
    platform: Platform,
) -> (bool, bool, String) {
    let Some(item) = item else {
        return (false, false, "未配置".into());
    };
    let command = item["command"].as_str().unwrap_or("");
    if item["args"].as_array().is_some_and(|args| {
        args.iter().any(|arg| {
            arg.as_str()
                .is_some_and(|arg| arg.ends_with("mcp_server.py"))
        })
    }) {
        return (
            false,
            true,
            "旧版 Python 配置，点击修复后改用应用内置工具。".into(),
        );
    }
    if !executable_matches(command, binary, platform) {
        return (
            false,
            true,
            "应用位置已变化或原程序不存在，点击修复以更新入口。".into(),
        );
    }
    if item["args"] != json!(["--mcp"])
        || item["env"]["REDMI_WATCH_AGENT"] != name
        || item["enabled"] == false
    {
        return (
            false,
            true,
            "启动参数或启用状态需要更新，请点击修复。".into(),
        );
    }
    (true, false, "已配置应用内置工具。".into())
}

fn status(config: &AgentConfig, binary: &Path, platform: Platform) -> AgentStatus {
    let installed = config.parent.exists() || config.path.exists();
    let (configured, needs_repair, message) = if !config.path.exists() {
        (
            false,
            false,
            if installed {
                "配置文件尚不存在"
            } else {
                "未检测到安装目录"
            }
            .into(),
        )
    } else {
        match fs::read_to_string(&config.path)
            .context("无法读取 MCP 配置")
            .and_then(|content| read_item(&content, config.format))
        {
            Ok(item) => inspect_item(item.as_ref(), binary, config.name, platform),
            Err(error) => (false, false, format!("{error}；原文件已保留。")),
        }
    };
    AgentStatus {
        key: config.key.into(),
        name: config.name.into(),
        installed,
        configured,
        needs_repair,
        message,
        config_path: config.path.to_string_lossy().into(),
    }
}

pub fn list_agent_statuses() -> Result<Vec<AgentStatus>> {
    let paths = ConfigPaths::current()?;
    let binary = get_binary_path();
    Ok(configs(&paths)
        .iter()
        .map(|config| status(config, &binary, paths.platform))
        .collect())
}

fn update_toml(content: &str, enable: bool, binary: &str, name: &str) -> Result<String> {
    let mut document = content
        .trim_start_matches('\u{feff}')
        .parse::<DocumentMut>()
        .context("现有 MCP 配置不是有效 TOML，已保留原文件")?;
    if document.get("mcp_servers").is_none() {
        if !enable {
            return Ok(content.into());
        }
        document["mcp_servers"] = Item::Table(Table::new());
    }
    let servers = document["mcp_servers"]
        .as_table_like_mut()
        .context("mcp_servers 必须是表，已保留原文件")?;
    if enable {
        let mut entry = Table::new();
        entry["command"] = value(binary);
        let mut args = Array::new();
        args.push("--mcp");
        entry["args"] = value(args);
        let mut env = Table::new();
        env["REDMI_WATCH_AGENT"] = value(name);
        entry["env"] = Item::Table(env);
        servers.insert(SERVER_NAME, Item::Table(entry));
    } else {
        servers.remove(SERVER_NAME);
    }
    Ok(document.to_string())
}

fn update_json(content: &str, enable: bool, binary: &str, name: &str) -> Result<String> {
    let mut document: Value = serde_json::from_str(content.trim_start_matches('\u{feff}'))
        .context("现有 MCP 配置不是有效 JSON，已保留原文件")?;
    let root = document
        .as_object_mut()
        .context("MCP 配置必须是 JSON 对象")?;
    let servers = root
        .entry("mcpServers")
        .or_insert(json!({}))
        .as_object_mut()
        .context("mcpServers 必须是对象")?;
    if enable {
        servers.insert(
            SERVER_NAME.into(),
            json!({"command": binary, "args": ["--mcp"], "env": {"REDMI_WATCH_AGENT": name}}),
        );
    } else {
        servers.remove(SERVER_NAME);
    }
    Ok(serde_json::to_string_pretty(&document)? + "\n")
}

pub(crate) fn backup_config(path: &Path) -> Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let backup = path.with_file_name(format!(
        "{}.redmi-watch.bak",
        path.file_name()
            .context("缺少配置文件名")?
            .to_string_lossy()
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&backup) {
        Ok(mut file) => {
            use std::io::Write;
            file.write_all(&fs::read(path)?)?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error).context("备份原配置失败"),
    }
    Ok(())
}

fn toggle_config(config: &AgentConfig, enable: bool, binary: &Path) -> Result<bool> {
    if !enable && !config.path.exists() {
        return Ok(false);
    }
    let content = if config.path.exists() {
        fs::read_to_string(&config.path)?
    } else if matches!(config.format, Format::Toml) {
        String::new()
    } else {
        "{}".into()
    };
    let binary = binary.to_str().context("应用路径不是有效 Unicode")?;
    let updated = match config.format {
        Format::Toml => update_toml(&content, enable, binary, config.name)?,
        Format::Json => update_json(&content, enable, binary, config.name)?,
    };
    if updated != content {
        backup_config(&config.path)?;
        write_text_private(&config.path, &updated)?;
    }
    Ok(enable)
}

pub fn toggle_agent(key: &str, enable: bool) -> Result<bool> {
    let paths = ConfigPaths::current()?;
    let config = configs(&paths)
        .into_iter()
        .find(|config| config.key == key)
        .context("此系统不支持该 Agent")?;
    toggle_config(&config, enable, &get_binary_path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_windows_paths_round_trip_and_preserve_other_servers_and_comments() {
        let original = "# keep this comment\nmodel = 'example'\n[mcp_servers.other]\ncommand = 'other'\n[mcp_servers.'redmi_watch_questions'] # legacy entry\ncommand = 'python3'\nargs = ['C:/old/mcp_server.py']\n[mcp_servers.redmi_watch_questions.env]\nOLD = 'value'\n[features]\nhooks = true\n";
        let binary = r#"C:\Program Files\小明's % Tools & (dev)\agent-command-pilot.exe"#;
        let updated = update_toml(original, true, binary, "Codex").unwrap();
        let parsed: Value = toml_edit::de::from_str(&updated).unwrap();
        assert_eq!(parsed["mcp_servers"][SERVER_NAME]["command"], binary);
        assert_eq!(parsed["mcp_servers"]["other"]["command"], "other");
        assert_eq!(parsed["features"]["hooks"], true);
        assert!(updated.contains("# keep this comment"));
        assert!(!updated.contains("mcp_server.py"));
        assert!(!updated.contains("OLD"));
        assert_eq!(
            update_toml(&updated, true, binary, "Codex").unwrap(),
            updated
        );
        let removed: Value =
            toml_edit::de::from_str(&update_toml(&updated, false, binary, "Codex").unwrap())
                .unwrap();
        assert!(removed["mcp_servers"].get(SERVER_NAME).is_none());
        assert_eq!(removed["mcp_servers"]["other"]["command"], "other");
        assert!(update_toml("[broken", true, binary, "Codex").is_err());
    }

    #[test]
    fn inline_toml_and_json_preserve_unrelated_configuration() {
        let binary = r"D:\Apps\指令助手\agent-command-pilot.exe";
        let updated = update_toml(
            "mcp_servers = { other = { command = 'other' } }\n",
            true,
            binary,
            "Codex",
        )
        .unwrap();
        assert_eq!(
            read_item(&updated, Format::Toml).unwrap().unwrap()["command"],
            binary
        );
        let original = r#"{"theme":"dark","mcpServers":{"other":{"command":"other"},"redmi_watch_questions":{"command":"python3","args":["old/mcp_server.py"]}}}"#;
        let updated = update_json(original, true, binary, "Claude Desktop").unwrap();
        let parsed: Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(parsed["mcpServers"][SERVER_NAME]["command"], binary);
        assert_eq!(parsed["mcpServers"]["other"]["command"], "other");
        assert_eq!(parsed["theme"], "dark");
        assert_eq!(
            update_json(&updated, true, binary, "Claude Desktop").unwrap(),
            updated
        );
        let removed: Value =
            serde_json::from_str(&update_json(&updated, false, binary, "Claude Desktop").unwrap())
                .unwrap();
        assert!(removed["mcpServers"].get(SERVER_NAME).is_none());
        assert_eq!(removed["mcpServers"]["other"]["command"], "other");
    }

    #[test]
    fn migration_is_atomic_backed_up_and_status_rejects_old_or_moved_entries() {
        let root = std::env::temp_dir().join(format!("pilot-mcp-{:016x}", rand::random::<u64>()));
        fs::create_dir_all(&root).unwrap();
        let binary = root.join("agent-command-pilot.exe");
        fs::write(&binary, "test binary").unwrap();
        let path = root.join("config.toml");
        let config = AgentConfig {
            key: "codex",
            name: "Codex",
            path: path.clone(),
            parent: root.clone(),
            format: Format::Toml,
        };
        let legacy = "[mcp_servers.redmi_watch_questions]\ncommand = 'python3'\nargs = ['C:/old/mcp_server.py']\n";
        fs::write(&path, legacy).unwrap();
        assert!(status(&config, &binary, Platform::Windows).needs_repair);
        assert!(!status(&config, &binary, Platform::Windows).configured);
        toggle_config(&config, true, &binary).unwrap();
        assert!(status(&config, &binary, Platform::Windows).configured);
        assert_eq!(
            fs::read_to_string(root.join("config.toml.redmi-watch.bak")).unwrap(),
            legacy
        );
        let moved = root.join("moved.exe");
        fs::rename(&binary, &moved).unwrap();
        assert!(status(&config, &moved, Platform::Windows).needs_repair);
        toggle_config(&config, true, &moved).unwrap();
        assert!(status(&config, &moved, Platform::Windows).configured);
        fs::write(&path, "[invalid").unwrap();
        assert!(toggle_config(&config, true, &moved).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "[invalid");
        fs::remove_dir_all(root).unwrap();
    }
}
