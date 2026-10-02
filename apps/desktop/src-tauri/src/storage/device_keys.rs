//! Local device credentials. No OS keychain access; files are plaintext with owner-only
//! permissions on Unix. Refreshing Xiaomi devices repopulates this store without migration.
use anyhow::{anyhow, Context, Result};
use std::{fs, io::Write, path::{Path, PathBuf}};

fn store_dir() -> Result<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .ok_or_else(|| anyhow!("无法确定当前用户目录"))?;
    Ok(PathBuf::from(home).join(".agent-command-pilot").join("device-keys"))
}

fn key_path(dir: &Path, mac: &str) -> Result<PathBuf> {
    let mac = mac.trim().replace([':', '-'], "").to_uppercase();
    if mac.len() != 12 || !mac.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(anyhow!("无效的设备 MAC 地址"));
    }
    Ok(dir.join(format!("{mac}.key")))
}

fn save_at(dir: &Path, mac: &str, authkey: &str) -> Result<()> {
    let path = key_path(dir, mac)?;
    let key = authkey.trim();
    if key.is_empty() { return Err(anyhow!("设备密钥为空")); }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)] {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir).context("创建本地凭据目录失败")?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    let temporary = dir.join(format!(".write-{:016x}", rand::random::<u64>()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| -> Result<()> {
        let mut file = options.open(&temporary)?;
        file.write_all(key.as_bytes())?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result.context("保存本地设备凭据失败")
}

pub fn save_authkey(mac: &str, authkey: &str) -> Result<()> {
    save_at(&store_dir()?, mac, authkey)
}

pub fn get_authkey(mac: &str) -> Option<String> {
    let path = key_path(&store_dir().ok()?, mac).ok()?;
    let key = fs::read_to_string(path).ok()?.trim().to_string();
    if key.is_empty() { None } else { Some(key) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_store_persists_and_replaces_credentials() {
        let dir = std::env::temp_dir().join(format!("pilot-keys-test-{:016x}", rand::random::<u64>()));
        save_at(&dir, " AA:BB:CC:DD:EE:FF ", " test-one ").unwrap();
        let path = key_path(&dir, "aa-bb-cc-dd-ee-ff").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "test-one");
        save_at(&dir, "AABBCCDDEEFF", "test-two").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "test-two");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
            assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        }
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn invalid_device_paths_and_empty_keys_are_rejected() {
        let dir = Path::new("unused-test-directory");
        for mac in ["../secret", "", "GG:BB:CC:DD:EE:FF"] {
            assert!(key_path(dir, mac).is_err());
        }
        assert!(save_at(dir, "AA:BB:CC:DD:EE:FF", " ").is_err());
    }
}
