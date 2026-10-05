use super::command_manager::CommandManager;
use super::models::{CommandCreate, CommandReply, PairResponse, WatchPendingResponse};
use super::pairing::PairingManager;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, Path, Query, State,
    },
    http::{HeaderMap, StatusCode},
    response::{Json, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use std::net::SocketAddr;
use std::sync::Arc;

pub struct AppState {
    pub manager: Arc<CommandManager>,
    pub pairing: Arc<PairingManager>,
    pub server_port: u16,
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub limit: Option<usize>,
}

fn is_loopback(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback()
}

fn check_watch_auth(headers: &HeaderMap, pairing: &PairingManager, client_addr: &SocketAddr) -> bool {
    if is_loopback(client_addr) {
        return true; // Local simulator allowed
    }
    if let Some(auth_val) = headers.get("authorization") {
        if let Ok(auth_str) = auth_val.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ").or_else(|| auth_str.strip_prefix("bearer ")) {
                return pairing.authenticate_watch(token, client_addr.ip());
            }
        }
    }
    false
}

async fn health_check() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "app": "Redmi Watch 5 Command Hub",
        "name": "小米手表 AI 指令助手",
        "version": env!("CARGO_PKG_VERSION")
    }))
}

async fn create_command(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CommandCreate>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_loopback(&addr) {
        return Err(StatusCode::FORBIDDEN);
    }
    payload.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    let cmd = state.manager.create_command(payload);
    super::notifier::notify_command(cmd.clone());
    Ok(Json(serde_json::json!({
        "status": "ok",
        "command": cmd
    })))
}

async fn create_command_and_wait(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    Json(payload): Json<CommandCreate>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_loopback(&addr) {
        return Err(StatusCode::FORBIDDEN);
    }
    payload.validate().map_err(|_| StatusCode::BAD_REQUEST)?;
    let res = state.manager.create_and_wait(payload).await;
    Ok(Json(res))
}

async fn list_commands(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListQuery>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_loopback(&addr) {
        return Err(StatusCode::FORBIDDEN);
    }
    let limit = query.limit.unwrap_or(50);
    let commands = state.manager.list_commands(limit);
    Ok(Json(serde_json::json!({
        "commands": commands
    })))
}

async fn get_command(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    Path(cmd_id): Path<String>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_loopback(&addr) {
        return Err(StatusCode::FORBIDDEN);
    }
    match state.manager.get_command(&cmd_id) {
        Some(cmd) => Ok(Json(serde_json::json!({ "command": cmd }))),
        None => Err(StatusCode::NOT_FOUND),
    }
}

async fn pair_watch(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    body: axum::body::Bytes,
) -> Result<Json<PairResponse>, (StatusCode, Json<serde_json::Value>)> {
    let payload: serde_json::Value = serde_json::from_slice(&body).unwrap_or_else(|_| serde_json::json!({}));
    let code = match payload.get("code") {
        Some(serde_json::Value::String(s)) => s.trim().to_string(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => "".to_string(),
    };
    log::info!("[Pairing] Incoming pair request from {} with code: '{}'", addr, code);

    if code.len() != 6 || !code.chars().all(|c| c.is_ascii_digit()) {
        log::warn!("[Pairing] Malformed pairing code rejected: '{}'", code);
        return Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "status": "error",
                "message": "Six-digit numeric pairing code required"
            })),
        ));
    }

    if let Some(token) = state.pairing.pair_with_code(&code) {
        log::info!("[Pairing] Successfully paired watch from {}", addr);
        Ok(Json(PairResponse {
            status: "paired".to_string(),
            token,
        }))
    } else {
        log::warn!("[Pairing] Code verification failed for {} (code: '{}')", addr, code);
        Err((
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "status": "error",
                "message": "Invalid or expired 6-digit pairing code"
            })),
        ))
    }
}

async fn get_watch_pending(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
) -> Result<Json<WatchPendingResponse>, StatusCode> {
    if !check_watch_auth(&headers, &state.pairing, &addr) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let pending_list = state.manager.get_all_pending();
    let latest = pending_list.first().cloned();
    Ok(Json(WatchPendingResponse {
        has_command: !pending_list.is_empty(),
        command: latest,
        total: pending_list.len(),
        commands: pending_list,
    }))
}

async fn reply_watch_command(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    State(state): State<Arc<AppState>>,
    Path(cmd_id): Path<String>,
    Json(reply): Json<CommandReply>,
) -> Result<Json<serde_json::Value>, (StatusCode, Json<serde_json::Value>)> {
    if !check_watch_auth(&headers, &state.pairing, &addr) {
        return Err((StatusCode::UNAUTHORIZED, Json(serde_json::json!({ "error": "Unauthorized" }))));
    }
    match state.manager.record_reply(&cmd_id, reply) {
        Ok(updated) => Ok(Json(serde_json::json!({
            "status": "accepted",
            "command": updated
        }))),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "status": "error", "message": e.to_string() })),
        )),
    }
}

async fn get_pairing_code(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_loopback(&addr) {
        return Err(StatusCode::FORBIDDEN);
    }
    let code = state.pairing.generate_code();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64();
    Ok(Json(serde_json::json!({
        "status": "ok",
        "code": code,
        "expires_in_seconds": 300,
        "expires_at": now + 300.0
    })))
}

async fn get_server_info(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_loopback(&addr) {
        return Err(StatusCode::FORBIDDEN);
    }
    let lan_ip = local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string());
    let valid_code = state.pairing.get_valid_code();
    let watch = state.pairing.connection_info(true);
    Ok(Json(serde_json::json!({
        "lan_ip": lan_ip,
        "port": state.server_port,
        "server_url": format!("http://{}:{}", lan_ip, state.server_port),
        "active_pairing_code": valid_code,
        "watch_paired": watch.watch_paired,
        "watch_connection_status": watch.watch_connection_status,
        "watch_last_seen_at": watch.watch_last_seen_at,
    })))
}

async fn get_hooks_config(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_loopback(&addr) {
        return Err(StatusCode::FORBIDDEN);
    }
    let statuses = crate::integrations::hooks::statuses().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let lan_ip = local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string());

    let mut map = serde_json::Map::new();
    map.insert("local_ip".to_string(), serde_json::Value::String(lan_ip));

    for st in statuses {
        let st = serde_json::to_value(st).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        map.insert(
            st["key"].as_str().unwrap_or_default().to_string(),
            serde_json::json!({
                "name": st["name"],
                "installed": st["configured"],
                "approval_installed": st["approval_installed"],
                "approval_supported": st["approval_supported"],
                "attention_installed": st["attention_installed"],
                "completion_installed": st["completion_installed"],
                "path": st["config_path"]
            }),
        );
    }
    Ok(Json(serde_json::Value::Object(map)))
}

#[derive(Deserialize)]
struct HookToggleReq {
    enable: bool,
}

async fn toggle_hook(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(agent_key): Path<String>,
    Json(payload): Json<HookToggleReq>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_loopback(&addr) {
        return Err(StatusCode::FORBIDDEN);
    }
    let key = match agent_key.as_str() {
        "claude" => "claude_code",
        other => other,
    };
    match crate::integrations::hooks::toggle(key, payload.enable) {
        Ok(()) => Ok(Json(serde_json::json!({ "status": "ok", "enabled": payload.enable }))),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn kimi_hook(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
    Json(event): Json<crate::integrations::kimi::BridgeEvent>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !is_loopback(&addr) { return Err(StatusCode::FORBIDDEN); }
    if !event.valid() { return Err(StatusCode::BAD_REQUEST); }
    super::kimi_bridge::enqueue(event, state.manager.clone());
    Ok(Json(serde_json::json!({"status":"accepted"})))
}

async fn ws_events_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
) -> Result<Response, StatusCode> {
    if !is_loopback(&addr) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(ws.on_upgrade(handle_socket))
}

async fn handle_socket(mut socket: WebSocket) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        interval.tick().await;
        let event = serde_json::json!({
            "type": "heartbeat",
            "timestamp": chrono::Utc::now().timestamp()
        });
        if socket.send(Message::Text(event.to_string())).await.is_err() {
            break;
        }
    }
}

pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(health_check))
        .route("/api/v1/commands", post(create_command).get(list_commands))
        .route("/api/v1/commands/wait-reply", post(create_command_and_wait))
        .route("/api/v1/commands/:cmd_id", get(get_command))
        .route("/api/v1/watch/pair", post(pair_watch))
        .route("/api/v1/watch/pending", get(get_watch_pending))
        .route("/api/v1/watch/commands/:cmd_id/reply", post(reply_watch_command))
        .route("/api/v1/config/pairing", get(get_pairing_code))
        .route("/api/v1/config/server-info", get(get_server_info))
        .route("/api/v1/config/hooks", get(get_hooks_config))
        .route("/api/v1/config/hooks/:agent", post(toggle_hook))
        .route("/api/v1/integrations/kimi/hook", post(kimi_hook))
        .route("/ws/events", get(ws_events_handler))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paired_state() -> (Arc<AppState>, String) {
        let pairing = Arc::new(PairingManager::for_test());
        let code = pairing.generate_code();
        let token = pairing.pair_with_code(&code).unwrap();
        (Arc::new(AppState { manager: Arc::new(CommandManager::new()), pairing, server_port: 8000 }), token)
    }

    #[tokio::test]
    async fn kimi_bridge_rejects_lan_requests_and_invalid_session_ids() {
        use crate::integrations::kimi::BridgeEvent;
        let (state,_)=paired_state();
        let event=BridgeEvent::Completion {session_id:"session_test".into(),server_id:"server_test".into(),turn_id:1,cwd:String::new()};
        assert_eq!(kimi_hook(ConnectInfo("192.168.1.20:5000".parse().unwrap()),State(state.clone()),Json(event)).await.err(),Some(StatusCode::FORBIDDEN));
        let event=BridgeEvent::Completion {session_id:"../other".into(),server_id:"server_test".into(),turn_id:1,cwd:String::new()};
        assert_eq!(kimi_hook(ConnectInfo("127.0.0.1:5000".parse().unwrap()),State(state),Json(event)).await.err(),Some(StatusCode::BAD_REQUEST));
    }

    #[tokio::test]
    async fn simulator_requests_never_establish_watch_connectivity() {
        let (state, token) = paired_state();
        let local: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", format!("Bearer {token}").parse().unwrap());
        assert!(get_watch_pending(ConnectInfo(local), headers, State(state.clone())).await.is_ok());
        let info = get_server_info(ConnectInfo(local), State(state)).await.unwrap().0;
        assert_eq!(info["watch_paired"], true);
        assert_eq!(info["watch_connection_status"], "waiting");
        assert!(info["watch_last_seen_at"].is_null());
    }

    #[tokio::test]
    async fn remote_polling_exposes_authentication_failure_and_recovery() {
        let (state, token) = paired_state();
        let remote: SocketAddr = "192.168.1.20:5000".parse().unwrap();
        let local: SocketAddr = "127.0.0.1:5000".parse().unwrap();
        assert_eq!(get_watch_pending(ConnectInfo(remote), HeaderMap::new(), State(state.clone())).await.err(), Some(StatusCode::UNAUTHORIZED));
        assert_eq!(state.pairing.connection_info(true).watch_connection_status, "waiting");
        for (bearer, expected_status) in [("obsolete-token", "auth_failed"), (token.as_str(), "online")] {
            let mut headers = HeaderMap::new();
            headers.insert("authorization", format!("Bearer {bearer}").parse().unwrap());
            let response = get_watch_pending(ConnectInfo(remote), headers, State(state.clone())).await;
            assert_eq!(response.is_ok(), expected_status == "online");
            let info = get_server_info(ConnectInfo(local), State(state.clone())).await.unwrap().0;
            assert_eq!(info["watch_paired"], true);
            assert_eq!(info["watch_connection_status"], expected_status);
        }
    }

    #[tokio::test]
    async fn rejected_watch_replies_also_report_pairing_errors() {
        let (state, _) = paired_state();
        let remote: SocketAddr = "192.168.1.20:5000".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer obsolete-token".parse().unwrap());
        let reply = serde_json::from_value(serde_json::json!({"action_id": "confirm_connection"})).unwrap();
        let response = reply_watch_command(ConnectInfo(remote), headers, State(state.clone()), Path("test".into()), Json(reply)).await;
        assert_eq!(response.err().unwrap().0, StatusCode::UNAUTHORIZED);
        assert_eq!(state.pairing.connection_info(true).watch_connection_status, "auth_failed");
    }
}
