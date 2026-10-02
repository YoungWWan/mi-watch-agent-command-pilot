use std::future::Future;
use std::time::Duration;

// Xiaomi REMOVE_APP has no acknowledgement. Re-query the watch until the
// package disappears before allowing the caller to send an installation.
pub async fn ensure_uninstalled<Q, QF, U, UF>(
    mut is_installed: Q,
    uninstall: U,
    poll_interval: Duration,
) -> Result<(), String>
where
    Q: FnMut() -> QF,
    QF: Future<Output = Result<bool, String>>,
    U: FnOnce() -> UF,
    UF: Future<Output = Result<(), String>>,
{
    if !is_installed().await? {
        return Ok(());
    }
    uninstall().await?;
    loop {
        tokio::time::sleep(poll_interval).await;
        if !is_installed().await? {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn installed_app_is_removed_and_confirmed_absent_before_install() {
        let steps = Arc::new(Mutex::new(Vec::new()));
        let query_steps = steps.clone();
        let remove_steps = steps.clone();
        let mut replies = [true, true, false].into_iter();
        ensure_uninstalled(
            move || {
                let installed = replies.next().unwrap();
                query_steps
                    .lock()
                    .unwrap()
                    .push(if installed { "present" } else { "absent" });
                async move { Ok(installed) }
            },
            move || async move {
                remove_steps.lock().unwrap().push("uninstall");
                Ok(())
            },
            Duration::ZERO,
        )
        .await
        .unwrap();
        steps.lock().unwrap().push("install");
        assert_eq!(
            *steps.lock().unwrap(),
            ["present", "uninstall", "present", "absent", "install"]
        );
    }

    #[tokio::test]
    async fn missing_app_can_be_installed_without_an_invalid_removal_request() {
        ensure_uninstalled(
            || async { Ok(false) },
            || async { panic!("missing apps have no fingerprint for removal") },
            Duration::ZERO,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn uninstall_failure_stops_installation() {
        let result = ensure_uninstalled(
            || async { Ok(true) },
            || async { Err("卸载失败".to_string()) },
            Duration::ZERO,
        )
        .await;
        assert_eq!(result.unwrap_err(), "卸载失败");
    }

    #[tokio::test]
    async fn query_failure_is_not_treated_as_an_absent_app() {
        let result = ensure_uninstalled(
            || async { Err("查询失败".to_string()) },
            || async { panic!("must not remove after a failed query") },
            Duration::ZERO,
        )
        .await;
        assert_eq!(result.unwrap_err(), "查询失败");
    }

    #[tokio::test]
    async fn failed_confirmation_stops_installation() {
        let mut first = true;
        let result = ensure_uninstalled(
            move || {
                let result = if first {
                    Ok(true)
                } else {
                    Err("确认失败".to_string())
                };
                first = false;
                async move { result }
            },
            || async { Ok(()) },
            Duration::ZERO,
        )
        .await;
        assert_eq!(result.unwrap_err(), "确认失败");
    }

    #[tokio::test]
    async fn an_app_that_remains_installed_never_allows_installation() {
        let result = tokio::time::timeout(
            Duration::from_millis(10),
            ensure_uninstalled(
                || async { Ok(true) },
                || async { Ok(()) },
                Duration::from_millis(1),
            ),
        )
        .await;
        assert!(result.is_err());
    }
}
