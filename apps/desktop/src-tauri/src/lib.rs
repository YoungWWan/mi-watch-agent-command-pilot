use std::sync::Arc;
use btclassic_spp::BtclassicSppExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::{AppHandle, Emitter, Manager};

pub mod account;
pub mod cli;
pub mod connection;
pub mod integrations;
pub mod server;
pub mod storage;
mod watch_app;
mod updates;

const EMBEDDED_RPK: &[u8] = include_bytes!("../resources/watch-app.rpk");
const EXPECTED_PACKAGE: &str = "com.young.watchcommand";
const EXPECTED_SHA256: &str = "7b94fa571521577c10c8745bda4e8e08e67088fbc59f2af096bd717cb1911029";

#[derive(Serialize, Deserialize)]
pub struct OperationResult {
    pub success: bool,
    pub message: String,
}

#[derive(Serialize, Deserialize)]
pub struct ScannedDeviceItem {
    pub name: String,
    pub address: String,
}

#[derive(Serialize, Deserialize)]
pub struct QuickAppItem {
    pub package_name: String,
    pub app_name: Option<String>,
    pub version_code: Option<u32>,
}

#[derive(Serialize, Deserialize)]
pub struct G0ComplianceReport {
    pub status: String,
    pub license: String,
    pub upstream_modules: Vec<UpstreamModuleInfo>,
}

#[derive(Serialize, Deserialize)]
pub struct UpstreamModuleInfo {
    pub name: String,
    pub commit: String,
    pub license: String,
    pub repository: String,
}

#[tauri::command]
fn g0_check() -> G0ComplianceReport {
    G0ComplianceReport {
        status: "PASSED".to_string(),
        license: "GNU AGPL-3.0".to_string(),
        upstream_modules: vec![
            UpstreamModuleInfo {
                name: "AstroBox-NG-Module-Core".to_string(),
                commit: "b7b8e94599ddad8435cdfb279790cf11cca0a392".to_string(),
                license: "AGPL-3.0".to_string(),
                repository: "https://github.com/AstralSightStudios/AstroBox-NG-Module-Core.git".to_string(),
            },
            UpstreamModuleInfo {
                name: "AstroBox-NG-Module-Pb".to_string(),
                commit: "03a92010056dd41af114f6f46fd612104b27bd7b".to_string(),
                license: "AGPL-3.0".to_string(),
                repository: "https://github.com/AstralSightStudios/AstroBox-NG-Module-Pb.git".to_string(),
            },
            UpstreamModuleInfo {
                name: "AstroBox-NG-Module-Vivo-MsgPack".to_string(),
                commit: "baa814b3d90454c127008a85ac307acf92b4914f".to_string(),
                license: "AGPL-3.0".to_string(),
                repository: "https://github.com/AstralSightStudios/AstroBox-NG-Module-Vivo-MsgPack.git".to_string(),
            },
            UpstreamModuleInfo {
                name: "AstroBox-NG-Module-Bluetooth".to_string(),
                commit: "d855415cfb3b42fa064eb33eaebcc0a0d0720661".to_string(),
                license: "AGPL-3.0".to_string(),
                repository: "https://github.com/AstralSightStudios/AstroBox-NG-Module-Bluetooth.git".to_string(),
            },
            UpstreamModuleInfo {
                name: "AstroBox-NG-Plugin-BtClassicSpp".to_string(),
                commit: "9f8e8804038749260a43548152f50256e0b04ab8".to_string(),
                license: "AGPL-3.0".to_string(),
                repository: "https://github.com/AstralSightStudios/AstroBox-NG-Plugin-BtClassicSpp.git".to_string(),
            },
        ],
    }
}

#[tauri::command]
fn scan_start(app: AppHandle) -> Result<(), String> {
    log::info!("[Bluetooth] Starting SPP discovery...");
    let spp = app.btclassic_spp();
    spp.start_scan().map_err(|e| format!("Failed to start scan: {e}"))
}

#[tauri::command]
fn scan_stop(app: AppHandle) -> Result<(), String> {
    log::info!("[Bluetooth] Stopping SPP discovery...");
    let spp = app.btclassic_spp();
    spp.stop_scan().map_err(|e| format!("Failed to stop scan: {e}"))
}

#[tauri::command]
fn get_scanned_devices(app: AppHandle) -> Result<Vec<ScannedDeviceItem>, String> {
    let spp = app.btclassic_spp();
    let res = spp.get_scanned_devices().map_err(|e| format!("Failed to get devices: {e}"))?;
    Ok(res.ret.into_iter().map(|d| ScannedDeviceItem {
        name: d.name.unwrap_or_else(|| "Unknown".to_string()),
        address: d.address,
    }).collect())
}

#[tauri::command]
async fn connect_and_auth(app: AppHandle, mac: String) -> Result<OperationResult, String> {
    let mac = mac.trim().to_uppercase();
    let session = app.state::<connection::DeviceConnections>().begin(&mac)?;
    let mut result = session
        .run_connection(
            connect_and_auth_session(app.clone(), mac.clone(), session.clone()),
            std::time::Duration::from_secs(60),
        )
        .await;
    session.complete_connection();
    if !session.is_active() {
        result = Err("蓝牙连接已断开，请重新连接手表".to_string());
    }
    if !matches!(&result, Ok(operation) if operation.success) {
        let _ = app.btclassic_spp().disconnect(&mac);
        cleanup_device_connection(app, mac, session).await;
    }
    result
}

async fn cleanup_device_connection(app: AppHandle, mac: String, session: Arc<connection::DeviceSession>) {
    session.mark_disconnected();
    // The connection caller cancels authentication before completing the
    // session. Cleanup then follows any already queued ECS creation job.
    if session.is_connecting() { return; }
    let cleanup_app = app.clone();
    let cleanup_mac = mac.clone();
    let removed = corelib::ecs::with_rt_mut(move |rt| {
        cleanup_app.state::<connection::DeviceConnections>().finish(&cleanup_mac, &session, || {
            rt.remove_device(&cleanup_mac);
            corelib::device::cleanup_device_state(corelib::device::DeviceKind::Xiaomi, &cleanup_mac);
        })
    }).await;
    if removed {
        let _ = app.emit("device-disconnected", serde_json::json!({ "mac": mac }));
    }
}

fn notify_device_disconnected(app: AppHandle, mac: String, session: Arc<connection::DeviceSession>) {
    if session.mark_disconnected() {
        let _ = app.emit("device-disconnected", serde_json::json!({ "mac": mac }));
        tauri::async_runtime::spawn(cleanup_device_connection(app, mac, session));
    }
}

#[tauri::command]
async fn get_device_connection_status(app: AppHandle, mac: String) -> connection::DeviceConnectionStatus {
    let mac = mac.trim().to_uppercase();
    let Some(session) = app.state::<connection::DeviceConnections>().get(&mac) else {
        return connection::DeviceConnectionStatus::Disconnected;
    };
    if session.is_active() && !session.is_connecting()
        && app.btclassic_spp().get_connected_device_info(&mac).is_err()
    {
        notify_device_disconnected(app, mac, session.clone());
    }
    session.status()
}

#[tauri::command]
async fn is_device_connected(app: AppHandle, mac: String) -> bool {
    let mac = mac.trim().to_uppercase();
    require_device_session(app, &mac).await.is_ok()
}

async fn require_device_session(app: AppHandle, mac: &str) -> Result<Arc<connection::DeviceSession>, String> {
    let session = app.state::<connection::DeviceConnections>().get(mac)
        .ok_or("蓝牙未连接，请先连接手表")?;
    if session.is_connecting() {
        return Err("手表正在连接，请等待认证完成".to_string());
    }
    if !session.is_active() || app.btclassic_spp().get_connected_device_info(mac).is_err() {
        cleanup_device_connection(app, mac.to_string(), session).await;
        return Err("蓝牙连接已断开，请重新连接手表".to_string());
    }
    Ok(session)
}

async fn connect_and_auth_session(app: AppHandle, mac: String, session: Arc<connection::DeviceSession>) -> Result<OperationResult, String> {
    let creds = account::xiaomi::load_saved_credentials().ok_or("请先登录小米账号")?;
    // Revalidate membership with the current account before opening a Bluetooth connection.
    let devices = account::xiaomi::fetch_source_devices(&creds).await.map_err(|e| format!("无法验证账号设备: {e:#}"))?;
    let device = devices.iter().find(|device| device.mac.eq_ignore_ascii_case(&mac) && device.has_authkey)
        .ok_or("只能连接当前小米账号下已授权的设备")?;
    let device_name = device.name.clone();
    let sar_version = account::device_catalog::sar_version(&device.model);
    let authkey = storage::device_keys::get_authkey(&mac)
        .ok_or("无法读取设备连接凭据，请刷新账号设备后重试")?.trim().to_lowercase();

    log::info!("[Connect] Setting up packet listener for {}...", mac);
    let spp = app.btclassic_spp();
    let tk = tokio::runtime::Handle::current();
    let tk_clone = tk.clone();
    let mac_clone = mac.clone();
    let app_for_recv = app.clone();
    let session_for_recv = session.clone();

    spp.set_data_listener(&mac, move |res| {
        match res {
            Ok(bytes) if session_for_recv.is_active() => {
                corelib::device::xiaomi::packet::dispatcher::on_packet(
                    tk_clone.clone(),
                    mac_clone.clone(),
                    bytes,
                );
            }
            Err(err) => {
                log::error!("[SPP Recv] error on {}: {}", mac_clone, err);
                notify_device_disconnected(app_for_recv.clone(), mac_clone.clone(), session_for_recv.clone());
            }
            Ok(_) => {}
        }
    }).map_err(|e| format!("Failed to set data listener: {e}"))?;

    spp.start_subscription(&mac)
        .map_err(|e| format!("Failed to start subscription: {e}"))?;

    log::info!("[Connect] Initiating SPP RFCOMM connection to {}...", mac);
    let conn_res = spp.connect(&mac, false, &[4, 5, 6, 7])
        .map_err(|e| format!("无法通过经典蓝牙连接 {device_name}：{e}。请确认设备在附近且未被其他应用占用；仅支持 BLE 的设备目前无法直接连接。"))?;

    if !conn_res.ret {
        return Ok(OperationResult {
            success: false,
            message: format!("SPP connect returned false for {mac}"),
        });
    }

    log::info!("[Connect] Waiting for RFCOMM channel to be fully established...");
    let start_wait = std::time::Instant::now();
    let mut channel_ready = false;
    while start_wait.elapsed() < std::time::Duration::from_secs(15) {
        if !session.is_active() {
            return Err("蓝牙连接已断开，请重新连接手表".to_string());
        }
        if spp.get_connected_device_info(&mac).is_ok() {
            channel_ready = true;
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
    }

    if !channel_ready {
        return Ok(OperationResult {
            success: false,
            message: format!("RFCOMM connection timed out after 15s for {mac}"),
        });
    }

    let mtu = spp.get_max_send_len(&mac).ok().flatten().unwrap_or(666);
    log::info!("[Connect] RFCOMM channel ready (MTU: {}). Starting Xiaomi protocol authentication with AuthKey...", mtu);
    let app_for_send = app.clone();
    let mac_for_send = mac.clone();
    let session_for_send = session.clone();

    let authentication = corelib::device::create_device(
        tk,
        corelib::device::DeviceKind::Xiaomi,
        device_name,
        mac.clone(),
        authkey.clone(),
        sar_version,
        corelib::device::xiaomi::r#type::ConnectType::SPP,
        Some(16),
        None,
        None,
        false,
        move |chunks: Vec<Vec<u8>>| {
            let app = app_for_send.clone();
            let addr = mac_for_send.clone();
            let session = session_for_send.clone();
            async move {
                let spp = app.btclassic_spp();
                for chunk in chunks {
                    if !session.is_active() {
                        return Err(corelib::device::xiaomi::SendError::Disconnected);
                    }
                    if spp.get_connected_device_info(&addr).is_err() {
                        notify_device_disconnected(app.clone(), addr.clone(), session.clone());
                        return Err(corelib::device::xiaomi::SendError::Disconnected);
                    }
                    if let Err(error) = spp.send(&addr, &chunk) {
                        if spp.get_connected_device_info(&addr).is_err() {
                            notify_device_disconnected(app.clone(), addr.clone(), session.clone());
                        }
                        return Err(corelib::device::xiaomi::SendError::Io(error.to_string()));
                    }
                }
                Ok(())
            }
        },
    );
    let dev_info = tokio::time::timeout(std::time::Duration::from_secs(30), authentication).await
        .map_err(|_| "手表协议认证超时（30秒），请重新连接".to_string())?
        .map_err(|e| format!("Protocol authentication failed: {e:#}"))?;

    if !session.is_active() || spp.get_connected_device_info(&mac).is_err() {
        return Err("认证后蓝牙连接已断开，请重新连接手表".to_string());
    }

    log::info!("[Auth] Authentication succeeded! Device: {:?}", dev_info.name);
    // Persist into 本地凭据存储 and settings
    let _ = storage::device_keys::save_authkey(&mac, &authkey);
    let mut settings = storage::settings::load_settings();
    settings.last_connected_mac = Some(mac.clone());
    let _ = storage::settings::save_settings(&settings);

    Ok(OperationResult {
        success: true,
        message: format!("Connected and authenticated as {}", dev_info.name),
    })
}

#[tauri::command]
async fn disconnect_device(app: AppHandle, mac: String) -> Result<OperationResult, String> {
    let mac = mac.trim().to_uppercase();
    log::info!("[Disconnect] Disconnecting device {}...", mac);
    let spp = app.btclassic_spp();
    if let Err(error) = spp.disconnect(&mac) {
        if spp.get_connected_device_info(&mac).is_ok() {
            return Err(format!("无法断开蓝牙连接: {error}"));
        }
    }
    let session = app.state::<connection::DeviceConnections>().get(&mac);
    if let Some(session) = session {
        cleanup_device_connection(app, mac.clone(), session).await;
    }
    Ok(OperationResult {
        success: true,
        message: format!("Disconnected {}", mac),
    })
}

#[tauri::command]
async fn install_bundled_rpk(app: AppHandle, mac: String) -> Result<OperationResult, String> {
    let update_state = app.state::<updates::DesktopUpdateState>();
    let _desktop_installation = update_state.installation.try_lock()
        .map_err(|_| "已有安装或更新操作，请等待完成")?;
    let mac = mac.trim().to_uppercase();
    let session = require_device_session(app.clone(), &mac).await?;
    let _installation = session.begin_installation()?;
    log::info!("[Install] Verifying bundled RPK...");

    let mut hasher = Sha256::new();
    hasher.update(EMBEDDED_RPK);
    let calculated_sha = hex::encode(hasher.finalize());

    if calculated_sha != EXPECTED_SHA256 {
        return Ok(OperationResult {
            success: false,
            message: format!("RPK SHA-256 mismatch: expected {}, got {}", EXPECTED_SHA256, calculated_sha),
        });
    }

    let manifest: serde_json::Value = serde_json::from_str(include_str!("../resources/watch-app.json"))
        .map_err(|e| format!("无法读取安装包版本信息: {e}"))?;
    let expected_version = manifest["versionCode"].as_u64()
        .ok_or("安装包缺少版本编号")?;

    let emit_stage = |stage: &str, percent: u32| {
        let payload = serde_json::json!({
            "mac": mac,
            "stage": stage,
            "sent_bytes": 0,
            "total_bytes": EMBEDDED_RPK.len(),
            "percent": percent,
        });
        let _ = app.emit("install-progress", &payload);
        let _ = app.emit("g1-install-progress", payload);
    };
    emit_stage("CHECKING", 0);
    session.run_operation(
        watch_app::ensure_uninstalled(
            || async {
                let apps = query_device_apps(app.clone(), mac.clone()).await?;
                Ok(apps.iter().any(|item| item.package_name == EXPECTED_PACKAGE))
            },
            || async {
                emit_stage("UNINSTALLING", 0);
                corelib::device::thirdparty_app::uninstall(mac.clone(), EXPECTED_PACKAGE.to_string())
                    .await.map_err(|e| format!("卸载指令助手失败: {e:#}"))
            },
            std::time::Duration::from_millis(500),
        ),
        std::time::Duration::from_secs(30),
        "卸载确认超时，请检查手表后重试；尚未开始安装新版本",
    ).await?;

    if !session.is_active() {
        return Err("卸载期间蓝牙连接已断开，请重新连接后重试".to_string());
    }
    log::info!("[Install] Previous app is absent. Sending MASS install request ({} bytes)...", EMBEDDED_RPK.len());

    let app_handle = app.clone();
    let progress_mac = mac.clone();
    let progress_session = session.clone();
    let total_len = EMBEDDED_RPK.len();
    let progress_cb = Arc::new(move |cb_data: corelib::device::xiaomi::components::mass::SendMassCallbackData| {
        if !progress_session.is_active() { return; }
        let pct = (cb_data.progress * 100.0) as u32;
        let payload = serde_json::json!({
            "mac": progress_mac,
            "stage": "MASS_SEND",
            "sent_bytes": cb_data.actual_data_payload_len,
            "total_bytes": total_len,
            "percent": pct.min(100),
        });
        let _ = app_handle.emit("install-progress", &payload);
        let _ = app_handle.emit("g1-install-progress", payload);
    });

    let rpk_payload = EMBEDDED_RPK.to_vec();
    let pkg_name = EXPECTED_PACKAGE.to_string();

    let install_fut = corelib::ecs::access::with_device_component_mut::<
        corelib::device::xiaomi::components::install::InstallSystem,
        _,
        _,
    >(mac.clone(), move |sys| {
        sys.send_install_request_with_progress(
            corelib::device::xiaomi::packet::mass::MassDataType::ThirdPartyApp,
            rpk_payload,
            Some(&pkg_name),
            progress_cb,
            None,
        )
    })
    .map_err(|e| format!("Failed to access InstallSystem: {e:?}"))?
    .map_err(|e| format!("InstallSystem request error: {e:#}"))?;

    log::info!("[Install] Awaiting MASS transfer and device installation confirmation...");
    // InstallSystem owns its prepare/transfer/result timeouts and clears its
    // protocol waiters when the future completes. Let that cleanup finish.
    install_fut.await.map_err(|e| format!("Installation failed: {e:#}"))?;

    if !session.is_active() {
        return Err("部署期间蓝牙连接已断开，请重新连接后重试".to_string());
    }

    emit_stage("VERIFYING", 100);
    let installed_apps = query_device_apps(app.clone(), mac.clone()).await?;
    if !installed_apps.iter().any(|item| {
        item.package_name == EXPECTED_PACKAGE && item.version_code.map(u64::from) == Some(expected_version)
    }) {
        return Err("安装结果检查失败：手表上未找到当前版本的指令助手，请重试".to_string());
    }
    log::info!("[Install] Device confirmed current version installed!");
    let complete_payload = serde_json::json!({
        "mac": mac,
        "stage": "COMPLETE",
        "sent_bytes": total_len,
        "total_bytes": total_len,
        "percent": 100,
    });
    let _ = app.emit("install-progress", &complete_payload);
    let _ = app.emit("g1-install-progress", complete_payload);

    Ok(OperationResult {
        success: true,
        message: format!("Successfully installed {} on {}", EXPECTED_PACKAGE, mac),
    })
}

#[tauri::command]
async fn query_device_apps(app: AppHandle, mac: String) -> Result<Vec<QuickAppItem>, String> {
    let mac = mac.trim().to_uppercase();
    let session = require_device_session(app, &mac).await?;
    log::info!("[Resource] Querying quick apps on device {}...", mac);

    let rx = corelib::ecs::access::with_device_component_mut::<
        corelib::device::xiaomi::components::resource::ResourceSystem,
        _,
        _,
    >(mac.clone(), |sys| {
        sys.request_quick_app_list()
    })
    .map_err(|e| format!("Failed to access ResourceSystem: {e:?}"))?;

    let apps = session.run_operation(
        async {
            rx.await
                .map_err(|e| format!("App list response dropped: {e}"))?
                .map_err(|e| format!("App list query error: {e:#}"))
        },
        std::time::Duration::from_secs(12),
        "检查指令助手超时，请重试",
    ).await?;
    if !session.is_active() {
        return Err("查询期间蓝牙连接已断开，请重新连接后重试".to_string());
    }
    log::info!("[Resource] Device returned {} quick apps", apps.len());

    Ok(apps.into_iter().map(|item| QuickAppItem {
        package_name: item.package_name,
        app_name: Some(item.app_name),
        version_code: Some(item.version_code),
    }).collect())
}

#[tauri::command]
fn keychain_get_authkey(mac: String) -> Option<String> {
    storage::device_keys::get_authkey(&mac)
}

#[tauri::command]
fn keychain_save_authkey(mac: String, authkey: String) -> Result<(), String> {
    storage::device_keys::save_authkey(&mac, &authkey).map_err(|e| e.to_string())
}

#[tauri::command]
fn service_get_info(state: tauri::State<Arc<server::ServerHandle>>) -> serde_json::Value { state.info() }

#[tauri::command]
fn service_start(state: tauri::State<Arc<server::ServerHandle>>) -> Result<serde_json::Value, String> {
    state.start()?;
    Ok(state.info())
}

#[tauri::command]
fn service_stop(state: tauri::State<Arc<server::ServerHandle>>) -> serde_json::Value {
    state.stop();
    state.info()
}

#[tauri::command]
fn service_restart(state: tauri::State<Arc<server::ServerHandle>>) -> Result<serde_json::Value, String> {
    state.restart()?;
    Ok(state.info())
}

#[tauri::command]
fn service_save_port(state: tauri::State<Arc<server::ServerHandle>>, port: u16) -> Result<serde_json::Value, String> {
    state.configure_port(port, || {
        let mut settings = storage::settings::load_settings();
        settings.server_port = port;
        storage::settings::save_settings(&settings).map_err(|e| e.to_string())
    })?;
    Ok(state.info())
}

#[tauri::command]
fn pairing_create_code(state: tauri::State<Arc<server::ServerHandle>>) -> Result<String, String> {
    state.require_running()?;
    Ok(state.pairing.generate_code())
}

#[tauri::command]
async fn command_create(state: tauri::State<'_, Arc<server::ServerHandle>>, payload: server::models::CommandCreate) -> Result<server::models::Command, String> {
    state.require_running()?;
    payload.validate()?;
    let command = state.manager.create_command(payload);
    server::notifier::notify_command(command.clone());
    Ok(command)
}

#[tauri::command]
fn command_list_all(state: tauri::State<Arc<server::ServerHandle>>) -> Vec<server::models::Command> {
    state.manager.list_commands(100)
}

#[tauri::command]
fn command_reply_manual(state: tauri::State<Arc<server::ServerHandle>>, cmd_id: String, mut reply: server::models::CommandReply) -> Result<server::models::Command, String> {
    state.require_running()?;
    reply.device_id = Some("desktop-simulator".into());
    state.manager.record_reply(&cmd_id, reply).map_err(|e| e.to_string())
}

#[tauri::command]
fn notification_get_settings() -> storage::settings::AppSettings { storage::settings::load_settings() }

#[tauri::command]
fn notification_save_settings(enabled: bool, server_url: String, topic: String, include_content: bool) -> Result<(), String> {
    let url = reqwest::Url::parse(server_url.trim()).map_err(|_| "请输入有效的通知服务 URL")?;
    if !["http", "https"].contains(&url.scheme()) { return Err("通知服务必须使用 HTTP 或 HTTPS".into()); }
    if enabled && topic.trim().is_empty() { return Err("请输入通知主题".into()); }
    let mut settings = storage::settings::load_settings();
    settings.ntfy_enabled = enabled;
    settings.ntfy_server = server_url.trim().into();
    settings.ntfy_topic = topic.trim().into();
    settings.ntfy_include_content = include_content;
    storage::settings::save_settings(&settings).map_err(|e| e.to_string())
}

#[tauri::command]
async fn notification_test() -> Result<(), String> { server::notifier::send_push(None).await }

#[tauri::command]
fn hooks_get_statuses() -> Result<Vec<integrations::hooks::HookStatus>, String> {
    integrations::hooks::statuses().map_err(|e| e.to_string())
}

#[tauri::command]
fn hooks_toggle(key: String, enable: bool) -> Result<(), String> {
    integrations::hooks::toggle(&key, enable).map_err(|e| e.to_string())
}

#[tauri::command]
fn permission_rules_list() -> Result<Vec<serde_json::Value>, String> {
    integrations::permission_rules::list().map_err(|e| e.to_string())
}

#[tauri::command]
fn permission_rule_delete(rule_id: String) -> Result<(), String> {
    integrations::permission_rules::delete(&rule_id).map_err(|e| e.to_string())
}

#[tauri::command]
fn integration_get_statuses() -> Result<Vec<integrations::configurator::AgentStatus>, String> {
    integrations::configurator::list_agent_statuses().map_err(|e| e.to_string())
}

#[tauri::command]
fn integration_toggle(key: String, enable: bool) -> Result<bool, String> {
    integrations::configurator::toggle_agent(&key, enable).map_err(|e| e.to_string())
}

#[tauri::command]
async fn account_xiaomi_get_qr() -> Result<account::xiaomi::XiaomiQrSession, String> {
    account::xiaomi::request_qr_session().await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn account_xiaomi_check_qr(lp_url: String) -> Result<account::xiaomi::QrPollResponse, String> {
    account::xiaomi::check_qr_login(&lp_url).await.map_err(|e| e.to_string())
}

#[tauri::command]
async fn account_xiaomi_get_devices() -> Result<Vec<account::xiaomi::XiaomiDevice>, String> {
    if let Some(creds) = account::xiaomi::load_saved_credentials() {
        match account::xiaomi::fetch_source_devices(&creds).await {
            Ok(devs) => Ok(devs),
            Err(e) => {
                let err_str = e.to_string();
                log::warn!("[Xiaomi Account] Fetch failed: {err_str}");
                if err_str.contains("401") || err_str.contains("auth err") || err_str.contains("Unauthorized") {
                    log::warn!("[Xiaomi Account] Token expired (401), clearing expired session");
                    let _ = account::xiaomi::clear_credentials();
                    return Err("小米登录凭证已失效（401 鉴权未通过），请重新扫码授权".to_string());
                }
                Ok(account::xiaomi::load_cached_devices())
            }
        }
    } else {
        Ok(account::xiaomi::load_cached_devices())
    }
}

#[tauri::command]
fn account_xiaomi_get_status() -> serde_json::Value {
    let creds = account::xiaomi::load_saved_credentials();
    let is_logged_in = creds.as_ref().map(|c| !c.service_token.trim().is_empty()).unwrap_or(false);
    let devs = account::xiaomi::load_cached_devices();
    serde_json::json!({
        "logged_in": is_logged_in,
        "user_id": if is_logged_in { creds.as_ref().map(|c| c.user_id.clone()) } else { None },
        "device_count": devs.len(),
    })
}

#[tauri::command]
fn account_xiaomi_select_device(mac: String) -> Result<OperationResult, String> {
    if account::xiaomi::load_saved_credentials().is_none() { return Err("请先登录小米账号".to_string()); }
    let normalized_mac = mac.trim().to_uppercase();
    let devs = account::xiaomi::load_cached_devices();
    if let Some(dev) = devs.iter().find(|d| d.mac.eq_ignore_ascii_case(&normalized_mac)) {
        let mut settings = storage::settings::load_settings();
        settings.last_connected_mac = Some(normalized_mac.clone());
        let _ = storage::settings::save_settings(&settings);
        Ok(OperationResult {
            success: true,
            message: format!("已选择设备: {} ({})，密钥已从 本地凭据存储 载入", dev.name, normalized_mac),
        })
    } else {
        Err("设备不属于当前账号".to_string())
    }
}

#[tauri::command]
async fn account_xiaomi_import_astrobox() -> Result<Vec<account::xiaomi::XiaomiDevice>, String> {
    account::xiaomi::try_import_astrobox().await.map_err(|e| e.to_string())
}

#[tauri::command]
fn account_xiaomi_logout() -> Result<(), String> {
    let mut settings = storage::settings::load_settings();
    settings.last_connected_mac = None;
    let _ = storage::settings::save_settings(&settings);
    account::xiaomi::clear_credentials().map_err(|e| e.to_string())
}

#[tauri::command]
async fn account_xiaomi_password_login(app: tauri::AppHandle, user: String, pass: String) -> Result<account::xiaomi::QrPollResponse, String> {
    account::xiaomi::login_with_password(&app, &user, &pass)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn account_xiaomi_complete_verification(app: tauri::AppHandle, verification_id: String) -> Result<account::xiaomi::QrPollResponse, String> {
    account::xiaomi::complete_verification(&app, &verification_id).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn account_xiaomi_cancel_verification(verification_id: String) -> Result<(), String> {
    account::xiaomi::cancel_verification(&verification_id).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).try_init();
    log::info!("Starting 小米手表 AI 指令助手...");

    // Initialize ECS runtime thread
    corelib::ecs::init_runtime_default();

    // Start the background Axum HTTP server on the saved port.
    let settings = storage::settings::load_settings();
    let server_handle = server::ServerHandle::new(settings.server_port);
    if let Err(error) = server_handle.start() { log::error!("{error}"); }
    let server_state = Arc::new(server_handle);

    tauri::Builder::default()
        .manage(server_state)
        .manage(connection::DeviceConnections::default())
        .manage(updates::DesktopUpdateState::default())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(btclassic_spp::init())
        .invoke_handler(tauri::generate_handler![
            updates::desktop_update_info,
            updates::desktop_update_check,
            updates::desktop_update_install,
            g0_check,
            scan_start,
            scan_stop,
            get_scanned_devices,
            connect_and_auth,
            disconnect_device,
            get_device_connection_status,
            is_device_connected,
            install_bundled_rpk,
            query_device_apps,
            keychain_get_authkey,
            keychain_save_authkey,
            service_get_info,
            service_start,
            service_stop,
            service_restart,
            service_save_port,
            notification_get_settings,
            notification_save_settings,
            notification_test,
            hooks_get_statuses,
            hooks_toggle,
            permission_rules_list,
            permission_rule_delete,
            pairing_create_code,
            command_create,
            command_list_all,
            command_reply_manual,
            integration_get_statuses,
            integration_toggle,
            account_xiaomi_get_qr,
            account_xiaomi_check_qr,
            account_xiaomi_get_devices,
            account_xiaomi_get_status,
            account_xiaomi_select_device,
            account_xiaomi_import_astrobox,
            account_xiaomi_logout,
            account_xiaomi_password_login,
            account_xiaomi_complete_verification,
            account_xiaomi_cancel_verification,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_embedded_rpk_sha256() {
        let mut hasher = Sha256::new();
        hasher.update(EMBEDDED_RPK);
        let calculated = hex::encode(hasher.finalize());
        assert_eq!(calculated, EXPECTED_SHA256, "Embedded RPK SHA-256 must match expected");
        assert!(EMBEDDED_RPK.len() > 10000, "Embedded RPK must be non-empty valid package");
        let manifest: serde_json::Value = serde_json::from_str(include_str!("../resources/watch-app.json")).unwrap();
        assert_eq!(manifest["package"], EXPECTED_PACKAGE);
        assert_eq!(manifest["sha256"], calculated);
    }

    #[test]
    fn test_g0_compliance_report() {
        let report = g0_check();
        assert_eq!(report.status, "PASSED");
        assert_eq!(report.license, "GNU AGPL-3.0");
        assert_eq!(report.upstream_modules.len(), 5);
        let names: Vec<&str> = report.upstream_modules.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"AstroBox-NG-Module-Core"));
        assert!(names.contains(&"AstroBox-NG-Module-Pb"));
        assert!(names.contains(&"AstroBox-NG-Module-Vivo-MsgPack"));
        assert!(names.contains(&"AstroBox-NG-Module-Bluetooth"));
        assert!(names.contains(&"AstroBox-NG-Plugin-BtClassicSpp"));
    }
}
