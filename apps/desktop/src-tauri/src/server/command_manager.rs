use super::models::{Command, CommandAction, CommandCreate, CommandReply, CommandStatus};
use anyhow::{bail, Result};
use parking_lot::RwLock;
use rand::Rng;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;

pub struct CommandManager {
    commands: RwLock<HashMap<String, Command>>,
    order: RwLock<Vec<String>>,
    waiters: parking_lot::Mutex<HashMap<String, oneshot::Sender<Command>>>,
}

impl CommandManager {
    pub fn new() -> Self {
        Self {
            commands: RwLock::new(HashMap::new()),
            order: RwLock::new(Vec::new()),
            waiters: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    fn now_secs() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    pub fn create_command(&self, payload: CommandCreate) -> Command {
        let mut rng = rand::thread_rng();
        let cmd_id = format!("cmd_{:08x}", rng.gen::<u32>());
        let now = Self::now_secs();
        let timeout = payload.timeout_seconds.unwrap_or(90);
        let expires_at = now + timeout as f64;

        let actions = payload.actions.unwrap_or_else(|| {
            vec![
                CommandAction {
                    id: "confirm".to_string(),
                    text: "确定".to_string(),
                    style: Some("primary".to_string()),
                },
                CommandAction {
                    id: "reject".to_string(),
                    text: "取消".to_string(),
                    style: Some("danger".to_string()),
                },
            ]
        });

        let cmd = Command {
            id: cmd_id.clone(),
            title: payload.title.unwrap_or_else(|| "待办事项".to_string()),
            content: payload.content,
            source: payload.source.unwrap_or_else(|| "Agent".to_string()),
            priority: payload.priority.unwrap_or_else(|| "high".to_string()),
            status: CommandStatus::Pending,
            created_at: now,
            expires_at,
            is_question: payload.is_question.unwrap_or(false),
            is_multiselect: payload.is_multiselect.unwrap_or(false),
            allow_custom_input: payload.allow_custom_input.unwrap_or(false),
            actions,
            reply: None,
        };

        {
            let mut cmds = self.commands.write();
            cmds.insert(cmd_id.clone(), cmd.clone());
        }
        {
            let mut order = self.order.write();
            order.insert(0, cmd_id);
            if order.len() > 100 {
                order.truncate(100);
            }
        }

        log::info!("[Command] Created command: {} - {}", cmd.id, cmd.title);
        cmd
    }

    pub async fn create_and_wait(&self, payload: CommandCreate) -> serde_json::Value {
        let timeout = payload.timeout_seconds.unwrap_or(90);
        let cmd = self.create_command(payload);
        super::notifier::notify_command(cmd.clone());
        let cmd_id = cmd.id.clone();

        let (tx, rx) = oneshot::channel();
        {
            let mut waiters = self.waiters.lock();
            waiters.insert(cmd_id.clone(), tx);
        }
        // A desktop reply can arrive on another thread before the waiter is registered.
        if let Some(updated) = self.get_command(&cmd_id) {
            if updated.status != CommandStatus::Pending {
                if let Some(sender) = self.waiters.lock().remove(&cmd_id) { let _ = sender.send(updated); }
            }
        }

        let timeout_duration = std::time::Duration::from_secs(timeout);
        match tokio::time::timeout(timeout_duration, rx).await {
            Ok(Ok(updated_cmd)) => {
                if updated_cmd.status != CommandStatus::Replied {
                    return serde_json::json!({ "status": "timeout", "command_id": cmd_id });
                }
                log::info!("[Command] Command {} replied successfully", cmd_id);
                if let Some(ref reply) = updated_cmd.reply {
                    serde_json::json!({
                        "status": "replied",
                        "action_id": reply.action_id,
                        "action_ids": reply.action_ids.clone().unwrap_or_else(|| vec![reply.action_id.clone()]),
                        "device_id": reply.device_id,
                        "command": updated_cmd
                    })
                } else {
                    serde_json::json!({
                        "status": "replied",
                        "command": updated_cmd
                    })
                }
            }
            Ok(Err(_)) => {
                serde_json::json!({
                    "status": "error",
                    "message": "Command channel dropped"
                })
            }
            Err(_) => {
                log::warn!("[Command] Command {} timed out after {}s", cmd_id, timeout);
                {
                    let mut waiters = self.waiters.lock();
                    waiters.remove(&cmd_id);
                }
                {
                    let mut cmds = self.commands.write();
                    if let Some(c) = cmds.get_mut(&cmd_id) {
                        if c.status == CommandStatus::Pending {
                            c.status = CommandStatus::Expired;
                        }
                    }
                }
                serde_json::json!({
                    "status": "timeout",
                    "message": format!("Timed out waiting for wrist reply after {}s", timeout),
                    "command_id": cmd_id
                })
            }
        }
    }

    pub fn record_reply(&self, cmd_id: &str, mut reply: CommandReply) -> Result<Command> {
        let mut cmds = self.commands.write();
        let cmd = cmds.get_mut(cmd_id).ok_or_else(|| anyhow::anyhow!("Command not found"))?;

        if cmd.expires_at <= Self::now_secs() && cmd.status == CommandStatus::Pending {
            cmd.status = CommandStatus::Expired;
        }
        if cmd.status != CommandStatus::Pending {
            bail!("Command is not in pending state (current: {:?})", cmd.status);
        }

        let ids = reply.action_ids.clone().filter(|ids| !ids.is_empty()).unwrap_or_else(|| vec![reply.action_id.clone()]);
        let mut unique = std::collections::HashSet::new();
        if ids.iter().any(|id| !unique.insert(id) || !cmd.actions.iter().any(|action| &action.id == id)) {
            bail!("Reply contains an unknown or duplicate action");
        }
        if !ids.contains(&reply.action_id) || (!cmd.is_multiselect && ids.len() > 1) {
            bail!("Invalid action selection");
        }
        if ids.len() > 1 && ids.iter().any(|id| id == "pc_input") { bail!("Computer input cannot be combined with other actions"); }
        if reply.custom_input.as_ref().is_some_and(|text| !text.is_empty()) && !cmd.allow_custom_input {
            bail!("Custom input is disabled for this command");
        }
        reply.action_ids = Some(ids);

        cmd.status = CommandStatus::Replied;
        cmd.reply = Some(reply);
        let updated = cmd.clone();

        let mut waiters = self.waiters.lock();
        if let Some(tx) = waiters.remove(cmd_id) {
            let _ = tx.send(updated.clone());
        }

        log::info!("[Command] Recorded reply for {}: action_id={:?}", cmd_id, updated.reply.as_ref().map(|r| &r.action_id));
        Ok(updated)
    }

    pub fn get_command(&self, cmd_id: &str) -> Option<Command> {
        self.refresh_expired();
        self.commands.read().get(cmd_id).cloned()
    }

    pub fn get_all_pending(&self) -> Vec<Command> {
        let now = Self::now_secs();
        let cmds = self.commands.read();
        let order = self.order.read();

        order.iter()
            .filter_map(|id| cmds.get(id))
            .filter(|c| c.status == CommandStatus::Pending && c.expires_at > now)
            .cloned()
            .collect()
    }

    pub fn list_commands(&self, limit: usize) -> Vec<Command> {
        self.refresh_expired();
        let cmds = self.commands.read();
        let order = self.order.read();

        order.iter()
            .take(limit)
            .filter_map(|id| cmds.get(id))
            .cloned()
            .collect()
    }

    fn refresh_expired(&self) {
        let now = Self::now_secs();
        for command in self.commands.write().values_mut() {
            if command.status == CommandStatus::Pending && command.expires_at <= now {
                command.status = CommandStatus::Expired;
            }
        }
    }

    pub(crate) fn expire_command(&self, id: &str) {
        if let Some(command) = self.commands.write().get_mut(id) {
            if command.status == CommandStatus::Pending { command.status = CommandStatus::Expired; }
        }
    }

    pub fn expire_pending(&self) {
        let mut commands = self.commands.write();
        let mut waiters = self.waiters.lock();
        for command in commands.values_mut() {
            if command.status == CommandStatus::Pending {
                command.status = CommandStatus::Expired;
            }
        }
        for (id, sender) in waiters.drain() {
            if let Some(command) = commands.get(&id) { let _ = sender.send(command.clone()); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_duplicate_and_expired_replies_and_accepts_multiselect() {
        let cm = CommandManager::new();
        let payload: CommandCreate = serde_json::from_value(serde_json::json!({
            "content":"test", "timeout_seconds":60, "is_multiselect":true,
            "actions":[{"id":"a","text":"A"},{"id":"b","text":"B"}]
        })).unwrap();
        let command = cm.create_command(payload.clone());
        for ids in [vec!["bad"], vec!["a", "a"]] {
            let reply = CommandReply { action_id:ids[0].into(), action_ids:Some(ids.into_iter().map(String::from).collect()), custom_input:None, device_id:None };
            assert!(cm.record_reply(&command.id,reply).is_err());
            assert_eq!(cm.get_command(&command.id).unwrap().status,CommandStatus::Pending);
        }
        let reply = CommandReply { action_id:"a".into(), action_ids:Some(vec!["a".into(),"b".into()]), custom_input:None, device_id:Some("desktop-simulator".into()) };
        assert_eq!(cm.record_reply(&command.id,reply.clone()).unwrap().reply.unwrap().action_ids.unwrap().len(),2);
        assert!(cm.record_reply(&command.id,reply.clone()).is_err());
        let mut single = payload.clone(); single.is_multiselect = Some(false);
        let command = cm.create_command(single);
        assert!(cm.record_reply(&command.id,reply.clone()).is_err());
        cm.commands.write().get_mut(&command.id).unwrap().expires_at = 0.0;
        assert!(cm.record_reply(&command.id,reply).is_err());
        assert_eq!(cm.list_commands(50).iter().find(|c| c.id == command.id).unwrap().status,CommandStatus::Expired);
        let mut invalid = payload; invalid.actions.as_mut().unwrap()[1].id = "a".into();
        assert!(invalid.validate().is_err());
    }

    #[tokio::test]
    async fn test_command_lifecycle() {
        let cm = CommandManager::new();
        let cmd = cm.create_command(CommandCreate {
            title: Some("测试问题".to_string()),
            content: "怎样放松？".to_string(),
            source: Some("Codex".to_string()),
            priority: Some("high".to_string()),
            timeout_seconds: Some(60),
            is_question: Some(true),
            is_multiselect: Some(false),
            allow_custom_input: Some(false),
            actions: Some(vec![
                CommandAction { id: "opt_0".to_string(), text: "海边".to_string(), style: None },
                CommandAction { id: "opt_1".to_string(), text: "公园".to_string(), style: None },
            ]),
        });

        assert_eq!(cmd.status, CommandStatus::Pending);
        assert_eq!(cm.get_all_pending().len(), 1);

        // Record reply
        let updated = cm.record_reply(&cmd.id, CommandReply {
            action_id: "opt_0".to_string(),
            action_ids: Some(vec!["opt_0".to_string()]),
            device_id: Some("watch".to_string()),
            custom_input: None,
        }).expect("record reply succeeds");

        assert_eq!(updated.status, CommandStatus::Replied);
        assert_eq!(updated.reply.unwrap().action_id, "opt_0");
        assert_eq!(cm.get_all_pending().len(), 0);
    }
}
