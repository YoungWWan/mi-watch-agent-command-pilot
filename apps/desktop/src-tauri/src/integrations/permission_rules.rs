use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub fn home_dir() -> PathBuf {
    std::env::var(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn store_path() -> PathBuf {
    super::paths::ConfigPaths::current()
        .map(|paths| paths.codex)
        .unwrap_or_else(|_| home_dir().join(".codex"))
        .join("redmi_watch_permission_rules.json")
}

pub fn read_document(path: &Path) -> Result<Value> {
    if !path.exists() {
        return Ok(json!({"version": 1, "rules": []}));
    }
    let value: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    if !value["rules"].is_array() {
        anyhow::bail!("信任规则文件格式无效");
    }
    Ok(value)
}

pub fn write_private(path: &Path, value: &Value) -> Result<()> {
    write_text_private(path, &serde_json::to_string_pretty(value)?)
}

pub(crate) fn write_text_private(path: &Path, text: &str) -> Result<()> {
    // Preserve user-managed symlinks while replacing the target atomically.
    let resolved;
    let path = if std::fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        resolved = std::fs::canonicalize(path)?;
        resolved.as_path()
    } else {
        path
    };
    let parent = path.parent().context("Missing parent directory")?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".redmi-watch-{:016x}.json", rand::random::<u64>()));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        use std::io::Write;
        let mut file = options.open(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, path)?;
        Ok(())
    })();
    if temporary.exists() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

pub fn list() -> Result<Vec<Value>> {
    Ok(read_document(&store_path())?["rules"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["id"].is_string())
        .cloned()
        .collect())
}

pub fn delete(id: &str) -> Result<()> {
    delete_at(&store_path(), id)
}

fn delete_at(path: &Path, id: &str) -> Result<()> {
    let mut document = read_document(path)?;
    let rules = document["rules"].as_array_mut().unwrap();
    let length = rules.len();
    rules.retain(|rule| rule["id"].as_str() != Some(id));
    if rules.len() == length {
        anyhow::bail!("未找到此信任规则，请刷新列表");
    }
    write_private(path, &document)
}

pub fn rule_for(data: &Value) -> Option<Value> {
    let cwd = data["cwd"].as_str()?.trim();
    let expanded = if cwd.starts_with("~/") {
        home_dir().join(&cwd[2..])
    } else {
        PathBuf::from(cwd)
    };
    if !expanded.is_absolute() {
        return None;
    }
    let cwd = std::fs::canonicalize(&expanded)
        .ok()?
        .to_string_lossy()
        .to_string();
    let tool = data
        .get("tool")
        .or_else(|| data.get("tool_name"))
        .or_else(|| data.get("name"))?
        .as_str()?;
    if tool.trim().is_empty() {
        return None;
    }
    let mut input = data
        .get("tool_input")
        .filter(|v| !v.is_null())
        .or_else(|| data.get("input"))
        .cloned()
        .unwrap_or(json!({}));
    if let Some(map) = input.as_object_mut() {
        map.remove("justification");
    }
    // serde_json's default BTreeMap ordering matches the legacy Python sorted compact JSON.
    let identity = json!({"app": "Codex", "cwd": cwd, "tool_name": tool, "tool_input": input});
    let encoded = serde_json::to_vec(&identity).ok()?;
    let id = hex::encode(Sha256::digest(encoded));
    let preview = ["command", "cmd", "path", "file_path", "query"]
        .iter()
        .find_map(|key| input[*key].as_str())
        .map(String::from)
        .unwrap_or_else(|| input.to_string());
    Some(
        json!({"id": id, "app": "Codex", "cwd": cwd, "tool_name": tool,
        "preview": preview.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(240).collect::<String>()}),
    )
}

pub fn is_trusted(rule: &Value) -> bool {
    list()
        .map(|rules| rules.iter().any(|value| value["id"] == rule["id"]))
        .unwrap_or(false)
}

pub fn remember(mut rule: Value) -> Result<()> {
    let path = store_path();
    let mut document = read_document(&path)?;
    let rules = document["rules"].as_array_mut().unwrap();
    if !rules.iter().any(|existing| existing["id"] == rule["id"]) {
        rule["created_at"] = json!(chrono::Utc::now().timestamp());
        rules.push(rule);
        write_private(&path, &document)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_rules_match_legacy_identity_and_revoke_only_one_rule() {
        let cwd = std::env::temp_dir().canonicalize().unwrap();
        let input = json!({"cwd": cwd, "tool_name": "exec_command", "tool_input": {"cmd": "echo test", "justification": "display"}});
        let rule = rule_for(&input).unwrap();
        let mut changed = input.clone();
        changed["tool_input"]["justification"] = json!("new explanation");
        assert_eq!(rule["id"], rule_for(&changed).unwrap()["id"]);
        changed["tool_input"]["cmd"] = json!("echo other");
        assert_ne!(rule["id"], rule_for(&changed).unwrap()["id"]);
        changed["cwd"] = json!("");
        assert!(rule_for(&changed).is_none());
        let path = cwd.join(format!("redmi-rules-test-{}.json", rand::random::<u64>()));
        write_private(&path, &json!({"version":1,"rules":[rule,{"id":"other"}]})).unwrap();
        delete_at(&path, rule_for(&input).unwrap()["id"].as_str().unwrap()).unwrap();
        assert_eq!(
            read_document(&path).unwrap()["rules"],
            json!([{"id":"other"}])
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::remove_file(path).unwrap();
    }
}
