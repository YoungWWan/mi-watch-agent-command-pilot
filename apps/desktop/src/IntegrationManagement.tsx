import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Bell, RefreshCw, Shield, Trash2, ChevronDown, Check, Copy } from "lucide-react";
import type { AgentSection } from "./AgentTabs";

interface HookStatus { key: string; name: string; configured: boolean; approval_installed: boolean; completion_installed: boolean; needs_repair: boolean; message: string; config_path: string }
interface PermissionRule { id: string; app: string; cwd: string; tool_name: string; preview: string; created_at?: number }
interface NotificationSettings { ntfy_enabled: boolean; ntfy_server: string; ntfy_topic: string; ntfy_include_content: boolean }

export function IntegrationManagement({ activeSection }: { activeSection: AgentSection }) {
  const [hooks, setHooks] = useState<HookStatus[]>([]);
  const [rules, setRules] = useState<PermissionRule[]>([]);
  const [notification, setNotification] = useState<NotificationSettings>({ ntfy_enabled: false, ntfy_server: "https://ntfy.sh", ntfy_topic: "", ntfy_include_content: false });
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [expandedHooks, setExpandedHooks] = useState<Record<string, boolean>>({});
  const [copiedHook, setCopiedHook] = useState<string | null>(null);
  const refresh = async () => {
    const results = await Promise.allSettled([invoke<HookStatus[]>("hooks_get_statuses"), invoke<PermissionRule[]>("permission_rules_list")]);
    if (results[0].status === "fulfilled") setHooks(results[0].value);
    if (results[1].status === "fulfilled") setRules(results[1].value);
    const errors = results.filter(result => result.status === "rejected").map(result => String((result as PromiseRejectedResult).reason));
    if (errors.length) setError(errors.join("；"));
  };
  useEffect(() => {
    void refresh();
    invoke<NotificationSettings>("notification_get_settings").then(value => { setNotification(value); setLoaded(true); }).catch(error => setError(String(error)));
  }, []);
  const run = async (key: string, action: () => Promise<void>) => {
    if (busy) return;
    setBusy(key); setError(""); setMessage("");
    try { await action(); await refresh(); } catch (error) { setError(String(error)); }
    finally { setBusy(null); }
  };
  const saveNotification = async () => {
    await invoke("notification_save_settings", { enabled: notification.ntfy_enabled, serverUrl: notification.ntfy_server, topic: notification.ntfy_topic, includeContent: notification.ntfy_include_content });
  };
  const copyHookPath = async (hook: HookStatus) => {
    try {
      await navigator.clipboard.writeText(hook.config_path);
      setCopiedHook(hook.key);
      setError("");
      setTimeout(() => setCopiedHook(current => current === hook.key ? null : current), 2000);
    } catch {
      setError("复制失败，请重试。");
    }
  };
  return <div className="integration-management" hidden={activeSection === "mcp"}>
    {error && <p role="alert" className="inline-error workbench-feedback">{error}</p>}
    {message && <p role="status" className="positive workbench-feedback">{message}</p>}
    <section className="integration-panel" id="agent-panel-hooks" role="tabpanel" aria-labelledby="agent-tab-hooks" tabIndex={0} hidden={activeSection !== "hooks"} aria-label="审批与完成通知钩子">
      <div className="section-heading"><div><h2>审批与完成通知</h2><p className="section-note">将原生权限审批和任务完成通知发送到手表。</p></div><button className="ghost icon-button" disabled={Boolean(busy)} aria-label="刷新钩子与信任规则" onClick={() => void run("refresh", refresh)}><RefreshCw size={15} /></button></div>
      <div className="agent-list">
        {hooks.map(hook => (
          <div className="agent-item" key={hook.key}>
            <div className="agent-info">
              <h3>{hook.name}</h3>
              <div className="agent-state">
                <span className={`status-dot ${hook.configured ? "online" : hook.needs_repair ? "warning" : ""}`} />
                {hook.needs_repair ? "需要修复" : <>审批{hook.approval_installed ? "已启用" : "未启用"} · 完成通知{hook.completion_installed ? "已启用" : "未启用"}</>}
              </div>
            </div>
            <button
              className="ghost agent-details-toggle"
              aria-label={`${hook.name} 钩子配置详情`}
              aria-expanded={Boolean(expandedHooks[hook.key])}
              aria-controls={`hook-details-${hook.key}`}
              onClick={() => setExpandedHooks(previous => ({ ...previous, [hook.key]: !previous[hook.key] }))}
            >
              配置详情
              <ChevronDown size={12} className={expandedHooks[hook.key] ? "rotated" : undefined} />
            </button>
            <button
              className="secondary agent-action"
              disabled={Boolean(busy)}
              aria-label={`${hook.configured ? "停用" : hook.needs_repair ? "修复" : "启用"} ${hook.name} 钩子`}
              onClick={() => void run(hook.key, async () => {
                await invoke("hooks_toggle", { key: hook.key, enable: !hook.configured });
                setMessage(`${hook.name} 钩子${hook.configured ? "已停用" : hook.needs_repair ? "已修复" : "已启用"}，请重开 Agent 会话。`);
              })}
            >
              {busy === hook.key ? "处理中" : hook.configured ? "停用" : hook.needs_repair ? "修复" : "启用"}
            </button>
            <div className="agent-details" id={`hook-details-${hook.key}`} hidden={!expandedHooks[hook.key]}>
              <div className="agent-path">
                <code>{hook.config_path}</code>
                <button className="ghost icon-button" aria-label={`复制 ${hook.name} 钩子配置路径`} onClick={() => void copyHookPath(hook)}>
                  {copiedHook === hook.key ? <Check size={12} /> : <Copy size={12} />}
                </button>
              </div>
              {hook.message && <p className="section-note">{hook.message}</p>}
            </div>
          </div>
        ))}
      </div>
      <details className="disclosure integration-help">
        <summary><span>使用说明</span><ChevronDown size={14} /></summary>
        <p>未收到手表决定时，保留 Agent 原生审批。已有其他钩子会保留。</p>
      </details>
    </section>
    <section className="management-section" id="agent-panel-notifications" role="tabpanel" aria-labelledby="agent-tab-notifications" tabIndex={0} hidden={activeSection !== "notifications"} aria-label="手表通知设置">
      <div className="section-heading"><h2><Bell size={16} /> 手表唤醒通知</h2></div>
      <form onSubmit={event => { event.preventDefault(); void run("notification-save", async () => { await saveNotification(); setMessage("通知设置已保存。"); }); }}>
        <label className="checkbox-label"><input type="checkbox" checked={notification.ntfy_enabled} onChange={event => setNotification(previous => ({ ...previous, ntfy_enabled: event.target.checked }))} />启用 ntfy 通知</label>
        <div className="field-row"><label>通知服务<input type="url" required value={notification.ntfy_server} onChange={event => setNotification(previous => ({ ...previous, ntfy_server: event.target.value }))} /></label><label>私有主题<input value={notification.ntfy_topic} required={notification.ntfy_enabled} onChange={event => setNotification(previous => ({ ...previous, ntfy_topic: event.target.value }))} placeholder="与手机 ntfy 订阅一致" /></label></div>
        <label className="checkbox-label"><input type="checkbox" checked={notification.ntfy_include_content} onChange={event => setNotification(previous => ({ ...previous, ntfy_include_content: event.target.checked }))} />在通知中包含指令标题与正文</label>
        <p className="section-note">默认仅发送打开手表应用的提醒。手机需要订阅同一主题，并允许向手表同步通知。</p>
        <div className="control-actions"><button className="secondary" disabled={!loaded || Boolean(busy)}>保存设置</button><button type="button" className="secondary" disabled={!loaded || Boolean(busy) || !notification.ntfy_enabled} onClick={() => void run("notification-test", async () => { await saveNotification(); await invoke("notification_test"); setMessage("测试通知已发送，请检查手机和手表。是否能息屏唤醒取决于手机的通知与蓝牙设置。"); })}><Bell size={14} />{busy === "notification-test" ? "发送中…" : "测试手表唤醒"}</button></div>
      </form>
    </section>
    <section className="management-section" id="agent-panel-rules" role="tabpanel" aria-labelledby="agent-tab-rules" tabIndex={0} hidden={activeSection !== "rules"} aria-label="Codex 手表信任规则">
      <div className="section-heading"><div><h2><Shield size={16} /> Codex 手表信任规则</h2><p className="section-note">“始终允许”只匹配相同项目、工具与完整执行参数。</p></div><span className="page-meta">{rules.length} 条</span></div>
      {rules.length ? rules.map(rule => <div className="permission-rule" key={rule.id}><div><strong>{rule.tool_name}</strong><p>{rule.preview}</p><code>{rule.cwd}</code>{rule.created_at && <small className="section-note">保存于 {new Date(rule.created_at * 1000).toLocaleString("zh-CN")}</small>}</div><button className="secondary" disabled={Boolean(busy)} aria-label={`撤销 ${rule.tool_name} 规则`} onClick={() => void run(rule.id, async () => { await invoke("permission_rule_delete", { ruleId: rule.id }); setMessage("规则已撤销，下次操作将重新请求审批。"); })}><Trash2 size={13} />撤销</button></div>) : <p className="section-note">尚无已保存的规则。手表审批中的“始终允许”会出现在这里。</p>}
    </section>
  </div>;
}
