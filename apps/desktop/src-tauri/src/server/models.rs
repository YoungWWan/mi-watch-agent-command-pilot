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
            // A question can have six choices plus the separate computer fallback.
            let has_computer_fallback = self.is_question == Some(true)
                && actions.iter().any(|action| action.id == "pc_input");
            let max_actions = if has_computer_fallback { 7 } else { 6 };
            if actions.is_empty() || actions.len() > max_actions {
                return Err("请设置 1–6 个操作按钮，选择题可额外添加一个电脑输入按钮".into());
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn question_with_actions(ids: &[&str], is_question: bool) -> CommandCreate {
        serde_json::from_value(serde_json::json!({
            "content": "选择水果", "is_question": is_question,
            "actions": ids.iter().map(|id| serde_json::json!({"id": id, "text": id})).collect::<Vec<_>>()
        })).unwrap()
    }

    #[test]
    fn six_choices_allow_one_separate_computer_fallback() {
        let ids = ["opt_0", "opt_1", "opt_2", "opt_3", "opt_4", "opt_5", "pc_input"];
        assert!(question_with_actions(&ids, true).validate().is_ok());
        assert!(question_with_actions(&ids, false).validate().is_err());
        assert!(question_with_actions(&ids[..6], true).validate().is_ok());
        assert!(question_with_actions(&["opt_0", "opt_1", "opt_2", "opt_3", "opt_4", "opt_5", "opt_6"], true).validate().is_err());
        assert!(question_with_actions(&["opt_0", "opt_1", "opt_2", "opt_3", "opt_4", "opt_5", "opt_6", "pc_input"], true).validate().is_err());
        assert!(question_with_actions(&["opt_0", "opt_1", "opt_2", "opt_3", "opt_4", "pc_input", "pc_input"], true).validate().is_err());
    }
}
