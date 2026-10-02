use crate::integrations::permission_rules;
use anyhow::Result;
use serde_json::{json, Value};
use std::io::{self, Read};

fn server_url() -> String {
    format!(
        "http://127.0.0.1:{}",
        crate::storage::settings::load_settings().server_port
    )
}

fn app_name() -> String {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|arg| arg == "--app")
        .and_then(|pos| args.get(pos + 1))
        .cloned()
        .unwrap_or("Codex".into())
}

fn claude_update(data: &Value) -> Option<Value> {
    data["permission_suggestions"]
        .as_array()?
        .iter()
        .filter(|suggestion| {
            suggestion["type"] == "addRules"
                && suggestion["behavior"] == "allow"
                && ["localSettings", "userSettings", "projectSettings"]
                    .contains(&suggestion["destination"].as_str().unwrap_or(""))
                && suggestion["rules"].as_array().is_some_and(|rules| {
                    !rules.is_empty()
                        && rules.iter().all(|rule| {
                            rule["ruleContent"]
                                .as_str()
                                .is_some_and(|text| !text.trim().is_empty())
                        })
                })
        })
        .min_by_key(|suggestion| match suggestion["destination"].as_str() {
            Some("localSettings") => 0,
            Some("userSettings") => 1,
            _ => 2,
        })
        .cloned()
}

fn permission_output(app: &str, allow: bool, updates: Option<Value>) -> Value {
    if app == "Cursor" {
        return json!({"permission": if allow {"allow"} else {"deny"}});
    }
    let mut decision = json!({"behavior":if allow {"allow"} else {"deny"}});
    if !allow {
        decision["message"] = json!("用户在手表上拒绝了此操作");
    }
    if let Some(update) = updates {
        decision["updatedPermissions"] = json!([update]);
    }
    json!({"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":decision}})
}

fn permission_fallback(app: &str) -> Value {
    if app == "Cursor" {
        json!({"permission":"ask"})
    } else {
        json!({})
    }
}

async fn permission(data: &Value, app: &str) -> Result<Value> {
    let event = data["hook_event_name"]
        .as_str()
        .unwrap_or("PermissionRequest");
    if app != "Cursor" && event != "PermissionRequest" {
        return Ok(json!({}));
    }
    let tool = data
        .get("tool_name")
        .or_else(|| data.get("tool"))
        .and_then(Value::as_str)
        .unwrap_or(if app == "Cursor" { "shell" } else { "tool" });
    // Questions remain MCP interactions; this handler only processes native permissions.
    if [
        "request_user_input",
        "request_user_input_async",
        "AskUserQuestion",
    ]
    .contains(&tool)
    {
        return Ok(json!({}));
    }
    let rule = if app == "Codex" {
        permission_rules::rule_for(data)
    } else {
        None
    };
    if rule.as_ref().is_some_and(permission_rules::is_trusted) {
        return Ok(permission_output(app, true, None));
    }
    let update = if app == "Claude Code" {
        claude_update(data)
    } else {
        None
    };
    let details = data
        .get("tool_input")
        .or_else(|| data.get("input"))
        .or_else(|| data.get("command"))
        .cloned()
        .unwrap_or(json!({}));
    let mut actions = vec![
        json!({"id":"reject","text":"拒绝","style":"danger"}),
        json!({"id":"confirm","text":"允许一次","style":"primary"}),
    ];
    if rule.is_some() || update.is_some() {
        actions.push(json!({"id":"allow_always","text":"始终允许","style":"default"}));
    }
    let payload = json!({"title":format!("{app} 操作审批"), "content":format!("{}\n工具：{}\n{}", data["cwd"].as_str().unwrap_or(""),tool,serde_json::to_string_pretty(&details)?),
        "source":app,"priority":"high","timeout_seconds":60,"allow_custom_input":false,"actions":actions});
    let response = reqwest::Client::new()
        .post(format!("{}/api/v1/commands/wait-reply", server_url()))
        .json(&payload)
        .timeout(std::time::Duration::from_secs(65))
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    if response["status"] != "replied" {
        return Ok(permission_fallback(app));
    }
    match response["action_id"].as_str() {
        Some("reject") => Ok(permission_output(app, false, None)),
        Some("confirm") => Ok(permission_output(app, true, None)),
        Some("allow_always") if rule.is_some() || update.is_some() => {
            if let Some(rule) = rule {
                permission_rules::remember(rule)?;
            }
            Ok(permission_output(app, true, update))
        }
        _ => Ok(permission_fallback(app)),
    }
}

pub async fn run_hook(hook_type: &str) -> Result<()> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let data: Value = serde_json::from_str(&stdin).unwrap_or(json!({}));
    let app = app_name();
    let output = match hook_type {
        "permission" => permission(&data, &app).await.unwrap_or_else(|error| {
            eprintln!("Watch permission fallback: {error}");
            permission_fallback(&app)
        }),
        "stop" => {
            let cwd = data["cwd"].as_str().unwrap_or("");
            let workspace = std::path::Path::new(cwd)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("任务");
            let message = data["last_assistant_message"]
                .as_str()
                .unwrap_or("AI 已完成本轮任务，请查看结果。");
            let message: String = message.chars().take(300).collect();
            let payload = json!({"title":format!("{workspace} 已完成"),"content":message,"source":app,"priority":"high","timeout_seconds":600,"allow_custom_input":false,
                "actions":[{"id":"dismiss","text":"知道了","style":"primary"}]});
            let _ = reqwest::Client::new()
                .post(format!("{}/api/v1/commands", server_url()))
                .json(&payload)
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await;
            json!({})
        }
        _ => json!({}),
    };
    println!("{output}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_permission_outputs_and_claude_suggestions() {
        assert_eq!(
            permission_output("Codex", true, None)["hookSpecificOutput"]["decision"]["behavior"],
            "allow"
        );
        assert_eq!(
            permission_output("Cursor", false, None)["permission"],
            "deny"
        );
        assert_eq!(permission_fallback("Cursor")["permission"], "ask");
        assert_eq!(permission_fallback("Codex"), json!({}));
        let valid = json!({"type":"addRules","behavior":"allow","destination":"localSettings","rules":[{"ruleContent":"Bash(echo test)"}]});
        let data = json!({"permission_suggestions":[{"type":"setMode","mode":"bypassPermissions"}, valid]});
        assert_eq!(claude_update(&data), Some(valid));
    }
}
