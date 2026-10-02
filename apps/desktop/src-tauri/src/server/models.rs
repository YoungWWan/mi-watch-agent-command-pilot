use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CommandStatus {
    Pending,
    Replied,
    Expired,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommandAction {
    pub id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub style: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommandReply {
    pub action_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub action_ids: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(alias = "custom_text")]
    pub custom_input: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Command {
    pub id: String,
    pub title: String,
    pub content: String,
    pub source: String,
    pub priority: String,
    pub status: CommandStatus,
    pub created_at: f64,
    pub expires_at: f64,
    pub is_question: bool,
    pub is_multiselect: bool,
    pub allow_custom_input: bool,
    pub actions: Vec<CommandAction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<CommandReply>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CommandCreate {
    pub title: Option<String>,
    pub content: String,
    pub source: Option<String>,
    pub priority: Option<String>,
    pub timeout_seconds: Option<u64>,
    pub is_question: Option<bool>,
    pub is_multiselect: Option<bool>,
    pub allow_custom_input: Option<bool>,
    pub actions: Option<Vec<CommandAction>>,
}

impl CommandCreate {
    pub fn validate(&self) -> Result<(), String> {
        if self.content.trim().is_empty() { return Err("指令正文不能为空".into()); }
        if !(1..=86400).contains(&self.timeout_seconds.unwrap_or(90)) { return Err("有效期应为 1–86400 秒".into()); }
        if let Some(priority) = &self.priority {
            if !["low", "normal", "high", "critical"].contains(&priority.as_str()) { return Err("无效的优先级".into()); }
        }
        if let Some(actions) = &self.actions {
            let mut ids = std::collections::HashSet::new();
            if actions.is_empty() || actions.len() > 6 { return Err("请设置 1–6 个操作按钮".into()); }
            for action in actions {
                if action.id.trim().is_empty() || action.text.trim().is_empty() || !ids.insert(&action.id) {
                    return Err("按钮名称和 ID 不能为空，ID 不能重复".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PairRequest {
    pub code: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PairResponse {
    pub status: String,
    pub token: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct WatchPendingResponse {
    pub has_command: bool,
    pub command: Option<Command>,
    pub commands: Vec<Command>,
    pub total: usize,
}
