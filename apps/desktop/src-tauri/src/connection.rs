use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use tokio::sync::Notify;

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
}

pub struct DeviceSession {
    active: AtomicBool,
    connecting: AtomicBool,
    disconnected: Notify,
    installation: tokio::sync::Mutex<()>,
}

impl DeviceSession {
    pub fn begin_installation(&self) -> Result<tokio::sync::MutexGuard<'_, ()>, String> {
        self.installation.try_lock()
            .map_err(|_| "指令助手正在安装，请等待完成".to_string())
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    pub fn mark_disconnected(&self) -> bool {
        let was_active = self.active.swap(false, Ordering::AcqRel);
        if was_active {
            self.disconnected.notify_waiters();
        }
        was_active
    }

    pub fn is_connecting(&self) -> bool {
        self.connecting.load(Ordering::Acquire)
    }

    pub fn complete_connection(&self) {
        self.connecting.store(false, Ordering::Release);
    }

    pub fn status(&self) -> DeviceConnectionStatus {
        if !self.is_active() {
            DeviceConnectionStatus::Disconnected
        } else if self.is_connecting() {
            DeviceConnectionStatus::Connecting
        } else {
            DeviceConnectionStatus::Connected
        }
    }

    async fn wait_for_disconnect(&self) {
        let notified = self.disconnected.notified();
        tokio::pin!(notified);
        // Register before reading active so a disconnect cannot be missed
        // between the state check and the first poll of the notification.
        notified.as_mut().enable();
        if self.is_active() {
            notified.await;
        }
    }

    pub async fn run_connection<T>(
        &self,
        operation: impl Future<Output = Result<T, String>>,
        timeout: Duration,
    ) -> Result<T, String> {
        self.run_operation(operation, timeout, "连接超时，请确认手表在附近并重新连接").await
    }

    pub async fn run_operation<T>(
        &self,
        operation: impl Future<Output = Result<T, String>>,
        timeout: Duration,
        timeout_message: &str,
    ) -> Result<T, String> {
        tokio::select! {
            biased;
            _ = self.wait_for_disconnect() => Err("蓝牙连接已断开，请重新连接手表".to_string()),
            result = tokio::time::timeout(timeout, operation) => {
                result.unwrap_or_else(|_| Err(timeout_message.to_string()))
            }
        }
    }
}

#[derive(Default)]
pub struct DeviceConnections {
    sessions: Mutex<HashMap<String, Arc<DeviceSession>>>,
}

impl DeviceConnections {
    pub fn begin(&self, mac: &str) -> Result<Arc<DeviceSession>, String> {
        let mut sessions = self.sessions.lock();
        if sessions.contains_key(mac) {
            return Err("设备正在连接或清理连接，请稍后重试".to_string());
        }
        let session = Arc::new(DeviceSession {
            active: AtomicBool::new(true),
            connecting: AtomicBool::new(true),
            disconnected: Notify::new(),
            installation: tokio::sync::Mutex::new(()),
        });
        sessions.insert(mac.to_string(), session.clone());
        Ok(session)
    }

    pub fn get(&self, mac: &str) -> Option<Arc<DeviceSession>> {
        self.sessions.lock().get(mac).cloned()
    }

    // Keep registration locked through cleanup so an old callback cannot
    // remove the protocol entity or cipher belonging to a newer connection.
    pub fn finish(&self, mac: &str, session: &Arc<DeviceSession>, cleanup: impl FnOnce()) -> bool {
        let mut sessions = self.sessions.lock();
        if !sessions
            .get(mac)
            .is_some_and(|current| Arc::ptr_eq(current, session))
        {
            return false;
        }
        session.mark_disconnected();
        cleanup();
        sessions.remove(mac);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installations_are_serialized_and_unlock_after_failure() {
        let connections = DeviceConnections::default();
        let session = connections.begin("watch").unwrap();
        let guard = session.begin_installation().unwrap();
        assert!(session.begin_installation().is_err());
        drop(guard);
        assert!(session.begin_installation().is_ok());
    }

    #[tokio::test]
    async fn device_operations_report_their_own_timeout() {
        let connections = DeviceConnections::default();
        let session = connections.begin("watch").unwrap();
        let result = session.run_operation(
            std::future::pending::<Result<(), String>>(),
            Duration::from_millis(10),
            "卸载确认超时",
        ).await;
        assert_eq!(result.unwrap_err(), "卸载确认超时");
    }

    struct PendingAuthentication(Arc<AtomicBool>);

    impl Drop for PendingAuthentication {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }

    #[test]
    fn delayed_disconnect_cannot_clean_up_a_reconnected_device() {
        let connections = DeviceConnections::default();
        let first = connections.begin("watch").unwrap();
        assert!(connections.begin("watch").is_err());
        assert!(first.mark_disconnected());
        assert!(!first.mark_disconnected());
        assert!(connections.finish("watch", &first, || {}));

        let second = connections.begin("watch").unwrap();
        assert!(!connections.finish("watch", &first, || panic!("stale cleanup ran")));
        assert!(second.is_active());
        assert!(Arc::ptr_eq(&connections.get("watch").unwrap(), &second));
    }

    #[test]
    fn disconnect_only_cleans_up_the_matching_device_once() {
        let connections = DeviceConnections::default();
        let first = connections.begin("first").unwrap();
        let second = connections.begin("second").unwrap();
        let mut cleanup_count = 0;
        assert!(connections.finish("first", &first, || cleanup_count += 1));
        assert!(!connections.finish("first", &first, || cleanup_count += 1));
        assert_eq!(cleanup_count, 1);
        assert!(!first.is_active());
        assert!(connections.get("first").is_none());
        assert!(second.is_active());
    }

    #[test]
    fn finishing_authentication_does_not_revive_a_disconnected_session() {
        let connections = DeviceConnections::default();
        let session = connections.begin("watch").unwrap();
        assert!(session.is_connecting());
        session.mark_disconnected();
        session.complete_connection();
        assert!(!session.is_connecting());
        assert!(!session.is_active());
    }

    #[tokio::test]
    async fn disconnect_ends_pending_authentication_and_allows_retry() {
        let connections = DeviceConnections::default();
        let session = connections.begin("watch").unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let waiting_session = session.clone();
        let authentication_dropped = Arc::new(AtomicBool::new(false));
        let pending_authentication = PendingAuthentication(authentication_dropped.clone());
        let operation = tokio::spawn(async move {
            waiting_session
                .run_connection(
                    async move {
                        let _pending_authentication = pending_authentication;
                        let _ = started_tx.send(());
                        std::future::pending::<Result<(), String>>().await
                    },
                    Duration::from_secs(60),
                )
                .await
        });
        started_rx.await.unwrap();
        session.mark_disconnected();
        let result = tokio::time::timeout(Duration::from_secs(1), operation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err(), "蓝牙连接已断开，请重新连接手表");
        assert!(authentication_dropped.load(Ordering::Acquire));
        session.complete_connection();
        assert!(connections.finish("watch", &session, || {}));
        assert!(connections.begin("watch").is_ok());
    }

    #[tokio::test]
    async fn disconnect_before_waiting_takes_precedence_over_success() {
        let connections = DeviceConnections::default();
        let session = connections.begin("watch").unwrap();
        session.mark_disconnected();
        let result = session
            .run_connection(async { Ok(()) }, Duration::from_secs(60))
            .await;
        assert_eq!(result.unwrap_err(), "蓝牙连接已断开，请重新连接手表");
        assert!(matches!(
            session.status(),
            DeviceConnectionStatus::Disconnected
        ));
    }

    #[tokio::test]
    async fn an_unanswered_connection_times_out() {
        let connections = DeviceConnections::default();
        let session = connections.begin("watch").unwrap();
        let result = session
            .run_connection(
                std::future::pending::<Result<(), String>>(),
                Duration::from_millis(10),
            )
            .await;
        assert_eq!(result.unwrap_err(), "连接超时，请确认手表在附近并重新连接");
        session.complete_connection();
        assert!(connections.finish("watch", &session, || {}));
        assert!(connections.begin("watch").is_ok());
    }

    #[tokio::test]
    async fn a_live_session_preserves_the_connection_result() {
        let connections = DeviceConnections::default();
        let session = connections.begin("watch").unwrap();
        assert!(matches!(
            session.status(),
            DeviceConnectionStatus::Connecting
        ));
        assert_eq!(
            session
                .run_connection(async { Ok(42) }, Duration::from_secs(60))
                .await
                .unwrap(),
            42
        );
        let result = session
            .run_connection(
                async { Err::<(), _>("认证失败".to_string()) },
                Duration::from_secs(60),
            )
            .await;
        assert_eq!(result.unwrap_err(), "认证失败");
        session.complete_connection();
        assert!(matches!(
            session.status(),
            DeviceConnectionStatus::Connected
        ));
    }
}
