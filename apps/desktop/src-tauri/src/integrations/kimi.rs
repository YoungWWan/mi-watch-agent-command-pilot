//! Kimi's observation hooks hand work to the hub; decisions go through its authenticated API.
use anyhow::{bail, Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{net::IpAddr, path::Path, time::Duration};

pub(crate) fn read_token(home: &Path) -> Result<String> {
    let path = home.join("server.token");
    let metadata = std::fs::metadata(&path).context("Kimi 本地认证尚未配置，请修复钩子")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            bail!("Kimi server.token 必须仅当前用户可读写，请将权限设为 600");
        }
    }
    if !metadata.is_file() || metadata.len() > 4096 {
        bail!("Kimi 本地认证文件无效");
    }
    let token = std::fs::read_to_string(path)?.trim().to_owned();
    if token.is_empty() || token.chars().any(char::is_whitespace) {
        bail!("Kimi 本地认证文件无效");
    }
    Ok(token)
}

pub(crate) fn ensure_token(home: &Path) -> Result<()> {
    let path = home.join("server.token");
    // Never replace an existing token, including an invalid or user-managed symlink.
    if std::fs::symlink_metadata(&path).is_ok() {
        read_token(home)?;
        return Ok(());
    }
    std::fs::create_dir_all(home)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            use std::io::Write;
            let bytes: [u8; 32] = rand::random();
            let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
            file.write_all(token.as_bytes())?;
            file.sync_all()?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    read_token(home)?;
    Ok(())
}

#[derive(Deserialize)]
struct Instance {
    host: String,
    port: u16,
    server_id: String,
    #[serde(default)]
    heartbeat_at: u64,
}

fn instance_url(instance: &Instance) -> Option<String> {
    let host = if instance.host == "localhost" {
        "127.0.0.1"
    } else {
        &instance.host
    };
    let ip: IpAddr = host.parse().ok()?;
    if !ip.is_loopback() || instance.port == 0 || instance.server_id.is_empty() {
        return None;
    }
    Some(match ip {
        IpAddr::V4(_) => format!("http://{ip}:{}", instance.port),
        IpAddr::V6(_) => format!("http://[{ip}]:{}", instance.port),
    })
}

// Deliberately not Debug: the client owns a credential that must never enter logs.
#[derive(Clone)]
pub(crate) struct Api {
    client: reqwest::Client,
    origin: String,
    token: String,
    pub server_id: String,
}

impl Api {
    async fn request(
        &self,
        method: reqwest::Method,
        parts: &[&str],
        query: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<Value> {
        let mut url = reqwest::Url::parse(&self.origin)?;
        url.path_segments_mut()
            .map_err(|_| anyhow::anyhow!("Kimi 地址无效"))?
            .extend(["api", "v1"])
            .extend(parts.iter().copied());
        let mut request = self
            .client
            .request(method, url)
            .bearer_auth(&self.token)
            .query(query);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.context("无法连接 Kimi 本地服务")?;
        let status = response.status();
        let value: Value = response.json().await.context("Kimi 接口响应格式无效")?;
        let code = value["code"].as_i64().context("Kimi 接口缺少业务状态码")?;
        if !status.is_success() || code != 0 {
            // Upstream messages may contain user data. Log only the numeric result.
            bail!("Kimi 接口失败（HTTP {}，code {}）", status.as_u16(), code);
        }
        Ok(value["data"].clone())
    }

    pub async fn snapshot(&self, session: &str) -> Result<Value> {
        self.request(
            reqwest::Method::GET,
            &["sessions", session, "snapshot"],
            &[],
            None,
        )
        .await
    }

    pub async fn approvals(&self, session: &str) -> Result<Value> {
        self.request(
            reqwest::Method::GET,
            &["sessions", session, "approvals"],
            &[("status", "pending".into())],
            None,
        )
        .await
    }

    pub async fn resolve(&self, session: &str, approval: &str, action: &str) -> Result<()> {
        let decision = match action {
            "confirm" => "approved",
            "reject" => "rejected",
            _ => bail!("无效的手表审批动作"),
        };
        let body = json!({"decision":decision});
        let data = self
            .request(
                reqwest::Method::POST,
                &["sessions", session, "approvals", approval],
                &[],
                Some(&body),
            )
            .await?;
        if data["resolved"] != true {
            bail!("Kimi 未确认审批结果");
        }
        Ok(())
    }

    pub async fn turn(&self, session: &str, turn_id: u64) -> Result<Option<Value>> {
        let data = self
            .request(
                reqwest::Method::GET,
                &["sessions", session, "transcript"],
                &[
                    ("agent_id", "main".into()),
                    ("page_size", "1".into()),
                    ("before_turn", format!("t{}", turn_id.saturating_add(1))),
                ],
                None,
            )
            .await?;
        let id = format!("t{turn_id}");
        Ok(data["items"]
            .as_array()
            .context("Kimi 轮次数据无效")?
            .iter()
            .find(|item| item["kind"] == "turn" && item["turnId"] == id)
            .cloned())
    }
}

pub(crate) async fn discover(home: &Path) -> Result<Vec<Api>> {
    let token = read_token(home)?;
    let mut instances = std::fs::read_dir(home.join("server/instances"))
        .context("Kimi 本地服务未启动，请打开 Kimi Code 或 kimi web")?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| {
            std::fs::read(entry.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Instance>(&bytes).ok())
        })
        .filter(|instance| instance_url(instance).is_some())
        .collect::<Vec<_>>();
    instances.sort_by_key(|instance| std::cmp::Reverse(instance.heartbeat_at));
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(2))
        .build()?;
    let mut checks = tokio::task::JoinSet::new();
    for instance in instances.into_iter().take(16) {
        let mut api = Api {
            client: client.clone(),
            origin: instance_url(&instance).unwrap(),
            token: token.clone(),
            server_id: instance.server_id,
        };
        checks.spawn(async move {
            let meta = api
                .request(reqwest::Method::GET, &["meta"], &[], None)
                .await
                .ok()?;
            // Kimi Code 1.0.4 assigns separate IDs to the registry entry and API server.
            // The registry locates the port; authenticated /meta is the runtime identity.
            api.server_id = meta["server_id"].as_str().filter(|id| valid_id(id))?.into();
            Some(api)
        });
    }
    let mut found = vec![];
    while let Some(result) = checks.join_next().await {
        if let Ok(Some(api)) = result {
            if !found
                .iter()
                .any(|existing: &Api| existing.server_id == api.server_id)
            {
                found.push(api);
            }
        }
    }
    if found.is_empty() {
        bail!("未连接到已认证的 Kimi 本地服务，请打开 Kimi Code 并检查版本");
    }
    Ok(found)
}

pub(crate) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum BridgeEvent {
    Approval {
        session_id: String,
        approval_id: String,
        tool_call_id: String,
        tool_name: String,
        agent_id: String,
        turn_id: u64,
        cwd: String,
    },
    Completion {
        session_id: String,
        server_id: String,
        turn_id: u64,
        cwd: String,
    },
}

impl BridgeEvent {
    pub fn approval(data: &Value) -> Option<Self> {
        if data["hook_event_name"] != "PermissionRequest" {
            return None;
        }
        let session = data["session_id"].as_str()?;
        let approval = data["approval_id"]
            .as_str()
            .or_else(|| data["id"].as_str())?;
        let tool_call = data["tool_call_id"].as_str()?;
        let agent = data["agent_id"].as_str()?;
        let tool = data["tool_name"].as_str()?;
        if ![session, approval, tool_call, agent]
            .iter()
            .all(|id| valid_id(id))
            || tool.is_empty()
        {
            return None;
        }
        Some(Self::Approval {
            session_id: session.into(),
            approval_id: approval.into(),
            tool_call_id: tool_call.into(),
            agent_id: agent.into(),
            tool_name: tool.into(),
            turn_id: data["turn_id"].as_u64()?,
            cwd: data["cwd"].as_str().unwrap_or("").into(),
        })
    }

    pub fn valid(&self) -> bool {
        match self {
            Self::Approval {
                session_id,
                approval_id,
                tool_call_id,
                agent_id,
                tool_name,
                ..
            } => {
                [session_id, approval_id, tool_call_id, agent_id]
                    .iter()
                    .all(|id| valid_id(id))
                    && !tool_name.is_empty()
            }
            Self::Completion {
                session_id,
                server_id,
                ..
            } => [session_id, server_id].iter().all(|id| valid_id(id)),
        }
    }

    pub fn matches_approval(&self, item: &Value) -> bool {
        let Self::Approval {
            session_id,
            approval_id,
            tool_call_id,
            tool_name,
            agent_id,
            turn_id,
            ..
        } = self
        else {
            return false;
        };
        item["session_id"] == *session_id
            && item["approval_id"] == *approval_id
            && item["tool_call_id"] == *tool_call_id
            && item["tool_name"] == *tool_name
            && item["agent_id"] == *agent_id
            && item["turn_id"] == *turn_id
            && item["expires_at"]
                .as_str()
                .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
                .is_some_and(|expires| expires > chrono::Utc::now())
    }
}

pub(crate) async fn completion_event(home: &Path, data: &Value) -> Result<BridgeEvent> {
    let session = data["session_id"]
        .as_str()
        .filter(|id| valid_id(id))
        .context("Kimi 完成事件缺少会话 ID")?;
    let mut found = vec![];
    for api in discover(home).await? {
        if let Ok(snapshot) = api.snapshot(session).await {
            if let Some(turn) = snapshot["in_flight_turn"]["turn_id"].as_u64() {
                found.push(BridgeEvent::Completion {
                    session_id: session.into(),
                    server_id: api.server_id,
                    turn_id: turn,
                    cwd: data["cwd"].as_str().unwrap_or("").into(),
                });
            }
        }
    }
    if found.len() != 1 {
        bail!("无法唯一定位 Kimi 当前轮次，已保留电脑端结果");
    }
    Ok(found.remove(0))
}

pub(crate) fn final_text(turn: &Value) -> Option<String> {
    if turn["state"] != "completed" {
        return None;
    }
    let step = turn["steps"].as_array()?.last()?;
    // Use the last step only, so a pre-tool progress message never becomes the final reply.
    let text = step["frames"]
        .as_array()?
        .iter()
        .filter(|frame| frame["kind"] == "text" && frame["role"] == "assistant")
        .filter_map(|frame| frame["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    (!text.trim().is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_token_is_created_once_and_invalid_existing_files_are_preserved() {
        let home = std::env::temp_dir().join(format!("kimi-token-{:016x}", rand::random::<u64>()));
        ensure_token(&home).unwrap();
        let first = read_token(&home).unwrap();
        assert_eq!(first.len(), 43);
        ensure_token(&home).unwrap();
        assert_eq!(read_token(&home).unwrap(), first);
        std::fs::write(home.join("server.token"), "").unwrap();
        assert!(ensure_token(&home).is_err());
        assert_eq!(std::fs::read(home.join("server.token")).unwrap(), b"");
        std::fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn discovery_only_sends_credentials_to_literal_loopback_hosts() {
        for (host, ok) in [
            ("127.0.0.1", true),
            ("::1", true),
            ("localhost", true),
            ("0.0.0.0", false),
            ("192.168.1.2", false),
            ("example.com", false),
        ] {
            let instance = Instance {
                host: host.into(),
                port: 58627,
                server_id: "server".into(),
                heartbeat_at: 0,
            };
            assert_eq!(instance_url(&instance).is_some(), ok);
        }
        assert!(!valid_id("../other"));
        assert!(!valid_id("id?secret"));
    }

    #[test]
    fn approval_requires_exact_request_and_tool_identity() {
        let event=BridgeEvent::approval(&json!({"hook_event_name":"PermissionRequest","session_id":"session_test","id":"approval_test","tool_call_id":"call_test","agent_id":"main","tool_name":"Bash","turn_id":2})).unwrap();
        let item = json!({"session_id":"session_test","approval_id":"approval_test","tool_call_id":"call_test","agent_id":"main","tool_name":"Bash","turn_id":2,"expires_at":"2099-01-01T00:00:00Z"});
        assert!(event.matches_approval(&item));
        for field in [
            "session_id",
            "approval_id",
            "tool_call_id",
            "agent_id",
            "tool_name",
            "turn_id",
            "expires_at",
        ] {
            let mut wrong = item.clone();
            wrong[field] = json!("other");
            assert!(!event.matches_approval(&wrong));
        }
        assert!(BridgeEvent::approval(
            &json!({"hook_event_name":"PermissionRequest","session_id":"session_test"})
        )
        .is_none());
    }

    #[test]
    fn completion_uses_final_assistant_text_without_thinking_or_earlier_progress() {
        let turn = json!({"state":"completed","steps":[{"frames":[{"kind":"text","role":"assistant","text":"working"}]},{"frames":[{"kind":"thinking","text":"private"},{"kind":"text","role":"user","text":"prompt"},{"kind":"text","role":"assistant","text":"完成了"},{"kind":"text","role":"assistant","text":"验证通过"}]}]});
        assert_eq!(final_text(&turn).unwrap(), "完成了\n验证通过");
        let mut running = turn.clone();
        running["state"] = json!("running");
        assert!(final_text(&running).is_none());
        let mut no_reply = turn;
        no_reply["steps"][1]["frames"] = json!([]);
        assert!(final_text(&no_reply).is_none());
    }
}
