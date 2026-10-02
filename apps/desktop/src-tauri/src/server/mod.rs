pub mod command_manager;
pub mod models;
pub mod notifier;
pub mod pairing;
pub mod routes;

use command_manager::CommandManager;
use pairing::PairingManager;
use parking_lot::Mutex;
use routes::{create_router, AppState};
use std::net::SocketAddr;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};
use tokio::sync::oneshot;

#[derive(Default)]
struct RuntimeControl {
    port: u16,
    worker: Option<(oneshot::Sender<()>, std::thread::JoinHandle<()>)>,
    last_error: Option<String>,
}

pub struct ServerHandle {
    pub manager: Arc<CommandManager>,
    pub pairing: Arc<PairingManager>,
    running: Arc<AtomicBool>,
    runtime: Mutex<RuntimeControl>,
}

impl ServerHandle {
    pub fn new(port: u16) -> Self {
        Self::with_pairing(port, Arc::new(PairingManager::new()))
    }

    fn with_pairing(port: u16, pairing: Arc<PairingManager>) -> Self {
        Self {
            manager: Arc::new(CommandManager::new()),
            pairing,
            running: Arc::new(AtomicBool::new(false)),
            runtime: Mutex::new(RuntimeControl { port, ..RuntimeControl::default() }),
        }
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Acquire)
    }

    pub fn require_running(&self) -> Result<(), String> {
        if self.is_running() { Ok(()) } else { Err("指令服务已停止，请先启动服务".into()) }
    }

    pub fn info(&self) -> serde_json::Value {
        let control = self.runtime.lock();
        let lan_ip = local_ip_address::local_ip()
            .map(|ip| ip.to_string()).unwrap_or_else(|_| "127.0.0.1".into());
        let watch = self.pairing.connection_info(self.is_running());
        serde_json::json!({
            "status": if self.is_running() { "running" } else if control.last_error.is_some() { "error" } else { "stopped" },
            "last_error": control.last_error,
            "lan_ip": lan_ip,
            "port": control.port,
            "server_url": format!("http://{}:{}", lan_ip, control.port),
            "active_pairing_code": self.pairing.get_valid_code(),
            "watch_paired": watch.watch_paired,
            "watch_connection_status": watch.watch_connection_status,
            "watch_last_seen_at": watch.watch_last_seen_at,
        })
    }

    pub fn start(&self) -> Result<(), String> {
        let mut control = self.runtime.lock();
        self.start_locked(&mut control)
    }

    fn start_locked(&self, control: &mut RuntimeControl) -> Result<(), String> {
        if self.is_running() { return Ok(()); }
        self.pairing.mark_offline();
        if let Some((_, worker)) = control.worker.take() { let _ = worker.join(); }
        let result = Self::bind_listener(control.port)
            .and_then(|listener| self.spawn_worker(control.port, listener));
        match result {
            Ok(worker) => { control.worker = Some(worker); control.last_error = None; Ok(()) }
            Err(error) => { control.last_error = Some(error.clone()); Err(error) }
        }
    }

    fn bind_listener(port: u16) -> Result<std::net::TcpListener, String> {
        if port == 0 { return Err("服务端口必须是 1–65535 的整数".into()); }
        // Bind before reporting success. Never manage or kill another process using this port.
        let listener = std::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port)))
            .map_err(|e| format!("无法启动指令服务（端口 {}）：{}。请检查端口是否被占用。", port, e))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(listener)
    }

    fn spawn_worker(&self, port: u16, listener: std::net::TcpListener) -> Result<(oneshot::Sender<()>, std::thread::JoinHandle<()>), String> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
        let state = Arc::new(AppState { manager: self.manager.clone(), pairing: self.pairing.clone(), server_port: port });
        let router = create_router(state);
        let running = self.running.clone();
        let (stop, stopped) = oneshot::channel();
        self.running.store(true, Ordering::Release);
        let worker = std::thread::Builder::new().name("command-http-service".into()).spawn(move || {
            rt.block_on(async move {
                match tokio::net::TcpListener::from_std(listener) {
                    Ok(listener) => {
                        let serve = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>());
                        tokio::select! {
                            result = serve => { if let Err(error) = result { log::error!("[Server] {error}"); } }
                            _ = stopped => {}
                        }
                    }
                    Err(error) => log::error!("[Server] {error}"),
                }
                running.store(false, Ordering::Release);
            });
        }).map_err(|e| { self.running.store(false, Ordering::Release); e.to_string() })?;
        Ok((stop, worker))
    }

    pub fn stop(&self) {
        let mut control = self.runtime.lock();
        self.stop_locked(&mut control);
    }

    fn stop_locked(&self, control: &mut RuntimeControl) {
        if let Some((stop, worker)) = control.worker.take() {
            let _ = stop.send(());
            let _ = worker.join();
        }
        self.running.store(false, Ordering::Release);
        self.pairing.mark_offline();
        self.manager.expire_pending();
        control.last_error = None;
    }

    pub fn restart(&self) -> Result<(), String> {
        let mut control = self.runtime.lock();
        self.stop_locked(&mut control);
        self.start_locked(&mut control)
    }

    pub fn configure_port(&self, port: u16, persist: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        if port == 0 { return Err("服务端口必须是 1–65535 的整数".into()); }
        let mut control = self.runtime.lock();
        if control.port == port { return Ok(()); }
        // Reserve the new port and save settings before interrupting the current service.
        let listener = Self::bind_listener(port)?;
        persist().map_err(|error| format!("无法保存服务端口：{error}"))?;
        let was_running = self.is_running();
        self.stop_locked(&mut control);
        control.port = port;
        if was_running {
            match self.spawn_worker(port, listener) {
                Ok(worker) => { control.worker = Some(worker); }
                Err(error) => { control.last_error = Some(error.clone()); return Err(error); }
            }
        }
        Ok(())
    }
}

impl Drop for ServerHandle {
    fn drop(&mut self) { self.stop(); }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unused_port() -> u16 {
        std::net::TcpListener::bind("0.0.0.0:0").unwrap().local_addr().unwrap().port()
    }

    #[tokio::test]
    async fn changing_running_port_moves_http_and_keeps_pairing_and_history() {
        let old_port = unused_port();
        let server = ServerHandle::with_pairing(old_port, Arc::new(PairingManager::for_test()));
        server.start().unwrap();
        let token = server.pairing.pair_with_code(&server.pairing.generate_code()).unwrap();
        assert!(server.pairing.authenticate_watch(&token, "192.168.1.20".parse().unwrap()));
        let payload = serde_json::from_value(serde_json::json!({"content":"pending", "timeout_seconds":60})).unwrap();
        let pending = server.manager.create_command(payload);
        let new_port = unused_port();
        let mut saved_port = old_port;
        server.configure_port(new_port, || { saved_port = new_port; Ok(()) }).unwrap();
        assert_eq!(saved_port, new_port);
        assert_eq!(server.info()["status"], "running");
        assert_eq!(server.info()["port"], new_port);
        assert!(server.info()["server_url"].as_str().unwrap().ends_with(&format!(":{new_port}")));
        assert!(server.pairing.authenticate(&token));
        assert_eq!(server.info()["watch_connection_status"], "offline");
        assert_eq!(server.manager.get_command(&pending.id).unwrap().status, models::CommandStatus::Expired);
        assert!(std::net::TcpListener::bind(("0.0.0.0", old_port)).is_ok());
        let client = reqwest::Client::new();
        let info: serde_json::Value = client.get(format!("http://127.0.0.1:{new_port}/api/v1/config/server-info"))
            .send().await.unwrap().json().await.unwrap();
        assert_eq!(info["port"], new_port);
        assert_eq!(info["watch_paired"], true);
        server.restart().unwrap();
        assert_eq!(server.info()["port"], new_port);
        assert!(client.get(format!("http://127.0.0.1:{new_port}/api/health")).send().await.unwrap().status().is_success());
    }

    #[tokio::test]
    async fn rejected_port_changes_leave_the_old_service_and_commands_untouched() {
        let old_port = unused_port();
        let server = ServerHandle::with_pairing(old_port, Arc::new(PairingManager::for_test()));
        server.start().unwrap();
        let payload = serde_json::from_value(serde_json::json!({"content":"keep pending", "timeout_seconds":60})).unwrap();
        let pending = server.manager.create_command(payload);
        let occupied = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let mut saves = 0;
        assert!(server.configure_port(occupied.local_addr().unwrap().port(), || { saves += 1; Ok(()) }).unwrap_err().contains("占用"));
        assert_eq!(saves, 0);
        assert!(server.configure_port(0, || { saves += 1; Ok(()) }).is_err());
        assert_eq!(saves, 0);
        let new_port = unused_port();
        assert!(server.configure_port(new_port, || Err("disk full".into())).unwrap_err().contains("disk full"));
        assert!(std::net::TcpListener::bind(("0.0.0.0", new_port)).is_ok());
        assert_eq!(server.info()["port"], old_port);
        assert_eq!(server.info()["status"], "running");
        assert!(server.info()["last_error"].is_null());
        assert_eq!(server.manager.get_command(&pending.id).unwrap().status, models::CommandStatus::Pending);
        assert!(occupied.local_addr().is_ok());
        assert!(reqwest::Client::new().get(format!("http://127.0.0.1:{old_port}/api/health")).send().await.unwrap().status().is_success());
    }

    #[tokio::test]
    async fn changing_stopped_port_saves_without_starting_and_next_start_uses_it() {
        let old_port = unused_port();
        let server = ServerHandle::with_pairing(old_port, Arc::new(PairingManager::for_test()));
        let new_port = loop { let port = unused_port(); if port != old_port { break port; } };
        let mut saved = false;
        server.configure_port(new_port, || { saved = true; Ok(()) }).unwrap();
        assert!(saved);
        assert_eq!(server.info()["status"], "stopped");
        assert_eq!(server.info()["port"], new_port);
        assert!(std::net::TcpListener::bind(("0.0.0.0", new_port)).is_ok());
        server.start().unwrap();
        assert!(reqwest::Client::new().get(format!("http://127.0.0.1:{new_port}/api/health")).send().await.unwrap().status().is_success());
    }

    #[test]
    fn unchanged_port_does_not_save_or_expire_pending_commands() {
        let port = unused_port();
        let server = ServerHandle::with_pairing(port, Arc::new(PairingManager::for_test()));
        server.start().unwrap();
        let payload = serde_json::from_value(serde_json::json!({"content":"keep pending", "timeout_seconds":60})).unwrap();
        let pending = server.manager.create_command(payload);
        server.configure_port(port, || panic!("unchanged port must not write settings")).unwrap();
        assert_eq!(server.info()["status"], "running");
        assert_eq!(server.manager.get_command(&pending.id).unwrap().status, models::CommandStatus::Pending);
    }

    #[tokio::test]
    async fn lifecycle_preserves_history_and_disables_web_management() {
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let server = ServerHandle::with_pairing(port, Arc::new(PairingManager::for_test()));
        assert_eq!(server.info()["status"], "stopped");
        assert!(server.info()["active_pairing_code"].is_null());
        let code = server.pairing.generate_code();
        let token = server.pairing.pair_with_code(&code).unwrap();
        assert_eq!(server.info()["watch_connection_status"], "offline");
        server.start().unwrap();
        server.start().unwrap();
        assert_eq!(server.info()["watch_connection_status"], "waiting");
        let client = reqwest::Client::new();
        for path in ["/", "/index.html", "/static/index.html", "/docs", "/redoc"] {
            let response = client.get(format!("http://127.0.0.1:{port}{path}")).send().await.unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);
        }
        let health = client.get(format!("http://127.0.0.1:{port}/api/health")).send().await.unwrap();
        assert!(health.status().is_success());
        let payload: models::CommandCreate = serde_json::from_value(serde_json::json!({ "content": "test", "timeout_seconds": 60 })).unwrap();
        let command = server.manager.create_command(payload);
        assert!(server.pairing.authenticate_watch(&token, "192.168.1.20".parse().unwrap()));
        assert_eq!(server.info()["watch_connection_status"], "online");
        server.restart().unwrap();
        assert!(server.pairing.authenticate(&token));
        assert_eq!(server.info()["watch_connection_status"], "offline");
        assert_eq!(server.manager.get_command(&command.id).unwrap().status, models::CommandStatus::Expired);
        server.stop();
        assert_eq!(server.info()["status"], "stopped");
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    }

    #[test]
    fn occupied_port_is_reported_without_stopping_its_owner() {
        let reservation = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        let server = ServerHandle::with_pairing(reservation.local_addr().unwrap().port(), Arc::new(PairingManager::for_test()));
        assert!(server.start().is_err());
        assert_eq!(server.info()["status"], "error");
        server.stop();
        assert!(reservation.local_addr().is_ok());
    }

    #[tokio::test]
    async fn stopping_closes_waiting_http_requests_and_expires_pending_commands() {
        let reservation = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let server = ServerHandle::with_pairing(port, Arc::new(PairingManager::for_test()));
        server.start().unwrap();
        let request = tokio::spawn(async move {
            reqwest::Client::new().post(format!("http://127.0.0.1:{port}/api/v1/commands/wait-reply"))
                .json(&serde_json::json!({"content":"pending request", "timeout_seconds":60}))
                .timeout(std::time::Duration::from_secs(2)).send().await
        });
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while server.manager.get_all_pending().is_empty() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        server.stop();
        // Shutdown may close the socket or resolve the dropped waiter before
        // the connection closes. Neither path may claim a successful wrist reply.
        if let Ok(response) = request.await.unwrap() {
            if let Ok(body) = response.json::<serde_json::Value>().await {
                assert!(matches!(body["status"].as_str(), Some("timeout" | "error")), "unexpected shutdown response: {body}");
            }
        }
        assert!(server.manager.get_all_pending().is_empty());
        assert_eq!(server.manager.list_commands(1)[0].status, models::CommandStatus::Expired);
    }
}
