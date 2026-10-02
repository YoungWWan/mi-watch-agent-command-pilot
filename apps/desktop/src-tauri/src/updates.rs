use std::{sync::Arc, time::Duration};

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};
use tokio::sync::Mutex;

use crate::server::ServerHandle;

#[derive(Default)]
pub struct DesktopUpdateState {
    update: Mutex<Option<Update>>,
    // Shared with watch installation so neither installer can interrupt the other.
    pub installation: Mutex<()>,
}

#[derive(Serialize)]
pub struct DesktopUpdateInfo {
    version: String,
    enabled: bool,
    message: &'static str,
}

#[derive(Serialize)]
pub struct AvailableDesktopUpdate {
    version: String,
    notes: String,
    date: Option<String>,
}

fn configured(app: &AppHandle) -> bool {
    app.config().plugins.0.get("updater").is_some_and(|config| {
        config["pubkey"]
            .as_str()
            .is_some_and(|key| !key.trim().is_empty())
            && config["endpoints"].as_array().is_some_and(|endpoints| {
                !endpoints.is_empty()
                    && endpoints.iter().all(|value| {
                        value
                            .as_str()
                            .and_then(|value| reqwest::Url::parse(value).ok())
                            .is_some_and(|url| {
                                url.scheme() == "https"
                                    && url.host_str().is_some()
                                    && url.username().is_empty()
                                    && url.password().is_none()
                            })
                    })
            })
    })
}

#[tauri::command]
pub fn desktop_update_info(app: AppHandle) -> DesktopUpdateInfo {
    let enabled = configured(&app);
    DesktopUpdateInfo {
        version: app.package_info().version.to_string(),
        enabled,
        message: if enabled {
            "启动时自动检查，每 6 小时再次检查。"
        } else {
            "此版本尚未开通在线更新。"
        },
    }
}

#[tauri::command]
pub async fn desktop_update_check(
    app: AppHandle,
    state: State<'_, DesktopUpdateState>,
) -> Result<Option<AvailableDesktopUpdate>, String> {
    if !configured(&app) {
        return Err("此版本尚未开通在线更新".into());
    }
    let mut cached = state
        .update
        .try_lock()
        .map_err(|_| "更新操作正在进行，请稍后重试")?;
    // Check quickly, but allow a larger signed installer enough time to download.
    let updater = app
        .updater_builder()
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|error| error.to_string())?;
    let update = tokio::time::timeout(Duration::from_secs(15), updater.check())
        .await
        .map_err(|_| "检查更新网络超时，请稍后重试".to_string())?
        .map_err(|error| error.to_string())?;
    let result = update.as_ref().map(|update| AvailableDesktopUpdate {
        version: update.version.clone(),
        notes: update.body.clone().unwrap_or_default(),
        date: update.date.map(|date| date.to_string()),
    });
    // On network failure keep the last discovered update available for retry.
    *cached = update;
    Ok(result)
}

fn require_no_pending(server: &ServerHandle) -> Result<(), String> {
    if server.manager.get_all_pending().is_empty() {
        Ok(())
    } else {
        Err("请先完成待回复指令，再更新桌面应用".into())
    }
}

fn progress(app: &AppHandle, phase: &str, downloaded: u64, total: Option<u64>) {
    let _ = app.emit(
        "desktop-update-progress",
        serde_json::json!({
            "phase": phase, "downloaded": downloaded, "total": total,
        }),
    );
}

#[tauri::command]
pub async fn desktop_update_install(
    app: AppHandle,
    state: State<'_, DesktopUpdateState>,
    server: State<'_, Arc<ServerHandle>>,
) -> Result<(), String> {
    if !configured(&app) {
        return Err("此版本尚未开通在线更新".into());
    }
    let _installation = state
        .installation
        .try_lock()
        .map_err(|_| "请等待当前安装完成，再更新桌面应用")?;
    let cached = state
        .update
        .try_lock()
        .map_err(|_| "更新操作正在进行，请稍后重试")?;
    let update = cached.as_ref().ok_or("请先检查更新")?;
    require_no_pending(&server)?;
    let mut downloaded = 0_u64;
    let bytes = update
        .download(
            |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                progress(&app, "downloading", downloaded, total);
            },
            || progress(&app, "verifying", 0, None),
        )
        .await
        .map_err(|error| error.to_string())?;
    // download() verifies the mandatory signature before returning any bytes.
    // Commands may have arrived during download; keep serving them until finished.
    require_no_pending(&server)?;
    progress(
        &app,
        "installing",
        bytes.len() as u64,
        Some(bytes.len() as u64),
    );
    let was_running = server.is_running();
    server.stop();
    if let Err(error) = update.install(bytes) {
        let recovery = if was_running {
            server.start().err()
        } else {
            None
        };
        return Err(match recovery {
            Some(recovery) => format!("安装失败：{error}；指令服务恢复失败：{recovery}"),
            None => error.to_string(),
        });
    }
    progress(&app, "restarting", 0, None);
    // On Windows the signed installer exits and restarts the app itself.
    #[cfg(not(target_os = "windows"))]
    app.request_restart();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_and_desktop_installers_share_one_gate_and_release_on_failure() {
        let state = DesktopUpdateState::default();
        let watch_install = state.installation.try_lock().unwrap();
        assert!(state.installation.try_lock().is_err());
        drop(watch_install);
        assert!(state.installation.try_lock().is_ok());
    }

    #[test]
    fn pending_commands_block_restart_until_answered() {
        let server = ServerHandle::new(0);
        let payload = serde_json::from_value(
            serde_json::json!({"content": "继续吗？", "timeout_seconds": 60}),
        )
        .unwrap();
        server.manager.create_command(payload);
        assert!(require_no_pending(&server).is_err());
        server.manager.expire_pending();
        assert!(require_no_pending(&server).is_ok());
    }

    // Exercise the actual Tauri downloader and signature/version verification.
    // The payload is text, and these tests never call install() or restart().
    async fn download_fixture(tampered: bool, version: &str) -> Result<Vec<u8>, String> {
        use axum::{routing::get, Json, Router};
        use tauri::test::{mock_builder, mock_context, noop_assets};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let signature = include_str!("../tests/fixtures/desktop-update.txt.sig");
        let metadata = serde_json::json!({
            "version": version, "notes": "test", "url": format!("http://{address}/package"),
            "signature": signature.trim(),
        });
        let bytes = if tampered {
            b"tampered".as_slice()
        } else {
            include_bytes!("../tests/fixtures/desktop-update.txt").as_slice()
        };
        let router = Router::new()
            .route("/latest.json", get(move || async move { Json(metadata) }))
            .route("/package", get(move || async move { bytes }));
        let worker = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let source: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let mut context = mock_context(noop_assets());
        context.config_mut().plugins.0.insert(
            "updater".into(),
            serde_json::json!({
                "endpoints": [format!("http://{address}/latest.json")],
                "pubkey": source["plugins"]["updater"]["pubkey"],
                "requireSignedVersion": true,
                "dangerousInsecureTransportProtocol": true,
            }),
        );
        let app = mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .unwrap();
        let updater = app
            .updater_builder()
            .timeout(Duration::from_secs(5))
            .no_proxy()
            .build()
            .unwrap();
        let result = async {
            let update = updater
                .check()
                .await
                .map_err(|error| error.to_string())?
                .unwrap();
            update
                .download(|_, _| {}, || {})
                .await
                .map_err(|error| error.to_string())
        }
        .await;
        worker.abort();
        result
    }

    #[tokio::test]
    async fn signed_download_succeeds_and_tampered_payload_is_rejected() {
        assert_eq!(
            download_fixture(false, "1.1.1").await.unwrap(),
            include_bytes!("../tests/fixtures/desktop-update.txt")
        );
        assert!(download_fixture(true, "1.1.1").await.is_err());
    }

    #[tokio::test]
    async fn manifest_cannot_use_an_old_signature_for_a_new_version() {
        assert!(download_fixture(false, "9.9.9").await.is_err());
    }
}
