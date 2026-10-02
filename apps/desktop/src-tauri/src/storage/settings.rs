use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct AppSettings {
    pub server_port: u16,
    pub last_connected_mac: Option<String>,
    pub paired_watch_token_hash: Option<String>,
    pub auto_reconnect: bool,
    pub ntfy_enabled: bool,
    pub ntfy_server: String,
    pub ntfy_topic: String,
    pub ntfy_include_content: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            server_port: 8000,
            last_connected_mac: None,
            paired_watch_token_hash: None,
            auto_reconnect: true,
            ntfy_enabled: false,
            ntfy_server: "https://ntfy.sh".into(),
            ntfy_topic: String::new(),
            ntfy_include_content: false,
        }
    }
}

fn home_dir() -> PathBuf {
    #[cfg(unix)]
    {
        std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
    }
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("."))
    }
}

fn config_path() -> PathBuf {
    let mut dir = home_dir();
    dir.push(".agent-command-pilot");
    let _ = fs::create_dir_all(&dir);
    dir.push("settings.json");
    dir
}

pub fn load_settings() -> AppSettings {
    let path = config_path();
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(settings) = serde_json::from_slice::<AppSettings>(&bytes) {
            return settings;
        }
    }
    AppSettings::default()
}

pub fn save_settings(settings: &AppSettings) -> Result<()> {
    let path = config_path();
    let json = serde_json::to_string_pretty(settings)
        .context("Failed to serialize settings")?;
    fs::write(&path, json)
        .context("Failed to write settings to disk")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn older_settings_keep_their_port_and_get_notification_defaults() {
        let settings: AppSettings = serde_json::from_str(r#"{"server_port":8123,"auto_reconnect":false}"#).unwrap();
        assert_eq!(settings.server_port,8123);
        assert!(!settings.auto_reconnect);
        assert!(!settings.ntfy_enabled);
        assert!(!settings.ntfy_include_content);
        assert!(settings.ntfy_topic.is_empty());
    }
}
