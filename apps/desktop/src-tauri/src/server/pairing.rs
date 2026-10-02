use rand::Rng;
use sha2::{Digest, Sha256};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const WATCH_ONLINE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Default)]
struct WatchAuthentication {
    token_hash: Option<String>,
    last_seen: Option<Instant>,
    last_seen_at: Option<f64>,
    last_peer: Option<IpAddr>,
    auth_failed: bool,
}

pub struct WatchConnectionInfo {
    pub watch_paired: bool,
    pub watch_connection_status: &'static str,
    pub watch_last_seen_at: Option<f64>,
}

pub struct PairingManager {
    current_code: Mutex<Option<(String, Instant)>>,
    authentication: Mutex<WatchAuthentication>,
    persistence_path: Option<std::path::PathBuf>,
}

fn auth_file_path() -> std::path::PathBuf {
    #[cfg(unix)]
    let base = std::env::var("HOME").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from("."));
    #[cfg(windows)]
    let base = std::env::var("USERPROFILE").map(std::path::PathBuf::from).unwrap_or_else(|_| std::path::PathBuf::from("."));

    base.join(".agent-command-pilot").join("watch_auth.json")
}

fn legacy_auth_file_path() -> std::path::PathBuf {
    std::path::PathBuf::from("server/.watch_auth.json")
}

impl PairingManager {
    #[cfg(test)]
    pub fn for_test() -> Self {
        Self { current_code: Mutex::new(None), authentication: Mutex::new(WatchAuthentication::default()), persistence_path: None }
    }
    pub fn new() -> Self {
        let mut initial_hash = None;

        // Try reading persisted auth token hash
        let path = auth_file_path();
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(h) = v.get("token_sha256").and_then(|s| s.as_str()) {
                        log::info!("[Pairing] Restored previous watch authentication token hash");
                        initial_hash = Some(h.to_string());
                    }
                }
            }
        } else if legacy_auth_file_path().exists() {
            if let Ok(content) = std::fs::read_to_string(legacy_auth_file_path()) {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(h) = v.get("token_sha256").and_then(|s| s.as_str()) {
                        log::info!("[Pairing] Restored legacy watch authentication token hash from server/.watch_auth.json");
                        initial_hash = Some(h.to_string());
                    }
                }
            }
        }

        Self {
            current_code: Mutex::new(None),
            authentication: Mutex::new(WatchAuthentication { token_hash: initial_hash, ..Default::default() }),
            persistence_path: Some(path),
        }
    }

    /// Generate a 6-digit pairing code valid for five minutes.
    pub fn generate_code(&self) -> String {
        let mut rng = rand::thread_rng();
        let code = format!("{:06}", rng.gen_range(100000..=999999));
        let mut lock = self.current_code.lock().unwrap();
        *lock = Some((code.clone(), Instant::now()));
        log::info!("[Pairing] New 6-digit watch pairing code generated: {}", code);
        code
    }

    /// Get current valid pairing code if not expired
    pub fn get_valid_code(&self) -> Option<String> {
        let mut lock = self.current_code.lock().unwrap();
        if let Some((code, created_at)) = lock.as_ref() {
            if created_at.elapsed() < Duration::from_secs(300) {
                return Some(code.clone());
            } else {
                *lock = None;
            }
        }
        None
    }

    /// Verify pairing code and issue a bearer token
    pub fn pair_with_code(&self, code: &str) -> Option<String> {
        let mut lock = self.current_code.lock().unwrap();
        let valid = match lock.as_ref() {
            Some((stored_code, created_at)) => {
                stored_code == code.trim() && created_at.elapsed() < Duration::from_secs(300)
            }
            None => false,
        };

        if valid {
            *lock = None; // Single use
            let token = format!("{:032x}", rand::random::<u128>());
            let mut hasher = Sha256::new();
            hasher.update(token.as_bytes());
            let hash = hex::encode(hasher.finalize());

            // Persist token hash to file
            if let Some(save_path) = &self.persistence_path {
                if let Err(error) = crate::integrations::permission_rules::write_private(save_path, &serde_json::json!({ "token_sha256": hash })) {
                    log::error!("[Pairing] Cannot persist pairing: {error}");
                    return None;
                }
            }
            *self.authentication.lock().unwrap() = WatchAuthentication { token_hash: Some(hash), ..Default::default() };

            log::info!("[Pairing] Watch successfully paired using code {}", code);
            Some(token)
        } else {
            let current = lock.as_ref().map(|(c, _)| c.clone());
            log::warn!("[Pairing] Invalid code attempt: got '{}', current active code is '{:?}'", code, current);
            None
        }
    }

    /// A stored pairing credential does not imply the watch is currently online.
    pub fn has_paired_watch(&self) -> bool {
        self.authentication.lock().unwrap().token_hash.is_some()
    }

    /// Authenticate bearer token from watch request header
    pub fn authenticate(&self, token: &str) -> bool {
        Self::token_matches(&self.authentication.lock().unwrap(), token)
    }

    fn token_matches(auth: &WatchAuthentication, token: &str) -> bool {
        let Some(active_hash) = auth.token_hash.as_ref() else {
            return false;
        };
        let mut hasher = Sha256::new();
        hasher.update(token.trim().as_bytes());
        let input_hash = hex::encode(hasher.finalize());
        input_hash == *active_hash
    }

    /// Only authenticated LAN requests establish watch connectivity. Local
    /// simulator traffic must never call this method.
    pub fn authenticate_watch(&self, token: &str, peer: IpAddr) -> bool {
        let mut auth = self.authentication.lock().unwrap();
        let valid = Self::token_matches(&auth, token);
        if valid {
            auth.last_seen = Some(Instant::now());
            auth.last_seen_at = Some(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64());
            auth.last_peer = Some(peer);
            auth.auth_failed = false;
        } else if !token.trim().is_empty() {
            // A rejected request from another LAN client must not replace a
            // recently verified watch's healthy status or revoke its credential.
            let other_client_while_online = auth.last_peer.is_some_and(|known| known != peer)
                && auth.last_seen.is_some_and(|seen| seen.elapsed() < WATCH_ONLINE_TIMEOUT);
            if !other_client_while_online {
                auth.auth_failed = true;
            }
        }
        valid
    }

    pub fn connection_info(&self, service_running: bool) -> WatchConnectionInfo {
        let auth = self.authentication.lock().unwrap();
        let paired = auth.token_hash.is_some();
        let status = if auth.auth_failed {
            "auth_failed"
        } else if !paired {
            "unpaired"
        } else if service_running && auth.last_seen.is_some_and(|seen| seen.elapsed() < WATCH_ONLINE_TIMEOUT) {
            "online"
        } else if !service_running || auth.last_seen_at.is_some() {
            "offline"
        } else {
            "waiting"
        };
        WatchConnectionInfo {
            watch_paired: paired,
            watch_connection_status: status,
            watch_last_seen_at: auth.last_seen_at,
        }
    }

    pub fn mark_offline(&self) {
        self.authentication.lock().unwrap().last_seen = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paired_manager() -> (PairingManager, String) {
        let pm = PairingManager::for_test();
        let code = pm.generate_code();
        let token = pm.pair_with_code(&code).unwrap();
        (pm, token)
    }

    #[test]
    fn saved_credential_waits_for_an_authenticated_watch_request() {
        let (pm, token) = paired_manager();
        assert_eq!(pm.connection_info(true).watch_connection_status, "waiting");
        assert!(pm.connection_info(true).watch_last_seen_at.is_none());
        assert!(pm.authenticate(&token));
        assert_eq!(pm.connection_info(true).watch_connection_status, "waiting");
        assert!(pm.authenticate_watch(&token, "192.168.1.20".parse().unwrap()));
        assert_eq!(pm.connection_info(true).watch_connection_status, "online");
        assert!(pm.connection_info(true).watch_last_seen_at.is_some());
    }

    #[test]
    fn sleeping_and_service_restart_keep_pairing_without_claiming_online() {
        let (pm, token) = paired_manager();
        let peer = "192.168.1.20".parse().unwrap();
        assert!(pm.authenticate_watch(&token, peer));
        pm.authentication.lock().unwrap().last_seen = Some(Instant::now() - WATCH_ONLINE_TIMEOUT - Duration::from_secs(1));
        assert_eq!(pm.connection_info(true).watch_connection_status, "offline");
        assert!(pm.connection_info(true).watch_paired);
        assert!(pm.authenticate_watch(&token, peer));
        assert_eq!(pm.connection_info(false).watch_connection_status, "offline");
        pm.mark_offline();
        assert_eq!(pm.connection_info(true).watch_connection_status, "offline");
        assert!(pm.authenticate_watch(&token, peer));
        assert_eq!(pm.connection_info(true).watch_connection_status, "online");
    }

    #[test]
    fn authentication_failure_is_visible_and_recovers_without_revoking_credentials() {
        let (pm, token) = paired_manager();
        let peer = "192.168.1.20".parse().unwrap();
        assert!(!pm.authenticate_watch("obsolete-token", peer));
        assert_eq!(pm.connection_info(true).watch_connection_status, "auth_failed");
        assert!(pm.has_paired_watch());
        pm.generate_code();
        assert_eq!(pm.connection_info(true).watch_connection_status, "auth_failed");
        assert!(pm.authenticate_watch(&token, peer));
        assert_eq!(pm.connection_info(true).watch_connection_status, "online");
        assert!(!pm.authenticate_watch("obsolete-token", peer));
        assert_eq!(pm.connection_info(true).watch_connection_status, "auth_failed");
        let code = pm.generate_code();
        let new_token = pm.pair_with_code(&code).unwrap();
        assert_eq!(pm.connection_info(true).watch_connection_status, "waiting");
        assert!(!pm.authenticate(&token));
        assert!(pm.authenticate_watch(&new_token, peer));
    }

    #[test]
    fn other_lan_clients_cannot_displace_an_online_watch() {
        let (pm, token) = paired_manager();
        assert!(pm.authenticate_watch(&token, "192.168.1.20".parse().unwrap()));
        assert!(!pm.authenticate_watch("invalid-token", "192.168.1.21".parse().unwrap()));
        assert_eq!(pm.connection_info(true).watch_connection_status, "online");
        assert!(pm.authenticate(&token));
    }

    #[test]
    fn pairing_status_is_independent_of_code_generation() {
        let pm = PairingManager::for_test();
        pm.generate_code();
        assert!(!pm.has_paired_watch());

        pm.authentication.lock().unwrap().token_hash = Some("saved-credential-hash".to_string());
        pm.generate_code();
        assert!(pm.has_paired_watch());
        *pm.current_code.lock().unwrap() = None;
        assert!(pm.has_paired_watch());
    }

    #[test]
    fn pairing_code_expires_after_five_minutes() {
        let pm = PairingManager::for_test();
        assert!(pm.get_valid_code().is_none());
        let code = pm.generate_code();
        *pm.current_code.lock().unwrap() = Some((code.clone(), Instant::now() - Duration::from_secs(301)));
        assert!(pm.get_valid_code().is_none());
        assert!(pm.pair_with_code(&code).is_none());
    }

    #[test]
    fn test_pairing_lifecycle() {
        let pm = PairingManager::for_test();
        let code = pm.generate_code();
        assert_eq!(code.len(), 6);
        assert!(code.chars().all(|c| c.is_ascii_digit()));

        // Pairing with incorrect code fails
        assert!(pm.pair_with_code("000000").is_none());

        // Pairing with correct code succeeds and returns token
        let token = pm.pair_with_code(&code);
        assert!(token.is_some());
        let token = token.unwrap();

        // Valid token authenticates
        assert!(pm.authenticate(&token));
        assert!(!pm.authenticate("invalid_token"));

        // Code is single use
        assert!(pm.pair_with_code(&code).is_none());
    }
}
