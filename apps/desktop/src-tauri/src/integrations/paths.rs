use anyhow::{Context, Result};
use std::{ffi::OsString, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

pub(crate) struct ConfigPaths {
    pub home: PathBuf,
    pub codex: PathBuf,
    pub claude_code: PathBuf,
    pub claude_mcp: PathBuf,
    pub claude_desktop: Option<PathBuf>,
    pub kimi: PathBuf,
    pub platform: Platform,
}

impl ConfigPaths {
    pub fn current() -> Result<Self> {
        Self::from_environment(Platform::current(), |name| std::env::var_os(name))
    }

    fn from_environment(
        platform: Platform,
        lookup: impl Fn(&str) -> Option<OsString>,
    ) -> Result<Self> {
        let get = |name| {
            lookup(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
        };
        let home = if platform == Platform::Windows {
            get("USERPROFILE").or_else(|| {
                let mut drive = lookup("HOMEDRIVE")?;
                drive.push(lookup("HOMEPATH")?);
                Some(PathBuf::from(drive))
            })
        } else {
            get("HOME")
        }
        .context("无法确定当前用户目录")?;
        let codex = get("CODEX_HOME").unwrap_or_else(|| home.join(".codex"));
        let kimi = get("KIMI_CODE_HOME").unwrap_or_else(|| {
            let current = home.join(".kimi-code");
            let legacy = home.join(".kimi");
            if !current.exists() && legacy.exists() {
                legacy
            } else {
                current
            }
        });
        let claude_override = get("CLAUDE_CONFIG_DIR");
        let claude_code = claude_override
            .clone()
            .unwrap_or_else(|| home.join(".claude"));
        let claude_mcp = claude_override
            .map(|directory| directory.join(".claude.json"))
            .unwrap_or_else(|| home.join(".claude.json"));
        let claude_desktop = match platform {
            Platform::Windows => {
                let roaming =
                    get("APPDATA").unwrap_or_else(|| home.join("AppData").join("Roaming"));
                let local =
                    get("LOCALAPPDATA").unwrap_or_else(|| home.join("AppData").join("Local"));
                let standard = roaming.join("Claude").join("claude_desktop_config.json");
                // Prefer the documented location; adopt an existing MSIX config if it is
                // the only one present rather than creating a second, unused configuration.
                let packages = local.join("Packages");
                let mut alternatives = std::fs::read_dir(packages)
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| entry.file_name().to_string_lossy().starts_with("Claude_"))
                    .map(|entry| {
                        entry
                            .path()
                            .join("LocalCache")
                            .join("Roaming")
                            .join("Claude")
                            .join("claude_desktop_config.json")
                    })
                    .filter(|path| path.is_file())
                    .collect::<Vec<_>>();
                alternatives.sort();
                Some(if standard.is_file() {
                    standard
                } else {
                    alternatives.into_iter().next().unwrap_or(standard)
                })
            }
            Platform::MacOs => Some(
                home.join("Library")
                    .join("Application Support")
                    .join("Claude")
                    .join("claude_desktop_config.json"),
            ),
            Platform::Linux => None,
        };
        Ok(Self {
            home,
            codex,
            claude_code,
            claude_mcp,
            claude_desktop,
            kimi,
            platform,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths(platform: Platform, values: &[(&str, &str)]) -> ConfigPaths {
        ConfigPaths::from_environment(platform, |name| {
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        })
        .unwrap()
    }

    #[test]
    fn windows_uses_profile_and_roaming_appdata_without_unix_home() {
        let result = paths(
            Platform::Windows,
            &[
                ("USERPROFILE", "D:/用户/小明"),
                ("HOME", "/wrong"),
                ("APPDATA", "E:/Roaming"),
            ],
        );
        assert_eq!(result.codex, PathBuf::from("D:/用户/小明").join(".codex"));
        assert_eq!(
            result.claude_mcp,
            PathBuf::from("D:/用户/小明").join(".claude.json")
        );
        assert_eq!(
            result.claude_desktop.unwrap(),
            PathBuf::from("E:/Roaming").join("Claude/claude_desktop_config.json")
        );
        let fallback = paths(
            Platform::Windows,
            &[("HOMEDRIVE", "D:"), ("HOMEPATH", "/用户/小明")],
        );
        assert_eq!(fallback.home, PathBuf::from("D:/用户/小明"));
    }

    #[test]
    fn overrides_apply_to_mcp_and_hooks_in_the_same_profile() {
        let result = paths(
            Platform::Windows,
            &[
                ("USERPROFILE", "C:/Users/Test"),
                ("CODEX_HOME", "D:/Codex Work"),
                ("CLAUDE_CONFIG_DIR", "D:/Claude Work"),
                ("KIMI_CODE_HOME", "D:/Kimi Work"),
            ],
        );
        assert_eq!(
            result.kimi.join("mcp.json"),
            PathBuf::from("D:/Kimi Work").join("mcp.json")
        );
        assert_eq!(
            result.codex.join("hooks.json"),
            PathBuf::from("D:/Codex Work").join("hooks.json")
        );
        assert_eq!(
            result.claude_mcp,
            PathBuf::from("D:/Claude Work").join(".claude.json")
        );
        assert_eq!(
            result.claude_code.join("settings.json"),
            PathBuf::from("D:/Claude Work").join("settings.json")
        );
    }

    #[test]
    fn mac_keeps_its_existing_paths_and_missing_home_is_an_error() {
        let result = paths(Platform::MacOs, &[("HOME", "/Users/Test")]);
        assert_eq!(
            result.claude_desktop.unwrap(),
            PathBuf::from(
                "/Users/Test/Library/Application Support/Claude/claude_desktop_config.json"
            )
        );
        assert!(ConfigPaths::from_environment(Platform::Windows, |_| None).is_err());
        assert!(paths(Platform::Linux, &[("HOME", "/home/test")])
            .claude_desktop
            .is_none());
    }

    #[test]
    fn windows_adopts_existing_msix_config_but_prefers_standard_config() {
        let root = std::env::temp_dir().join(format!("pilot-msix-{:016x}", rand::random::<u64>()));
        let roaming = root.join("Roaming");
        let local = root.join("Local");
        let msix =
            local.join("Packages/Claude_test/LocalCache/Roaming/Claude/claude_desktop_config.json");
        std::fs::create_dir_all(msix.parent().unwrap()).unwrap();
        std::fs::write(&msix, "{}").unwrap();
        let lookup = |name: &str| match name {
            "USERPROFILE" => Some(root.clone().into_os_string()),
            "APPDATA" => Some(roaming.clone().into_os_string()),
            "LOCALAPPDATA" => Some(local.clone().into_os_string()),
            _ => None,
        };
        assert_eq!(
            ConfigPaths::from_environment(Platform::Windows, lookup)
                .unwrap()
                .claude_desktop
                .unwrap(),
            msix
        );
        let standard = roaming.join("Claude/claude_desktop_config.json");
        std::fs::create_dir_all(standard.parent().unwrap()).unwrap();
        std::fs::write(&standard, "{}").unwrap();
        assert_eq!(
            ConfigPaths::from_environment(Platform::Windows, lookup)
                .unwrap()
                .claude_desktop
                .unwrap(),
            standard
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
