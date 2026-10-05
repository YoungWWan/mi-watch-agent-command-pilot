use super::{
    command_manager::CommandManager,
    models::{CommandCreate, CommandStatus},
};
use crate::integrations::{
    kimi::{self, Api, BridgeEvent},
    paths::ConfigPaths,
};
use anyhow::{bail, Context, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

// Keep completed identities briefly too: repeated Stop hooks must not send repeated cards.
fn claim(key: &str) -> bool {
    static JOBS: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
    let mut jobs = JOBS.get_or_init(Default::default).lock();
    let now = Instant::now();
    jobs.retain(|_, expires| *expires > now);
    if jobs.contains_key(key) || jobs.len() >= 1024 {
        return false;
    }
    jobs.insert(key.into(), now + Duration::from_secs(1800));
    true
}

fn card(title: String, content: String, approval: bool) -> CommandCreate {
    serde_json::from_value(json!({"title":title,"content":content,"source":"Kimi Code","priority":"high",
        "timeout_seconds":if approval {60} else {600},"allow_custom_input":false,
        "actions":if approval {json!([{"id":"reject","text":"拒绝","style":"danger"},{"id":"confirm","text":"允许一次","style":"primary"}])}
            else {json!([{"id":"dismiss","text":"知道了","style":"primary"}])}})).unwrap()
}

fn workspace(cwd: &str) -> &str {
    std::path::Path::new(cwd)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("任务")
}

fn notify(manager: &CommandManager, payload: CommandCreate) {
    let cmd = manager.create_command(payload);
    super::notifier::notify_command(cmd);
}

pub(crate) fn enqueue(event: BridgeEvent, manager: Arc<CommandManager>) {
    tokio::spawn(async move {
        let result = match &event {
            BridgeEvent::Approval { .. } => approval(&event, &manager).await,
            BridgeEvent::Completion { .. } => completion(&event, &manager).await,
        };
        if let Err(error) = result {
            log::warn!("Kimi watch bridge: {error}");
            // A failed approval stays pending in Kimi; this card never offers permission buttons.
            if let BridgeEvent::Approval {
                session_id,
                approval_id,
                ..
            } = &event
            {
                if claim(&format!("fallback:{session_id}:{approval_id}")) {
                    notify(&manager, card("Kimi Code 等待审批".into(),
                        "暂未连接 Kimi 审批接口，请在电脑确认本次操作。可在助手中修复 Kimi 钩子后重开会话。".into(),false));
                }
            } else if let BridgeEvent::Completion {
                session_id,
                server_id,
                turn_id,
                ..
            } = &event
            {
                if claim(&format!("fallback:{server_id}:{session_id}:{turn_id}")) {
                    notify(
                        &manager,
                        card(
                            "Kimi 回复同步失败".into(),
                            "未能同步本轮回复，请在电脑查看，并确认 Kimi Code 和指令服务保持运行。"
                                .into(),
                            false,
                        ),
                    );
                }
            }
        }
    });
}

async fn find_approval(event: &BridgeEvent, apis: &[Api]) -> Result<Option<(Api, Value)>> {
    let BridgeEvent::Approval { session_id, .. } = event else {
        bail!("审批事件无效");
    };
    // PermissionRequest is dispatched just before Kimi registers the pending interaction.
    let mut reachable = false;
    for _ in 0..5 {
        let mut matches = vec![];
        for api in apis {
            if let Ok(data) = api.approvals(session_id).await {
                reachable = true;
                for item in data["items"].as_array().into_iter().flatten() {
                    if event.matches_approval(item) {
                        matches.push((api.clone(), item.clone()));
                    }
                }
            }
        }
        match matches.len() {
            1 => return Ok(Some(matches.remove(0))),
            0 => tokio::time::sleep(Duration::from_millis(150)).await,
            _ => bail!("Kimi 审批在多个服务中出现，无法唯一定位"),
        }
    }
    if reachable {
        return Ok(None);
    }
    bail!("无法读取 Kimi 待审批请求")
}

async fn approval(event: &BridgeEvent, manager: &CommandManager) -> Result<()> {
    let apis = kimi::discover(&ConfigPaths::current()?.kimi).await?;
    let Some((api, item)) = find_approval(event, &apis).await? else {
        return Ok(());
    };
    approval_with_api(event, manager, &api, &item).await
}

async fn approval_with_api(
    event: &BridgeEvent,
    manager: &CommandManager,
    api: &Api,
    item: &Value,
) -> Result<()> {
    let BridgeEvent::Approval {
        session_id,
        approval_id,
        cwd,
        ..
    } = event
    else {
        bail!("审批事件无效");
    };
    if !claim(&format!(
        "approval:{}:{session_id}:{approval_id}",
        api.server_id
    )) {
        return Ok(());
    }
    let content = format!(
        "{}\n{}\n工具：{}\n{}",
        cwd,
        item["action"].as_str().unwrap_or("操作审批"),
        item["tool_name"].as_str().unwrap_or("tool"),
        serde_json::to_string_pretty(&item["tool_input_display"])?
    );
    let cmd = manager.create_command(card(
        format!("{} · Kimi 操作审批", workspace(cwd)),
        content,
        true,
    ));
    super::notifier::notify_command(cmd.clone());
    let result = async {
        let mut next_check = Instant::now();
        loop {
            let current = manager.get_command(&cmd.id).context("手表审批指令不存在")?;
            if current.status == CommandStatus::Replied {
                let action = current
                    .reply
                    .as_ref()
                    .map(|reply| reply.action_id.as_str())
                    .unwrap_or("");
                if !["confirm", "reject"].contains(&action) {
                    return Ok(());
                }
                let pending = api.approvals(session_id).await?;
                if !pending["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|item| event.matches_approval(item))
                {
                    return Ok(()); // Already handled on the computer; never approve a later request.
                }
                api.resolve(session_id, approval_id, action).await?;
                return Ok(());
            }
            if current.status != CommandStatus::Pending {
                return Ok(());
            }
            if Instant::now() >= next_check {
                let pending = api.approvals(session_id).await?;
                if !pending["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|item| event.matches_approval(item))
                {
                    return Ok(());
                }
                next_check = Instant::now() + Duration::from_secs(1);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
    .await;
    manager.expire_command(&cmd.id);
    result
}

async fn completion(event: &BridgeEvent, manager: &CommandManager) -> Result<()> {
    let BridgeEvent::Completion { server_id, .. } = event else {
        bail!("完成事件无效");
    };
    let api = kimi::discover(&ConfigPaths::current()?.kimi)
        .await?
        .into_iter()
        .find(|api| &api.server_id == server_id)
        .context("Kimi 服务已退出或重启")?;
    completion_with_api(event, manager, &api).await
}

async fn completion_with_api(
    event: &BridgeEvent,
    manager: &CommandManager,
    api: &Api,
) -> Result<()> {
    let BridgeEvent::Completion {
        session_id,
        server_id,
        turn_id,
        cwd,
    } = event
    else {
        bail!("完成事件无效");
    };
    if !claim(&format!("completion:{server_id}:{session_id}:{turn_id}")) {
        return Ok(());
    }
    // Stop itself is blocking. Work runs in the hub so Kimi can finish and other Stop hooks can continue it.
    let deadline = Instant::now() + Duration::from_secs(900);
    let mut missing = 0;
    while Instant::now() < deadline {
        if let Some(turn) = api.turn(session_id, *turn_id).await? {
            missing = 0;
            match turn["state"].as_str() {
                Some("completed") => {
                    let text = kimi::final_text(&turn)
                        .unwrap_or_else(|| "本轮已结束，没有可显示的文字回复。".into());
                    notify(
                        manager,
                        card(format!("{} 已完成", workspace(cwd)), text, false),
                    );
                    return Ok(());
                }
                Some("failed" | "cancelled") => return Ok(()),
                _ => {}
            }
        } else {
            missing += 1;
            if missing >= 6 {
                bail!("Kimi 未返回对应轮次，请检查接口版本");
            }
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    bail!("Kimi 本轮尚未结束，完成通知已停止等待")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        extract::{Query, State},
        http::HeaderMap,
        routing::get,
        Json, Router,
    };
    use std::sync::atomic::{AtomicBool, Ordering};

    struct Fake {
        token: String,
        server_id: String,
        item: Value,
        pending: AtomicBool,
        business_error: AtomicBool,
        decisions: Mutex<Vec<Value>>,
        turn: Mutex<Value>,
        queries: Mutex<Vec<HashMap<String, String>>>,
    }

    struct Fixture {
        home: std::path::PathBuf,
        api: Api,
        fake: Arc<Fake>,
        server: tokio::task::JoinHandle<()>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.server.abort();
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }
    fn authenticated(headers: &HeaderMap, fake: &Fake) {
        assert!(headers
            .get("authorization")
            .is_some_and(|value| value.as_bytes() == format!("Bearer {}", fake.token).as_bytes()));
    }
    async fn meta(State(fake): State<Arc<Fake>>, headers: HeaderMap) -> Json<Value> {
        authenticated(&headers, &fake);
        Json(json!({"code":0,"data":{"server_id":fake.server_id}}))
    }
    async fn approvals(
        State(fake): State<Arc<Fake>>,
        headers: HeaderMap,
        Query(query): Query<HashMap<String, String>>,
    ) -> Json<Value> {
        authenticated(&headers, &fake);
        assert_eq!(query.get("status").map(String::as_str), Some("pending"));
        Json(
            json!({"code":0,"data":{"items":if fake.pending.load(Ordering::SeqCst) {vec![fake.item.clone()]} else {vec![]}}}),
        )
    }
    async fn resolve(
        State(fake): State<Arc<Fake>>,
        headers: HeaderMap,
        Json(body): Json<Value>,
    ) -> Json<Value> {
        authenticated(&headers, &fake);
        fake.decisions.lock().push(body);
        if fake.business_error.load(Ordering::SeqCst) {
            return Json(json!({"code":40902,"data":{"resolved":false}}));
        }
        fake.pending.store(false, Ordering::SeqCst);
        Json(json!({"code":0,"data":{"resolved":true}}))
    }
    async fn transcript(
        State(fake): State<Arc<Fake>>,
        headers: HeaderMap,
        Query(query): Query<HashMap<String, String>>,
    ) -> Json<Value> {
        authenticated(&headers, &fake);
        fake.queries.lock().push(query);
        Json(json!({"code":0,"data":{"items":[fake.turn.lock().clone()],"has_more":false}}))
    }
    async fn fixture() -> Fixture {
        let home = std::env::temp_dir().join(format!("kimi-http-{:016x}", rand::random::<u64>()));
        kimi::ensure_token(&home).unwrap();
        let fake = Arc::new(Fake {
            token: kimi::read_token(&home).unwrap(),
            server_id: format!("server_{}", rand::random::<u64>()),
            item: json!({"session_id":"session_test","approval_id":"approval_test","tool_call_id":"call_test","agent_id":"main","tool_name":"Bash","turn_id":7,"action":"执行测试命令","tool_input_display":{"kind":"shell","command":"echo test"},"expires_at":"2099-01-01T00:00:00Z"}),
            pending: AtomicBool::new(true),
            business_error: AtomicBool::new(false),
            decisions: Mutex::new(vec![]),
            queries: Mutex::new(vec![]),
            turn: Mutex::new(json!({"kind":"turn","turnId":"t7","state":"running","steps":[]})),
        });
        let router = Router::new()
            .route("/api/v1/meta", get(meta))
            .route("/api/v1/sessions/session_test/approvals", get(approvals))
            .route(
                "/api/v1/sessions/session_test/approvals/approval_test",
                axum::routing::post(resolve),
            )
            .route("/api/v1/sessions/session_test/transcript", get(transcript))
            .with_state(fake.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        std::fs::create_dir_all(home.join("server/instances")).unwrap();
        std::fs::write(
            home.join("server/instances/test.json"),
            json!({"host":"127.0.0.1","port":port,"server_id":"different_registry_identity"})
                .to_string(),
        )
        .unwrap();
        let api = kimi::discover(&home).await.unwrap().remove(0);
        assert_eq!(api.server_id, fake.server_id);
        Fixture {
            home,
            api,
            fake,
            server,
        }
    }
    fn event() -> BridgeEvent {
        BridgeEvent::Approval {
            session_id: "session_test".into(),
            approval_id: "approval_test".into(),
            tool_call_id: "call_test".into(),
            agent_id: "main".into(),
            tool_name: "Bash".into(),
            turn_id: 7,
            cwd: "/workspace/test".into(),
        }
    }
    async fn pending(manager: &CommandManager) -> super::super::models::Command {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Some(cmd) = manager.get_all_pending().first() {
                    return cmd.clone();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn real_http_approval_waits_for_watch_then_submits_only_that_once_or_reject_decision() {
        for (action, decision) in [("confirm", "approved"), ("reject", "rejected")] {
            let fixture = fixture().await;
            let manager = Arc::new(CommandManager::new());
            let worker = {
                let manager = manager.clone();
                let api = fixture.api.clone();
                let item = fixture.fake.item.clone();
                tokio::spawn(
                    async move { approval_with_api(&event(), &manager, &api, &item).await },
                )
            };
            let cmd = pending(&manager).await;
            assert!(fixture.fake.decisions.lock().is_empty());
            manager
                .record_reply(
                    &cmd.id,
                    serde_json::from_value(json!({"action_id":action})).unwrap(),
                )
                .unwrap();
            tokio::time::timeout(Duration::from_secs(3), worker)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(
                *fixture.fake.decisions.lock(),
                vec![json!({"decision":decision})]
            );
            assert!(manager.get_all_pending().is_empty());
        }
    }

    #[tokio::test]
    async fn computer_resolution_and_watch_expiry_never_submit_permission() {
        for computer in [true, false] {
            let fixture = fixture().await;
            let manager = Arc::new(CommandManager::new());
            let worker = {
                let manager = manager.clone();
                let api = fixture.api.clone();
                let item = fixture.fake.item.clone();
                tokio::spawn(
                    async move { approval_with_api(&event(), &manager, &api, &item).await },
                )
            };
            let cmd = pending(&manager).await;
            if computer {
                fixture.fake.pending.store(false, Ordering::SeqCst);
            } else {
                manager.expire_command(&cmd.id);
            }
            tokio::time::timeout(Duration::from_secs(3), worker)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert!(fixture.fake.decisions.lock().is_empty());
            assert_eq!(
                manager.get_command(&cmd.id).unwrap().status,
                CommandStatus::Expired
            );
            assert!(manager
                .record_reply(
                    &cmd.id,
                    serde_json::from_value(json!({"action_id":"confirm"})).unwrap()
                )
                .is_err());
        }
    }

    #[tokio::test]
    async fn http_200_with_business_failure_is_not_a_successful_approval() {
        let fixture = fixture().await;
        fixture.fake.business_error.store(true, Ordering::SeqCst);
        assert!(fixture
            .api
            .resolve("session_test", "approval_test", "confirm")
            .await
            .is_err());
        assert!(fixture
            .api
            .resolve("session_test", "approval_test", "allow_always")
            .await
            .is_err());
        assert_eq!(fixture.fake.decisions.lock().len(), 1);
    }

    #[tokio::test]
    async fn completion_waits_for_exact_turn_and_sends_full_final_text_once() {
        let fixture = fixture().await;
        let manager = Arc::new(CommandManager::new());
        let event = BridgeEvent::Completion {
            session_id: "session_test".into(),
            server_id: fixture.api.server_id.clone(),
            turn_id: 7,
            cwd: "/workspace/test".into(),
        };
        let worker = {
            let manager = manager.clone();
            let api = fixture.api.clone();
            let event = event.clone();
            tokio::spawn(async move { completion_with_api(&event, &manager, &api).await })
        };
        tokio::time::timeout(Duration::from_secs(2), async {
            while fixture.fake.queries.lock().is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert!(manager.list_commands(10).is_empty());
        let text = "完整回复内容。".repeat(100);
        *fixture.fake.turn.lock() = json!({"kind":"turn","turnId":"t7","state":"completed","steps":[{"frames":[{"kind":"thinking","text":"private"},{"kind":"text","role":"assistant","text":text}]}]});
        tokio::time::timeout(Duration::from_secs(3), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        completion_with_api(&event, &manager, &fixture.api)
            .await
            .unwrap();
        let commands = manager.list_commands(10);
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].content, text);
        for query in fixture.fake.queries.lock().iter() {
            assert_eq!(query["agent_id"], "main");
            assert_eq!(query["before_turn"], "t8");
        }
    }

    #[tokio::test]
    async fn cancelled_or_failed_turns_never_claim_completion() {
        for state in ["cancelled", "failed"] {
            let fixture = fixture().await;
            fixture.fake.turn.lock()["state"] = json!(state);
            let event = BridgeEvent::Completion {
                session_id: "session_test".into(),
                server_id: fixture.api.server_id.clone(),
                turn_id: 7,
                cwd: String::new(),
            };
            let manager = CommandManager::new();
            completion_with_api(&event, &manager, &fixture.api)
                .await
                .unwrap();
            assert!(manager.list_commands(10).is_empty());
        }
    }
    #[test]
    fn approval_cards_offer_only_once_and_never_automatic_or_persistent_permission() {
        let payload = card("test".into(), "test".into(), true);
        assert_eq!(payload.timeout_seconds, Some(60));
        let actions = payload.actions.unwrap();
        assert_eq!(
            actions
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            ["reject", "confirm"]
        );
        assert_eq!(
            card("test".into(), "test".into(), false).actions.unwrap()[0].id,
            "dismiss"
        );
        let key = format!("test-{}", rand::random::<u64>());
        assert!(claim(&key));
        assert!(!claim(&key));
    }
}
