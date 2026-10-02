use crate::storage::settings::load_settings;
use super::models::Command;

pub async fn send_push(command: Option<&Command>) -> Result<(), String> {
    let settings = load_settings();
    if !settings.ntfy_enabled || settings.ntfy_topic.trim().is_empty() {
        return Err("请先设置 ntfy 主题并启用通知".into());
    }
    let include = settings.ntfy_include_content;
    let payload = serde_json::json!({
        "topic": settings.ntfy_topic,
        "title": if include { command.map(|c| c.title.as_str()).unwrap_or("手表唤醒测试") } else { "手表指令待处理" },
        "message": if include { command.map(|c| c.content.as_str()).unwrap_or("请打开手表上的指令助手查看详情") } else { "请打开手表上的指令助手查看详情" },
        "priority": 5,
        "tags": ["watch"],
    });
    reqwest::Client::new().post(settings.ntfy_server.trim_end_matches('/')).json(&payload)
        .timeout(std::time::Duration::from_secs(5)).send().await
        .map_err(|e| format!("通知发送失败：{e}"))?
        .error_for_status().map_err(|e| format!("通知服务拒绝请求：{e}"))?;
    Ok(())
}

pub fn notify_command(command: Command) {
    #[cfg(test)]
    let _ = command; // Automated command tests never notify the user's real topic.
    #[cfg(not(test))]
    {
        if !load_settings().ntfy_enabled { return; }
        tokio::spawn(async move {
            if let Err(error) = send_push(Some(&command)).await { log::warn!("[ntfy] {error}"); }
        });
    }
}
