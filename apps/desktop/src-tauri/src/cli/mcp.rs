use anyhow::Result;
use serde_json::Value;
use std::io::{self, BufRead, Write};

fn server_url() -> String { format!("http://127.0.0.1:{}", crate::storage::settings::load_settings().server_port) }

fn get_agent_name() -> String {
    std::env::var("REDMI_WATCH_AGENT").unwrap_or_else(|_| "Codex".to_string())
}

fn send_response(resp: Value) {
    let mut stdout = io::stdout().lock();
    let _ = writeln!(stdout, "{}", resp);
    let _ = stdout.flush();
}

fn make_text_result(data: Value, is_error: bool) -> Value {
    serde_json::json!({
        "content": [
            {
                "type": "text",
                "text": data.to_string()
            }
        ],
        "isError": is_error
    })
}

fn parse_server_reply(status: reqwest::StatusCode, body: &[u8]) -> std::result::Result<Value, String> {
    let parsed = serde_json::from_slice::<Value>(body);
    if !status.is_success() {
        let message = parsed.as_ref().ok()
            .and_then(|value| value.get("message").or_else(|| value.get("error")))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                let text = String::from_utf8_lossy(body);
                let preview: String = text.trim().chars().take(240).collect();
                if preview.is_empty() { "empty response body".into() } else { preview }
            });
        return Err(format!("Watch Command Hub returned HTTP {status}: {message}"));
    }
    parsed.map_err(|error| format!("Invalid JSON from server (HTTP {status}): {error}"))
}

async fn handle_ask_question(arguments: &Value) -> Value {
    let question = match arguments.get("question").and_then(|v| v.as_str()) {
        Some(q) if !q.trim().is_empty() => q.trim(),
        _ => return make_text_result(serde_json::json!({"status": "error", "message": "question is required"}), true),
    };

    let options = match arguments.get("options").and_then(|v| v.as_array()) {
        Some(opts) if opts.len() >= 2 && opts.len() <= 6 => opts,
        _ => return make_text_result(serde_json::json!({"status": "error", "message": "options must contain 2 to 6 items"}), true),
    };

    let mut option_map = std::collections::HashMap::new();
    let mut actions = Vec::new();

    for (i, opt) in options.iter().enumerate() {
        let label = opt.as_str().unwrap_or_default();
        if label.trim().is_empty() {
            return make_text_result(serde_json::json!({"status": "error", "message": "option cannot be empty"}), true);
        }
        let action_id = format!("opt_{}", i);
        option_map.insert(action_id.clone(), label.to_string());
        actions.push(serde_json::json!({
            "id": action_id,
            "text": label,
            "style": "default"
        }));
    }

    actions.push(serde_json::json!({
        "id": "pc_input",
        "text": "在电脑输入",
        "style": "danger"
    }));

    let title = arguments.get("title").and_then(|v| v.as_str()).unwrap_or("选择回答");
    let is_multiselect = arguments.get("is_multiselect").and_then(|v| v.as_bool()).unwrap_or(false);
    let timeout = arguments.get("timeout_seconds").and_then(|v| v.as_u64()).unwrap_or(90).clamp(10, 300);
    let agent_name = get_agent_name();

    let payload = serde_json::json!({
        "title": format!("[{}] {}", agent_name, title),
        "content": question,
        "source": agent_name,
        "priority": "high",
        "timeout_seconds": timeout,
        "is_question": true,
        "is_multiselect": is_multiselect,
        "allow_custom_input": false,
        "actions": actions
    });

    let client = match reqwest::Client::builder().no_proxy().build() {
        Ok(client) => client,
        Err(error) => return make_text_result(serde_json::json!({"status": "error", "message": format!("Watch Command Hub unavailable: {error}")}), true),
    };
    let res = client
        .post(format!("{}/api/v1/commands/wait-reply", server_url()))
        .json(&payload)
        .timeout(std::time::Duration::from_secs(timeout + 5))
        .send()
        .await;

    let reply: Value = match res {
        Ok(r) => {
            let status = r.status();
            let body = match r.bytes().await {
                Ok(body) => body,
                Err(error) => return make_text_result(serde_json::json!({"status": "error", "message": format!("Could not read Watch Command Hub response (HTTP {status}): {error}")}), true),
            };
            match parse_server_reply(status, &body) {
                Ok(reply) => reply,
                Err(message) => return make_text_result(serde_json::json!({"status": "error", "message": message}), true),
            }
        },
        Err(e) => return make_text_result(serde_json::json!({"status": "error", "message": format!("Watch Command Hub unavailable: {e}")}), true),
    };

    if reply.get("status").and_then(|v| v.as_str()) != Some("replied") {
        return make_text_result(serde_json::json!({"status": "timeout", "question": question}), true);
    }

    let selected_ids = if let Some(ids) = reply.get("action_ids").and_then(|v| v.as_array()) {
        ids.iter().filter_map(|v| v.as_str()).map(|s| s.to_string()).collect::<Vec<_>>()
    } else if let Some(id) = reply.get("action_id").and_then(|v| v.as_str()) {
        vec![id.to_string()]
    } else {
        Vec::new()
    };

    if selected_ids.contains(&"pc_input".to_string()) {
        return make_text_result(serde_json::json!({
            "status": "computer_input",
            "question": question
        }), false);
    }

    let selected_options: Vec<String> = selected_ids
        .iter()
        .filter_map(|id| option_map.get(id).cloned())
        .collect();

    if selected_options.is_empty() {
        return make_text_result(serde_json::json!({"status": "error", "message": "No valid options selected"}), true);
    }

    make_text_result(serde_json::json!({
        "status": "answered",
        "question": question,
        "selected_options": selected_options
    }), false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn http_errors_report_status_and_server_validation_message() {
        let body = serde_json::json!({"status":"error","message":"请设置 1–6 个操作按钮"}).to_string();
        let error = parse_server_reply(StatusCode::BAD_REQUEST, body.as_bytes()).unwrap_err();
        assert!(error.contains("HTTP 400 Bad Request"));
        assert!(error.contains("请设置 1–6 个操作按钮"));
        assert!(!error.contains("Invalid JSON"));
    }

    #[test]
    fn empty_and_non_json_http_errors_keep_the_http_status() {
        assert!(parse_server_reply(StatusCode::BAD_REQUEST, b"").unwrap_err().contains("HTTP 400 Bad Request: empty response body"));
        let error = parse_server_reply(StatusCode::BAD_GATEWAY, b"upstream unavailable").unwrap_err();
        assert!(error.contains("HTTP 502 Bad Gateway: upstream unavailable"));
        let long_body = "错".repeat(300);
        let error = parse_server_reply(StatusCode::BAD_GATEWAY, long_body.as_bytes()).unwrap_err();
        assert_eq!(error.matches('错').count(), 240);
    }

    #[test]
    fn successful_replies_still_require_valid_json() {
        let reply = parse_server_reply(StatusCode::OK, br#"{"status":"replied","action_ids":["opt_0","opt_5"]}"#).unwrap();
        assert_eq!(reply["action_ids"], serde_json::json!(["opt_0", "opt_5"]));
        assert!(parse_server_reply(StatusCode::OK, b"").unwrap_err().contains("Invalid JSON from server (HTTP 200 OK)"));
    }
}

pub async fn run_mcp_server() -> Result<()> {
    let stdin = io::stdin();
    let reader = stdin.lock();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let req_id = req.get("id").cloned();
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or_default();
        let params = req.get("params").cloned().unwrap_or_else(|| serde_json::json!({}));

        match method {
            "initialize" => {
                send_response(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "protocolVersion": "2024-11-05",
                        "serverInfo": {
                            "name": "agent-command-pilot",
                            "version": env!("CARGO_PKG_VERSION")
                        },
                        "capabilities": {
                            "tools": {}
                        }
                    }
                }));
            }
            "notifications/initialized" => {
                // No response needed for notification
            }
            "tools/list" => {
                send_response(serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": req_id,
                    "result": {
                        "tools": [
                            {
                                "name": "ask_watch_question",
                                "description": "Send a choice question to the user's connected Xiaomi wearable and wait for their answer. Use this when you have 2-6 distinct choices for the user to pick from.",
                                "inputSchema": {
                                    "type": "object",
                                    "properties": {
                                        "question": {
                                            "type": "string",
                                            "description": "The question to ask the user on their smartwatch."
                                        },
                                        "options": {
                                            "type": "array",
                                            "items": {"type": "string"},
                                            "description": "Between 2 and 6 concrete choices for the user to choose from."
                                        },
                                        "title": {
                                            "type": "string",
                                            "description": "Short category or title shown on watch header (max 16 chars). Defaults to '选择回答'."
                                        },
                                        "is_multiselect": {
                                            "type": "boolean",
                                            "description": "Whether the user can select multiple options. Defaults to false."
                                        },
                                        "timeout_seconds": {
                                            "type": "integer",
                                            "description": "Timeout in seconds waiting for the watch answer (10-300). Defaults to 90."
                                        }
                                    },
                                    "required": ["question", "options"]
                                }
                            }
                        ]
                    }
                }));
            }
            "tools/call" => {
                let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                let arguments = params.get("arguments").cloned().unwrap_or_else(|| serde_json::json!({}));

                if tool_name == "ask_watch_question" {
                    let tool_result = handle_ask_question(&arguments).await;
                    send_response(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "result": tool_result
                    }));
                } else {
                    send_response(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": req_id,
                        "error": {
                            "code": -32601,
                            "message": format!("Tool not found: {}", tool_name)
                        }
                    }));
                }
            }
            _ => {
                if let Some(id) = req_id {
                    send_response(serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32601,
                            "message": format!("Method not found: {}", method)
                        }
                    }));
                }
            }
        }
    }

    Ok(())
}
