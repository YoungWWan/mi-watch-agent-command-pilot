use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const HEALTH_UA: &str = "accountsdk-18.8.15";

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct XiaomiQrSession {
    pub qr_url: String,
    pub qr_data_url: String,
    pub login_url: String,
    pub lp_url: String,
    pub timeout: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct XiaomiCredentials {
    pub user_id: String,
    pub c_user_id: Option<String>,
    pub service_token: String,
    pub pass_token: Option<String>,
    pub ssecurity: Option<String>,
    pub updated_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct XiaomiDevice {
    pub id: String,
    pub name: String,
    pub model: String,
    pub mac: String,
    pub fw_ver: Option<String>,
    pub has_authkey: bool,
    #[serde(default)]
    pub is_verified: bool,
    #[serde(default)]
    pub compatibility_note: String,
}

impl XiaomiDevice {
    fn refresh_identity(&mut self) {
        self.name = super::device_catalog::display_name(&self.model);
        self.is_verified = super::device_catalog::is_verified(&self.model);
        self.compatibility_note = super::device_catalog::compatibility_note(&self.model);
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeviceDetailRaw {
    pub mac: Option<String>,
    pub encrypt_key: Option<String>,
    pub fw_ver: Option<String>,
    pub sn: Option<String>,
    pub beaconkey: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DeviceItemRaw {
    pub sid: Option<String>,
    pub identifier: Option<String>,
    pub model: Option<String>,
    pub status: Option<i32>,
    pub detail: Option<DeviceDetailRaw>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SourceListResultRaw {
    pub data: Option<Vec<DeviceItemRaw>>,
    pub list: Option<Vec<DeviceItemRaw>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SourceListResponseRaw {
    pub code: i32,
    pub message: Option<String>,
    pub result: Option<SourceListResultRaw>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct QrPollResponse {
    pub status: String, // "waiting", "success", "timeout", "error"
    pub message: Option<String>,
    pub user_id: Option<String>,
    pub devices: Vec<XiaomiDevice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verification_id: Option<String>,
}

fn home_dir() -> PathBuf {
    #[cfg(unix)]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    }
    #[cfg(windows)]
    {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    }
}

fn session_path() -> PathBuf {
    let mut dir = home_dir();
    dir.push(".agent-command-pilot");
    let _ = fs::create_dir_all(&dir);
    dir.push("xiaomi_session.json");
    dir
}

fn cached_devices_path() -> PathBuf {
    let mut dir = home_dir();
    dir.push(".agent-command-pilot");
    let _ = fs::create_dir_all(&dir);
    dir.push("xiaomi_devices.json");
    dir
}

pub fn load_saved_credentials() -> Option<XiaomiCredentials> {
    let path = session_path();
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(creds) = serde_json::from_slice::<XiaomiCredentials>(&bytes) {
            if !creds.service_token.trim().is_empty() {
                return Some(creds);
            }
        }
    }
    None
}

pub fn save_credentials(creds: &XiaomiCredentials) -> Result<()> {
    let path = session_path();
    let json = serde_json::to_string_pretty(creds)?;
    fs::write(&path, json)?;
    Ok(())
}

pub fn clear_credentials() -> Result<()> {
    let path = session_path();
    if path.exists() {
        let _ = fs::remove_file(path);
    }
    let dev_path = cached_devices_path();
    if dev_path.exists() {
        let _ = fs::remove_file(dev_path);
    }
    Ok(())
}

pub fn load_cached_devices() -> Vec<XiaomiDevice> {
    let path = cached_devices_path();
    if let Ok(bytes) = fs::read(&path) {
        if let Ok(mut devs) = serde_json::from_slice::<Vec<XiaomiDevice>>(&bytes) {
            for device in &mut devs {
                device.refresh_identity();
            }
            return devs;
        }
    }
    Vec::new()
}

pub fn save_cached_devices(devs: &[XiaomiDevice]) -> Result<()> {
    let path = cached_devices_path();
    let json = serde_json::to_string_pretty(devs)?;
    fs::write(&path, json)?;
    Ok(())
}

fn login_client() -> Result<(reqwest::Client, std::sync::Arc<reqwest::cookie::Jar>)> {
    let jar = std::sync::Arc::new(reqwest::cookie::Jar::default());
    let device_id = format!("{:016x}", rand::random::<u64>());
    for domain in [
        "https://mi.com",
        "https://xiaomi.com",
        "https://account.xiaomi.com",
    ] {
        let url = reqwest::Url::parse(domain)?;
        jar.add_cookie_str("sdkVersion=accountsdk-18.8.15; Path=/", &url);
        jar.add_cookie_str(&format!("deviceId={device_id}; Path=/"), &url);
    }
    let client = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .cookie_provider(jar.clone())
        .redirect(reqwest::redirect::Policy::limited(10))
        .timeout(std::time::Duration::from_secs(35))
        .build()?;
    Ok((client, jar))
}

async fn exchange_service_token(
    client: &reqwest::Client,
    jar: &reqwest::cookie::Jar,
    location: &str,
) -> Result<String> {
    use reqwest::cookie::CookieStore;
    let url = reqwest::Url::parse(location).context("登录响应缺少有效 STS 回调地址")?;
    client
        .get(url)
        .send()
        .await
        .context("换取小米服务令牌失败")?
        .error_for_status()
        .context("小米 STS 回调失败")?;
    let sts = reqwest::Url::parse("https://sts-hlth.io.mi.com/healthapp/sts")?;
    let cookies = jar
        .cookies(&sts)
        .ok_or_else(|| anyhow!("STS 未返回服务 Cookie"))?;
    cookies
        .to_str()?
        .split(';')
        .find_map(|part| {
            part.trim()
                .strip_prefix("serviceToken=")
                .map(|v| v.trim_matches('"').to_string())
                .filter(|v| !v.is_empty())
        })
        .ok_or_else(|| anyhow!("STS 未返回 serviceToken，请重新登录"))
}

/// Request a new QR login session from Xiaomi Account
pub async fn request_qr_session() -> Result<XiaomiQrSession> {
    let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    let url = format!(
        "https://account.xiaomi.com/longPolling/loginUrl?_qrsize=480&qs=%3Fsid%3Dmiothealth%26_json%3Dtrue&callback=https%3A%2F%2Fsts-hlth.io.mi.com%2Fhealthapp%2Fsts&_hasLogo=false&sid=miothealth&serviceParam=&_locale=zh_CN&_dc={now_ms}"
    );

    let resp = client
        .get(&url)
        .send()
        .await
        .context("Failed to request QR login URL")?;
    let body = resp
        .text()
        .await
        .context("Failed to read QR login response body")?;

    let clean_json = body.trim_start_matches("&&&START&&&").trim();
    let val: serde_json::Value = serde_json::from_str(clean_json)
        .with_context(|| format!("Failed to parse QR login JSON: {clean_json}"))?;

    let qr_url = val["qr"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'qr' URL in response"))?
        .to_string();

    let login_url = val["loginUrl"].as_str().unwrap_or(&qr_url).to_string();

    let lp_url = val["lp"]
        .as_str()
        .ok_or_else(|| anyhow!("Missing 'lp' URL in response"))?
        .to_string();

    let timeout = val["timeout"].as_u64().unwrap_or(300);

    // Fetch the QR code image bytes and convert to base64 data URL
    let qr_bytes = if let Ok(img_resp) = client.get(&qr_url).send().await {
        img_resp
            .bytes()
            .await
            .map(|b| b.to_vec())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&qr_bytes);
    let qr_data_url = if b64.is_empty() {
        qr_url.clone()
    } else {
        format!("data:image/png;base64,{b64}")
    };

    Ok(XiaomiQrSession {
        qr_url,
        qr_data_url,
        login_url,
        lp_url,
        timeout,
    })
}

/// Poll the long polling URL to check if the user has confirmed QR login
pub async fn check_qr_login(lp_url: &str) -> Result<QrPollResponse> {
    let (client, jar) = login_client()?;

    let resp_res = client.get(lp_url).send().await;

    let resp = match resp_res {
        Ok(r) => r,
        Err(e) if e.is_timeout() => {
            return Ok(QrPollResponse {
                status: "waiting".to_string(),
                message: Some("等待扫码授权中...".to_string()),
                user_id: None,
                devices: Vec::new(),
                verification_id: None,
            });
        }
        Err(e) => {
            return Ok(QrPollResponse {
                status: "error".to_string(),
                message: Some(format!("轮询失败: {e:#}")),
                user_id: None,
                devices: Vec::new(),
                verification_id: None,
            });
        }
    };

    let status = resp.status();
    if !status.is_success() {
        return Ok(QrPollResponse {
            status: "waiting".to_string(),
            message: Some(format!("等待中 (HTTP {status})")),
            user_id: None,
            devices: Vec::new(),
            verification_id: None,
        });
    }

    let body = resp.text().await.unwrap_or_default();
    let clean = body.trim_start_matches("&&&START&&&").trim();

    let val: serde_json::Value = match serde_json::from_str(clean) {
        Ok(v) => v,
        Err(_) => {
            return Ok(QrPollResponse {
                status: "waiting".to_string(),
                message: Some("等待扫码...".to_string()),
                user_id: None,
                devices: Vec::new(),
                verification_id: None,
            });
        }
    };

    let user_id_val = &val["userId"];
    let user_id = if let Some(s) = user_id_val.as_str() {
        s.to_string()
    } else if let Some(n) = user_id_val.as_i64() {
        n.to_string()
    } else {
        return Ok(QrPollResponse {
            status: "waiting".to_string(),
            message: Some("已扫码，请在手机上点击确认授权...".to_string()),
            user_id: None,
            devices: Vec::new(),
            verification_id: None,
        });
    };

    let c_user_id = val["cUserId"].as_str().map(|s| s.to_string());
    let pass_token = val["passToken"].as_str().map(|s| s.to_string());
    let ssecurity = val["ssecurity"].as_str().map(|s| s.to_string());
    let location = val["location"].as_str().unwrap_or("");

    log::info!(
        "[Xiaomi Login] QR authorization confirmed for user_id={}",
        user_id
    );

    let service_token = exchange_service_token(&client, &jar, location).await?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let creds = XiaomiCredentials {
        user_id: user_id.clone(),
        c_user_id: c_user_id.clone(),
        service_token: service_token.clone(),
        pass_token,
        ssecurity,
        updated_at: now,
    };

    // Validate credentials immediately with Xiaomi Health cloud
    let devices = match fetch_source_devices(&creds).await {
        Ok(d) => {
            save_credentials(&creds).context("保存小米登录凭据失败")?;
            d
        }
        Err(e) => {
            log::warn!("[Xiaomi Login] Failed to fetch device list immediately: {e:#}");
            return Ok(QrPollResponse {
                status: "error".to_string(),
                message: Some(format!("获取账号绑定的穿戴设备失败: {e:#}，请重新扫码授权")),
                user_id: Some(user_id),
                devices: Vec::new(),
                verification_id: None,
            });
        }
    };

    Ok(QrPollResponse {
        status: "success".to_string(),
        message: Some(format!("登录成功 (账号: {})", user_id)),
        user_id: Some(user_id),
        devices,
        verification_id: None,
    })
}

/// Fetch list of bound wearable devices from https://hlth.io.mi.com/app/v1/source/get_source_list
pub async fn fetch_source_devices(creds: &XiaomiCredentials) -> Result<Vec<XiaomiDevice>> {
    if creds.service_token.trim().is_empty() {
        return Err(anyhow!("serviceToken 为空，未能获取有效云端授权"));
    }
    let ssecurity = creds
        .ssecurity
        .as_deref()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("登录凭据缺少 ssecurity，请重新登录"))?;
    let c_user_id = creds
        .c_user_id
        .as_deref()
        .filter(|v| !v.is_empty())
        .ok_or_else(|| anyhow!("登录凭据缺少 cUserId，请重新登录"))?;
    let params = std::collections::HashMap::from([(
        "data".to_string(),
        r#"{"page_size":50,"status":1}"#.to_string(),
    )]);
    let body = super::xiaomi_crypto::mi_service_call_encrypted(
        super::xiaomi_crypto::MiAccountToken {
            ssecurity: ssecurity.to_string(),
            service_token: creds.service_token.clone(),
            c_user_id: c_user_id.to_string(),
        },
        String::new(),
        "https://hlth.io.mi.com/app/v1/source/get_source_list".to_string(),
        params,
        HEALTH_UA.to_string(),
    )
    .await
    .context("查询小米运动健康设备失败")?;
    let raw_resp: SourceListResponseRaw =
        serde_json::from_str(&body).context("解析小米运动健康设备响应失败")?;

    if raw_resp.code != 0 {
        return Err(anyhow!(
            "Cloud API returned error code {}: {}",
            raw_resp.code,
            raw_resp.message.unwrap_or_default()
        ));
    }

    let items = raw_resp
        .result
        .and_then(|r| r.data.or(r.list))
        .unwrap_or_default();

    log::info!(
        "[Xiaomi Cloud] Received {} bound source entries from cloud",
        items.len()
    );

    let mut result_devices = Vec::new();

    for item in items {
        if let Some(detail) = item.detail {
            if let Some(raw_mac) = detail.mac {
                let mac = raw_mac.trim().to_uppercase();
                let encrypt_key = detail.encrypt_key.unwrap_or_default().trim().to_lowercase();
                let model = item.model.unwrap_or_else(|| "Wearable Device".to_string());
                let identifier = item.identifier.unwrap_or_else(|| mac.clone());
                let fw_ver = detail.fw_ver;

                let name = super::device_catalog::display_name(&model);
                let mut has_authkey = !encrypt_key.is_empty();
                let is_verified = super::device_catalog::is_verified(&model);
                let compatibility_note = super::device_catalog::compatibility_note(&model);

                // Save device credentials in the private local store.
                if has_authkey {
                    if let Err(e) = crate::storage::device_keys::save_authkey(&mac, &encrypt_key) {
                        has_authkey = false;
                        log::warn!("[本地凭据存储] Failed to cache authkey for {mac}: {e:#}");
                    } else {
                        log::info!("[本地凭据存储] Auto-saved cloud AuthKey for {name} ({mac})");
                    }
                }

                result_devices.push(XiaomiDevice {
                    id: identifier,
                    name,
                    model,
                    mac,
                    fw_ver,
                    has_authkey,
                    is_verified,
                    compatibility_note,
                });
            }
        }
    }

    // Cache the devices locally
    let _ = save_cached_devices(&result_devices);

    Ok(result_devices)
}

/// Try to import Xiaomi credentials from local AstroBox installation if present
pub async fn try_import_astrobox() -> Result<Vec<XiaomiDevice>> {
    let mut astrobox_path = home_dir();
    astrobox_path.push(
        "Library/Application Support/moe.astralsight.astrobox/accounts/xiaomi_credentials_v1.json",
    );

    if !astrobox_path.exists() {
        return Err(anyhow!("未发现本地已安装的 AstroBox 账号凭据文件"));
    }

    let bytes = fs::read(&astrobox_path)
        .with_context(|| format!("读取 AstroBox 凭据文件失败: {:?}", astrobox_path))?;

    let val: serde_json::Value =
        serde_json::from_slice(&bytes).context("解析 AstroBox 凭据 JSON 失败")?;

    let user_id = val["user_id"]
        .as_str()
        .ok_or_else(|| anyhow!("凭据中缺少 user_id"))?
        .to_string();

    let service_token = val["service_token"]
        .as_str()
        .ok_or_else(|| anyhow!("凭据中缺少 service_token"))?
        .to_string();

    let c_user_id = val["c_user_id"].as_str().map(|s| s.to_string());
    let pass_token = val["pass_token"].as_str().map(|s| s.to_string());
    let ssecurity = val["ssecurity"].as_str().map(|s| s.to_string());

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let creds = XiaomiCredentials {
        user_id: user_id.clone(),
        c_user_id,
        service_token,
        pass_token,
        ssecurity,
        updated_at: now,
    };

    // Attempt to fetch devices using imported credentials
    match fetch_source_devices(&creds).await {
        Ok(devs) => {
            save_credentials(&creds).context("保存小米登录凭据失败")?;
            log::info!(
                "[AstroBox Import] Successfully imported credentials and fetched {} devices",
                devs.len()
            );
            Ok(devs)
        }
        Err(e) => {
            log::warn!("[AstroBox Import] Credential validation failed: {e:#}");
            Err(anyhow!(
                "AstroBox 凭证验证失败，请检查网络或重新登录: {e:#}"
            ))
        }
    }
}

/// Login with Xiaomi account username (email/phone/id) and password
pub async fn login_with_password(
    app: &tauri::AppHandle,
    username: &str,
    password: &str,
) -> Result<QrPollResponse> {
    use md5::{Digest, Md5};
    let mut hasher = Md5::new();
    hasher.update(password.as_bytes());
    let hash_hex = format!("{:X}", hasher.finalize());

    let (client, jar) = login_client()?;
    let step1 = client
        .get("https://account.xiaomi.com/pass/serviceLogin?sid=miothealth&_json=true")
        .send()
        .await
        .context("获取小米登录签名失败")?
        .error_for_status()?;
    let body = step1.text().await?;
    let prelogin: serde_json::Value =
        serde_json::from_str(body.trim().trim_start_matches("&&&START&&&"))
            .context("解析小米登录签名失败")?;
    let sign = prelogin["_sign"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("小米未返回有效 _sign，请稍后重试"))?;

    let session = PasswordSession {
        client,
        jar,
        username: username.to_string(),
        hash: hash_hex,
        sign: sign.to_string(),
    };
    let val = submit_password(&session).await?;
    finish_password_login(app, session, val).await
}

struct PasswordSession {
    client: reqwest::Client,
    jar: std::sync::Arc<reqwest::cookie::Jar>,
    username: String,
    hash: String,
    sign: String,
}

async fn submit_password(session: &PasswordSession) -> Result<serde_json::Value> {
    let params = [
        ("sid", "miothealth"),
        ("_json", "true"),
        ("user", session.username.as_str()),
        ("hash", session.hash.as_str()),
        ("callback", "https://sts-hlth.io.mi.com/healthapp/sts"),
        ("qs", "%3Fsid%3Dmiothealth%26_json%3Dtrue"),
        ("_sign", session.sign.as_str()),
    ];
    let text = session
        .client
        .post("https://account.xiaomi.com/pass/serviceLoginAuth2")
        .form(&params)
        .send()
        .await
        .context("发送账号密码登录请求失败")?
        .error_for_status()?
        .text()
        .await?;
    serde_json::from_str(text.trim().trim_start_matches("&&&START&&&")).context("解析登录响应失败")
}

async fn finish_password_login(
    app: &tauri::AppHandle,
    session: PasswordSession,
    val: serde_json::Value,
) -> Result<QrPollResponse> {
    if let Some(url) = verification_url(&val)? {
        return begin_verification(app, session, url).await;
    }
    let PasswordSession {
        client,
        jar,
        username,
        ..
    } = session;
    let code = val["code"].as_i64().unwrap_or(-1);
    if code == 0 {
        let location = val["location"].as_str().unwrap_or_default().to_string();
        let user_id = if let Some(s) = val["userId"].as_str() {
            s.to_string()
        } else if let Some(n) = val["userId"].as_i64() {
            n.to_string()
        } else {
            username.to_string()
        };
        let c_user_id = val["cUserId"].as_str().map(|s| s.to_string());
        let pass_token = val["passToken"].as_str().map(|s| s.to_string());
        let ssecurity = val["ssecurity"].as_str().map(|s| s.to_string());

        let service_token = exchange_service_token(&client, &jar, &location).await?;

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let creds = XiaomiCredentials {
            user_id: user_id.clone(),
            c_user_id,
            service_token,
            pass_token,
            ssecurity,
            updated_at: now,
        };

        // Validate immediately with Xiaomi Health cloud before declaring success
        let devices = match fetch_source_devices(&creds).await {
            Ok(d) => {
                save_credentials(&creds).context("保存小米登录凭据失败")?;
                d
            }
            Err(e) => {
                log::warn!("[Xiaomi Login] Cloud validation failed: {e:#}");
                return Ok(QrPollResponse {
                    status: "error".to_string(),
                    message: Some(format!(
                        "云端鉴权未通过: {e:#}。建议切换至「扫码快捷登录」重新授权。"
                    )),
                    user_id: None,
                    devices: Vec::new(),
                    verification_id: None,
                });
            }
        };

        Ok(QrPollResponse {
            status: "success".to_string(),
            message: Some(format!("登录成功 (账号: {user_id})")),
            user_id: Some(user_id),
            devices,
            verification_id: None,
        })
    } else {
        let desc = val["description"]
            .as_str()
            .or_else(|| val["desc"].as_str())
            .unwrap_or("登录失败");

        let error_msg = if code == 70016 {
            "用户名或密码错误，请检查输入是否正确。若多次尝试失败，建议使用「扫码登录」以免除风控限制。".to_string()
        } else {
            format!("{desc} (错误代码: {code})")
        };

        Ok(QrPollResponse {
            status: "error".to_string(),
            message: Some(error_msg),
            user_id: None,
            devices: Vec::new(),
            verification_id: None,
        })
    }
}

const ACCOUNT_LOGIN: &str =
    "https://account.xiaomi.com/pass/serviceLogin?sid=miothealth&_json=true";

fn trusted_xiaomi_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| {
            host == "xiaomi.com"
                || host.ends_with(".xiaomi.com")
                || host == "mi.com"
                || host.ends_with(".mi.com")
        })
}

fn verification_url(val: &serde_json::Value) -> Result<Option<reqwest::Url>> {
    if let Some(raw) = val["notificationUrl"].as_str().filter(|v| !v.is_empty()) {
        let url = reqwest::Url::parse("https://account.xiaomi.com")?.join(raw)?;
        if !trusted_xiaomi_url(&url) {
            return Err(anyhow!("小米返回的验证地址不受信任"));
        }
        return Ok(Some(url));
    }
    if val["captchaUrl"].as_str().is_some_and(|v| !v.is_empty())
        || val["code"].as_i64() == Some(87001)
    {
        // captchaUrl can be an image, not an interactive challenge. Use Xiaomi's full login UI.
        return Ok(Some(reqwest::Url::parse(
            "https://account.xiaomi.com/pass/serviceLogin?sid=miothealth",
        )?));
    }
    Ok(None)
}

struct PendingVerification {
    id: String,
    session: PasswordSession,
    window: tauri::WebviewWindow,
    created: std::time::Instant,
    close_requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

static PENDING_VERIFICATION: std::sync::Mutex<Option<PendingVerification>> =
    std::sync::Mutex::new(None);

pub fn cancel_verification(id: &str) -> Result<()> {
    let pending = {
        let mut slot = PENDING_VERIFICATION
            .lock()
            .map_err(|_| anyhow!("验证状态不可用"))?;
        if slot.as_ref().is_some_and(|p| p.id == id) {
            slot.take()
        } else {
            None
        }
    };
    if let Some(pending) = pending {
        let _ = pending.window.destroy();
    }
    Ok(())
}

async fn begin_verification(
    app: &tauri::AppHandle,
    session: PasswordSession,
    url: reqwest::Url,
) -> Result<QrPollResponse> {
    use reqwest::cookie::CookieStore;
    let id = format!("xiaomi-verify-{:016x}", rand::random::<u64>());
    let window = tauri::WebviewWindowBuilder::new(
        app,
        &id,
        tauri::WebviewUrl::External("about:blank".parse()?),
    )
    .title("小米账号官方安全验证 · 验证后自动登录")
    .inner_size(520.0, 720.0)
    .incognito(true)
    .user_agent(USER_AGENT)
    .on_navigation(|url| url.as_str() == "about:blank" || trusted_xiaomi_url(url))
    .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
    .build()
    .context("打开小米安全验证窗口失败")?;
    // Only seed this account session, never the user's other browser cookies.
    let setup = (|| -> Result<()> {
        let account = reqwest::Url::parse("https://account.xiaomi.com")?;
        if let Some(cookies) = session.jar.cookies(&account) {
            for pair in cookies.to_str()?.split(';') {
                if let Some((name, value)) = pair.trim().split_once('=') {
                    let cookie =
                        tauri::webview::Cookie::build((name.to_string(), value.to_string()))
                            .domain("account.xiaomi.com")
                            .path("/")
                            .secure(true)
                            .build();
                    window.set_cookie(cookie)?;
                }
            }
        }
        window.navigate(url)?;
        Ok(())
    })();
    if let Err(e) = setup {
        let _ = window.close();
        return Err(e);
    }
    let close_requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let old = PENDING_VERIFICATION
        .lock()
        .map_err(|_| anyhow!("验证状态不可用"))?
        .replace(PendingVerification {
            id: id.clone(),
            session,
            window: window.clone(),
            created: std::time::Instant::now(),
            close_requested: close_requested.clone(),
        });
    if let Some(old) = old {
        let _ = old.window.destroy();
    }
    let closed_id = id.clone();
    let close_window = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            let active = PENDING_VERIFICATION
                .lock()
                .ok()
                .is_some_and(|slot| slot.as_ref().is_some_and(|p| p.id == closed_id));
            if active {
                // Preserve the WebKit cookie store until the async completion check has read it.
                api.prevent_close();
                close_requested.store(true, std::sync::atomic::Ordering::Release);
                let _ = close_window.hide();
            }
        }
    });
    let timeout_id = id.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(600)).await;
        let _ = cancel_verification(&timeout_id);
    });
    Ok(QrPollResponse {
        status: "verification_required".to_string(),
        message: Some(
            "请在小米官方窗口完成验证，助手将自动登录并同步设备，无需手动确认。".to_string(),
        ),
        user_id: None,
        devices: Vec::new(),
        verification_id: Some(id),
    })
}

fn import_verification_cookies(
    jar: &reqwest::cookie::Jar,
    cookies: Vec<tauri::webview::Cookie<'static>>,
) -> Result<()> {
    for cookie in cookies {
        let Some(domain) = cookie.domain() else {
            continue;
        };
        let origin = reqwest::Url::parse(&format!("https://{}", domain.trim_start_matches('.')))?;
        if trusted_xiaomi_url(&origin) {
            jar.add_cookie_str(&cookie.to_string(), &origin);
        }
    }
    Ok(())
}

// Read only a completion signal, never the verification code or other page contents.
const VERIFICATION_COMPLETE_SCRIPT: &str = include_str!("xiaomi_verification_complete.js");

async fn verification_page_complete(window: &tauri::WebviewWindow) -> Result<bool> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let sender = std::sync::Mutex::new(Some(sender));
    window.eval_with_callback(VERIFICATION_COMPLETE_SCRIPT, move |value| {
        if let Some(sender) = sender.lock().ok().and_then(|mut s| s.take()) {
            let _ = sender.send(value);
        }
    })?;
    match tokio::time::timeout(std::time::Duration::from_secs(3), receiver).await {
        Ok(Ok(value)) => Ok(serde_json::from_str::<bool>(&value).unwrap_or(false)),
        _ => Ok(false), // Navigation can interrupt an evaluation; retry on the next tick.
    }
}

async fn wait_for_verification(id: &str) -> Result<Vec<tauri::webview::Cookie<'static>>> {
    loop {
        let (window, closing, expired) = {
            let slot = PENDING_VERIFICATION
                .lock()
                .map_err(|_| anyhow!("验证状态不可用"))?;
            let pending = slot
                .as_ref()
                .filter(|p| p.id == id)
                .ok_or_else(|| anyhow!("验证已取消或超时，请重新登录"))?;
            (
                pending.window.clone(),
                pending.close_requested.clone(),
                pending.created.elapsed().as_secs() >= 600,
            )
        };
        if expired {
            cancel_verification(id)?;
            return Err(anyhow!("验证已超时，请重新登录"));
        }
        let ready = verification_page_complete(&window).await?;
        let read_window = window.clone();
        let cookies = tauri::async_runtime::spawn_blocking(move || read_window.cookies()).await??;
        let has_service_token = cookies.iter().any(|c| {
            c.name() == "serviceToken"
                && !c.value().is_empty()
                && c.domain()
                    .is_some_and(|d| d.trim_start_matches('.') == "sts-hlth.io.mi.com")
        });
        // The 'ok' page is only a signal to resume. The cloud still validates the login.
        if ready || has_service_token {
            return Ok(cookies);
        }
        if closing.load(std::sync::atomic::Ordering::Acquire) {
            // One final read after the close event catches a just-completed navigation.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if verification_page_complete(&window).await? {
                return Ok(tauri::async_runtime::spawn_blocking(move || window.cookies()).await??);
            }
            cancel_verification(id)?;
            return Err(anyhow!("已取消小米安全验证，可重新登录"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    }
}

pub async fn complete_verification(app: &tauri::AppHandle, id: &str) -> Result<QrPollResponse> {
    let cookies = match wait_for_verification(id).await {
        Ok(cookies) => cookies,
        Err(error) => {
            let _ = cancel_verification(id);
            return Err(error);
        }
    };
    let pending = {
        let mut slot = PENDING_VERIFICATION
            .lock()
            .map_err(|_| anyhow!("验证状态不可用"))?;
        if !slot.as_ref().is_some_and(|p| p.id == id) {
            return Err(anyhow!("验证窗口已关闭或会话已失效，请重新登录"));
        }
        slot.take().unwrap()
    };
    let window = pending.window.clone();
    let result = async {
        if pending.created.elapsed() > std::time::Duration::from_secs(600) {
            return Err(anyhow!("验证已超时，请重新登录"));
        }
        let mut session = pending.session;
        import_verification_cookies(&session.jar, cookies)?;
        // Resume through serviceLogin first: a completed official login may already have passToken.
        let text = session
            .client
            .get(ACCOUNT_LOGIN)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        let val: serde_json::Value =
            serde_json::from_str(text.trim().trim_start_matches("&&&START&&&"))
                .context("解析验证后的登录响应失败")?;
        if val["code"].as_i64() == Some(0)
            && val["ssecurity"].as_str().is_some_and(|s| !s.is_empty())
        {
            return finish_password_login(app, session, val).await;
        }
        if let Some(sign) = val["_sign"].as_str().filter(|s| !s.is_empty()) {
            session.sign = sign.to_string();
        }
        let val = submit_password(&session).await?;
        finish_password_login(app, session, val).await
    }
    .await;
    let _ = window.destroy();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_cached_devices_rederive_names_and_validation_from_model() {
        let mut device: XiaomiDevice = serde_json::from_value(serde_json::json!({
            "id": "test-device",
            "name": "Redmi Watch 5",
            "model": "lchz.watch.n65",
            "mac": "00:11:22:33:44:55",
            "has_authkey": true,
            "is_supported": true
        })).unwrap();
        device.refresh_identity();
        assert_eq!(device.name, "Redmi Watch 4");
        assert!(!device.is_verified);
        assert!(device.has_authkey);
        assert!(device.compatibility_note.contains("HTTP"));
    }

    #[test]
    fn routes_official_notification_even_without_ssecurity() {
        let val =
            serde_json::json!({"code": 0, "notificationUrl": "/identity/authStart?ticket=test"});
        let url = verification_url(&val).unwrap().unwrap();
        assert_eq!(
            url.as_str(),
            "https://account.xiaomi.com/identity/authStart?ticket=test"
        );
        assert!(verification_url(&serde_json::json!({"code": 70016}))
            .unwrap()
            .is_none());
        assert!(verification_url(
            &serde_json::json!({"code": 0, "notificationUrl": "", "captchaUrl": ""})
        )
        .unwrap()
        .is_none());
    }

    #[test]
    fn captcha_opens_interactive_login_instead_of_image() {
        for val in [
            serde_json::json!({"captchaUrl": "/pass/getCode?icodeType=antispam"}),
            serde_json::json!({"code": 87001}),
        ] {
            assert_eq!(
                verification_url(&val).unwrap().unwrap().as_str(),
                "https://account.xiaomi.com/pass/serviceLogin?sid=miothealth"
            );
        }
    }

    #[test]
    fn rejects_untrusted_verification_navigation() {
        for url in [
            "http://account.xiaomi.com",
            "https://xiaomi.com.evil.test",
            "https://evilxiaomi.com",
            "https://account.xiaomi.com@evil.test",
            "https://account.xiaomi.com:444",
            "javascript:alert(1)",
            "file:///tmp/test",
        ] {
            assert!(
                !trusted_xiaomi_url(&reqwest::Url::parse(url).unwrap()),
                "{url}"
            );
            assert!(
                verification_url(&serde_json::json!({"notificationUrl": url})).is_err(),
                "{url}"
            );
        }
    }

    #[test]
    fn verification_cookies_keep_scope_and_http_only_values() {
        use reqwest::cookie::CookieStore;
        let jar = reqwest::cookie::Jar::default();
        let cookies = vec![
            tauri::webview::Cookie::parse(
                "passToken=test-token; Domain=.xiaomi.com; Path=/; Secure; HttpOnly",
            )
            .unwrap()
            .into_owned(),
            tauri::webview::Cookie::parse("unrelated=secret; Domain=example.com; Path=/")
                .unwrap()
                .into_owned(),
        ];
        import_verification_cookies(&jar, cookies).unwrap();
        let account = reqwest::Url::parse("https://account.xiaomi.com/pass/serviceLogin").unwrap();
        assert_eq!(
            jar.cookies(&account).unwrap().to_str().unwrap(),
            "passToken=test-token"
        );
        assert!(jar
            .cookies(&reqwest::Url::parse("https://example.com").unwrap())
            .is_none());
        assert!(jar
            .cookies(&reqwest::Url::parse("https://sts-hlth.io.mi.com").unwrap())
            .is_none());
    }

    #[test]
    fn test_parse_source_list_response() {
        let json = r#"{
            "code": 0,
            "message": "ok",
            "result": {
                "data": [
                    {
                        "sid": "wearapp",
                        "identifier": "1867EF4E3F72",
                        "model": "Redmi Watch 5",
                        "status": 1,
                        "detail": {
                            "mac": "18:67:EF:4E:3F:72",
                            "encrypt_key": "a1b2c3d4e5f60718293a4b5c6d7e8f90",
                            "fw_ver": "1.1.20",
                            "sn": "51234/A0001"
                        }
                    }
                ]
            }
        }"#;

        let parsed: SourceListResponseRaw =
            serde_json::from_str(json).expect("Must parse response");
        assert_eq!(parsed.code, 0);
        let items = parsed.result.unwrap().data.unwrap();
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.model.as_deref(), Some("Redmi Watch 5"));
        let detail = item.detail.as_ref().unwrap();
        assert_eq!(detail.mac.as_deref(), Some("18:67:EF:4E:3F:72"));
        assert_eq!(
            detail.encrypt_key.as_deref(),
            Some("a1b2c3d4e5f60718293a4b5c6d7e8f90")
        );
    }
}
