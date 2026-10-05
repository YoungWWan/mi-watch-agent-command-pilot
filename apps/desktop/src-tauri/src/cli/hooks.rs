use crate::integrations::permission_rules;
use crate::integrations::{antigravity, kimi, paths::ConfigPaths};
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
    if app == "Antigravity" {
        return json!({"decision": if allow {"allow"} else {"deny"}, "reason": if allow {"用户已在手表允许此操作"} else {"用户在手表上拒绝了此操作"}});
    }
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
    } else if app == "Antigravity" {
        json!({"decision":"ask"})
    } else {
        json!({})
    }
}

fn stop_output(app: &str) -> Value {
    if app == "Antigravity" {
        json!({"decision":"stop"})
    } else {
        json!({})
    }
}

fn completion_message(data: &Value, app: &str) -> String {
    if app == "Antigravity" {
        match antigravity::final_message(data) {
            Ok(Some(message)) => return message,
            Err(error) => eprintln!("Antigravity watch reply: {error}"),
            _ => {}
        }
    }
    data["last_assistant_message"]
        .as_str()
        .filter(|message| !message.trim().is_empty())
        .unwrap_or("AI 已完成本轮任务，请查看结果。")
        .chars()
        .take(300)
        .collect()
}

fn normalize_input(data: &Value, app: &str) -> Value {
    if app != "Antigravity" {
        return data.clone();
    }
    let mut normalized = data.clone();
    if !normalized.is_object() {
        return json!({});
    }
    normalized["tool_name"] = data["toolCall"]["name"].clone();
    normalized["tool_input"] = data["toolCall"]["args"].clone();
    normalized["cwd"] = data["toolCall"]["args"]["Cwd"]
        .as_str()
        .or_else(|| data["workspacePaths"][0].as_str())
        .map(Value::from)
        .unwrap_or(Value::Null);
    normalized
}

async fn permission(data: &Value, app: &str) -> Result<Value> {
    // Kimi PermissionRequest is observation-only: never emit an approval decision.
    if app == "Kimi Code" {
        return Ok(json!({}));
    }
    if app == "Antigravity" && !data["tool_name"].is_string() {
        return Ok(permission_fallback(app));
    }
    let event = data["hook_event_name"]
        .as_str()
        .unwrap_or("PermissionRequest");
    if app != "Cursor" && app != "Antigravity" && event != "PermissionRequest" {
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
        "ask_question",
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
    let update = if app == "Claude Code" || app == "ZCode" {
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

async fn kimi_hook(data: &Value, mode: &str) -> Result<()> {
    let event = if mode == "stop" {
        if !should_notify(data, "Kimi Code", mode) {
            return Ok(());
        }
        Some(
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                kimi::completion_event(&ConfigPaths::current()?.kimi, data),
            )
            .await??,
        )
    } else {
        kimi::BridgeEvent::approval(data)
    };
    let Some(event) = event else {
        return Ok(());
    };
    let response = reqwest::Client::builder()
        .no_proxy()
        .build()?
        .post(format!("{}/api/v1/integrations/kimi/hook", server_url()))
        .json(&event)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await?
        .error_for_status()?;
    if response.json::<Value>().await?["status"] != "accepted" {
        anyhow::bail!("指令服务未接收 Kimi 事件，请更新并启动桌面助手");
    }
    Ok(())
}

pub async fn run_hook(hook_type: &str) -> Result<()> {
    let mut stdin = String::new();
    io::stdin().read_to_string(&mut stdin)?;
    let app = app_name();
    let data: Value = match serde_json::from_str::<Value>(&stdin) {
        Ok(data) if data.is_object() => data,
        _ => {
            let output = if hook_type == "permission" {
                permission_fallback(&app)
            } else {
                stop_output(&app)
            };
            println!("{output}");
            return Ok(());
        }
    };
    if app == "Antigravity" && hook_type == "stop" {
        antigravity::record_stop(&data, "received");
    }
    let data = normalize_input(&data, &app);
    if app == "Kimi Code" && ["permission", "attention", "stop"].contains(&hook_type) {
        if let Err(error) = kimi_hook(&data, hook_type).await {
            eprintln!("Kimi watch bridge: {error}");
            if hook_type == "stop" && should_notify(&data, &app, hook_type) {
                let _ = reqwest::Client::builder().no_proxy().build()?
                    .post(format!("{}/api/v1/commands",server_url()))
                    .json(&json!({"title":"Kimi 回复同步失败","content":"未能读取本轮回复，请在电脑查看结果，并在助手中修复 Kimi 钩子后重开会话。","source":app,"priority":"high","timeout_seconds":600,"allow_custom_input":false,"actions":[{"id":"dismiss","text":"知道了","style":"primary"}]}))
                    .timeout(std::time::Duration::from_secs(2)).send().await;
            }
        }
        // The hook stays observation-only; the hub submits only actual user decisions over REST.
        println!("{{}}");
        return Ok(());
    }
    let output = match hook_type {
        "permission" => permission(&data, &app).await.unwrap_or_else(|error| {
            eprintln!("Watch permission fallback: {error}");
            permission_fallback(&app)
        }),
        "stop" | "attention" => {
            if !should_notify(&data, &app, hook_type) {
                if app == "Antigravity" {
                    antigravity::record_stop(&data, "skipped");
                    eprintln!("Antigravity watch Stop skipped: fullyIdle={}, terminationReason={}, has_error={}",
                        data["fullyIdle"], data["terminationReason"],
                        data["error"].as_str().is_some_and(|error| !error.is_empty()));
                }
                println!("{}", stop_output(&app));
                return Ok(());
            }
            let cwd = data["cwd"].as_str().unwrap_or("");
            let workspace = std::path::Path::new(cwd)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("任务");
            let message = if hook_type == "attention" {
                "AI 正在等待权限确认，请回到电脑审批后继续任务。".into()
            } else {
                completion_message(&data, &app)
            };
            let title = if hook_type == "attention" {
                format!("{workspace} 等待审批")
            } else {
                format!("{workspace} 已完成")
            };
            let payload = json!({"title":title,"content":message,"source":app,"priority":"high","timeout_seconds":600,"allow_custom_input":false,
                "actions":[{"id":"dismiss","text":"知道了","style":"primary"}]});
            let result = reqwest::Client::builder()
                .no_proxy()
                .build()?
                .post(format!("{}/api/v1/commands", server_url()))
                .json(&payload)
                .timeout(std::time::Duration::from_secs(2))
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            if app == "Antigravity" {
                antigravity::record_stop(
                    &data,
                    if result.is_ok() {
                        "sent"
                    } else {
                        "send_failed"
                    },
                );
            }
            if let Err(error) = result {
                eprintln!("Watch completion notification failed: {error}");
            }
            stop_output(&app)
        }
        _ => json!({}),
    };
    println!("{output}");
    Ok(())
}

fn should_notify(data: &Value, app: &str, mode: &str) -> bool {
    if mode == "attention" {
        return app == "Kimi Code" && data["hook_event_name"] == "PermissionRequest";
    }
    if data["stop_hook_active"] == true {
        return false;
    }
    if app == "Antigravity" {
        return data["fullyIdle"] == true
            && data["terminationReason"] == "model_stop"
            && !data["error"]
                .as_str()
                .is_some_and(|error| !error.is_empty());
    }
    if app == "Kimi Code" || app == "ZCode" {
        return data["hook_event_name"] == "Stop";
    }
    true
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

    #[test]
    fn antigravity_payload_and_decisions_follow_its_protocol() {
        assert_eq!(stop_output("Antigravity"), json!({"decision":"stop"}));
        assert_eq!(stop_output("Codex"), json!({}));
        let input = json!({"toolCall":{"name":"run_command","args":{"CommandLine":"npm test","Cwd":"/workspace/actual"}},"workspacePaths":["/workspace/fallback"]});
        let data = normalize_input(&input, "Antigravity");
        assert_eq!(data["tool_name"], "run_command");
        assert_eq!(data["tool_input"], input["toolCall"]["args"]);
        assert_eq!(data["cwd"], "/workspace/actual");
        assert_eq!(
            normalize_input(
                &json!({"workspacePaths":["/workspace/fallback"]}),
                "Antigravity"
            )["cwd"],
            "/workspace/fallback"
        );
        assert_eq!(
            permission_output("Antigravity", true, None)["decision"],
            "allow"
        );
        assert_eq!(
            permission_output("Antigravity", false, None)["decision"],
            "deny"
        );
        assert_eq!(
            permission_fallback("Antigravity"),
            json!({"decision":"ask"})
        );
        assert_eq!(
            permission_output("ZCode", true, None)["hookSpecificOutput"]["decision"]["behavior"],
            "allow"
        );
        assert_eq!(
            completion_message(&json!({}), "Antigravity"),
            "AI 已完成本轮任务，请查看结果。"
        );
    }

    #[tokio::test]
    async fn kimi_observation_hook_never_returns_a_permission_decision() {
        let data = json!({"hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"npm test"}});
        assert_eq!(permission(&data, "Kimi Code").await.unwrap(), json!({}));
    }

    #[test]
    fn notifications_distinguish_completed_turns_from_errors_and_pending_background_work() {
        let completed = json!({"fullyIdle":true,"terminationReason":"model_stop"});
        assert!(should_notify(&completed, "Antigravity", "stop"));
        for change in [
            json!({"fullyIdle":false}),
            json!({"terminationReason":"error"}),
            json!({"error":"model failed"}),
        ] {
            let mut data = completed.clone();
            data.as_object_mut()
                .unwrap()
                .extend(change.as_object().unwrap().clone());
            assert!(!should_notify(&data, "Antigravity", "stop"));
        }
        assert!(!should_notify(&json!({}), "Antigravity", "stop"));
        assert!(should_notify(
            &json!({"hook_event_name":"PermissionRequest"}),
            "Kimi Code",
            "attention"
        ));
        assert!(!should_notify(
            &json!({"hook_event_name":"Stop"}),
            "Kimi Code",
            "attention"
        ));
        assert!(!should_notify(
            &json!({"hook_event_name":"Stop","stop_hook_active":true}),
            "ZCode",
            "stop"
        ));
    }
}
