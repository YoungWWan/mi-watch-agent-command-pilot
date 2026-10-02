import { useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Power, RefreshCw, Send, ChevronDown, Radio } from "lucide-react";
import { createConnectionTest, parseServicePort, replyText, type CommandItem, type ServerInfo } from "./commandModel";

interface Props { server: ServerInfo | null; commands: CommandItem[]; pairingPanel: ReactNode; refresh: () => Promise<void> }
export function CommandCenter({ server, commands, pairingPanel, refresh }: Props) {
  const [sentTest, setSentTest] = useState<CommandItem | null>(null);
  const [testError, setTestError] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [filter, setFilter] = useState("all");
  const [portInput, setPortInput] = useState(() => server ? String(server.port) : "");
  const actionInFlight = useRef(false);
  useEffect(() => { setPortInput(server ? String(server.port) : ""); }, [server?.port]);
  const running = server?.status === "running";
  const pairingFailed = server?.watch_connection_status === "auth_failed";
  const testCommand = sentTest
    ? commands.find(command => command.id === sentTest.id) || sentTest
    : commands.find(command => command.source === "桌面连接测试");
  const waiting = running && !pairingFailed && testCommand?.status === "pending" && testCommand.expires_at * 1000 > Date.now();
  const confirmed = testCommand?.status === "replied"
    && testCommand.reply?.action_id === "confirm_connection"
    && Boolean(testCommand.reply.device_id)
    && testCommand.reply.device_id !== "desktop-simulator";
  const currentlyConfirmed = confirmed && running && !pairingFailed && server?.watch_connection_status === "online";
  const testResult = busy === "test" ? "正在发送测试消息…"
    : testError ? testError
    : !running ? "请先启动指令服务。"
    : pairingFailed ? "配对验证失败，请检查手表电脑地址或使用上方配对码重新配对。"
    : waiting ? "已发送，请在手表「指令助手」中点击「确认连接」（60 秒内）。"
    : testCommand?.status === "replied" ? confirmed
      ? currentlyConfirmed ? "连接正常，已收到手表确认。" : "上次测试已收到手表确认；当前连接尚未验证，请打开手表应用后重新测试。"
      : "未收到有效的手表确认，请重新测试。"
    : testCommand ? "未收到手表回复，请检查手表应用与局域网连接后重试。"
    : !server?.watch_paired ? "请先完成上方的手表配对。" : "";
  const resultClass = busy === "test" ? "section-note"
    : testError || pairingFailed ? "inline-error"
    : !running || waiting ? "section-note"
    : currentlyConfirmed ? "positive"
    : confirmed ? "section-note"
    : testCommand ? "inline-error" : "section-note";
  const run = async (key: string, action: () => Promise<void>) => {
    if (actionInFlight.current) return;
    actionInFlight.current = true;
    setBusy(key); setError(""); setMessage("");
    if (key === "test") setTestError("");
    try { await action(); } catch (error) {
      if (key === "test") setTestError(`测试发送失败：${String(error)}`);
      else setError(String(error));
    }
    finally {
      try { await refresh(); } catch (error) { setError(String(error)); }
      actionInFlight.current = false;
      setBusy(null);
    }
  };
  const testConnection = () => {
    if (!running || !server?.watch_paired || pairingFailed || waiting) return;
    void run("test", async () => {
      setSentTest(await invoke<CommandItem>("command_create", { payload: createConnectionTest() }));
    });
  };
  const savePort = () => {
    if (actionInFlight.current || !server) return;
    const port = parseServicePort(portInput);
    if (port === null) { setError("服务端口必须是 1–65535 的整数。"); setMessage(""); return; }
    if (port === server.port) return;
    void run("port", async () => {
      const info = await invoke<ServerInfo>("service_save_port", { port });
      setPortInput(String(info.port));
      setMessage(info.status === "running"
        ? `端口已保存，服务已切换至 ${info.port}；未完成指令已过期。请在手表设置中将电脑地址改为 ${info.lan_ip}:${info.port}。`
        : `端口 ${info.port} 已保存，下次启动服务时使用。请在手表设置中同步修改电脑地址。`);
    });
  };
  return <div className="page-stack">
    <section className="service-control" aria-label="指令服务控制">
      <div><h2>指令服务</h2><p className="section-note"><span className={`status-dot ${running ? "online" : ""}`} /> {running ? "运行中" : server?.status === "error" ? "启动失败" : "已停止"} · 端口 {server?.port ?? "—"}</p></div>
      <div className="control-actions">
        <button className={running ? "secondary" : "primary"} disabled={Boolean(busy) || !server} onClick={() => void run("service", async () => { await invoke(running ? "service_stop" : "service_start"); setMessage(running ? "服务已停止，未完成指令已过期。" : "服务已启动。"); })}><Power size={14} />{busy === "service" ? "处理中…" : running ? "停止服务" : "启动服务"}</button>
        <button className="secondary" disabled={Boolean(busy) || !running} onClick={() => void run("restart", async () => { await invoke("service_restart"); setMessage("服务已重启，未完成指令已过期。"); })}><RefreshCw size={14} />重启</button>
      </div>
      <form className="service-port-form" aria-label="服务端口设置" noValidate onSubmit={event => { event.preventDefault(); savePort(); }}>
        <label htmlFor="service-port">服务端口</label>
        <input id="service-port" aria-label="服务端口" aria-describedby="service-port-note" type="number" inputMode="numeric" min={1} max={65535} step={1} required value={portInput} disabled={Boolean(busy) || !server} onChange={event => { setPortInput(event.target.value); setError(""); setMessage(""); }} />
        <button className="secondary" type="submit" disabled={Boolean(busy) || !server || parseServicePort(portInput) === server.port}>{busy === "port" ? "保存中…" : running ? "保存并应用" : "保存端口"}</button>
        <p className="section-note" id="service-port-note">可填写 1–65535。{running ? "保存后服务切换到新端口，未完成指令会过期。" : "保存后下次启动生效。"}手表需同步修改电脑地址中的端口。</p>
      </form>
      {server?.last_error && <p className="inline-error" role="alert">{server.last_error}</p>}
      <p className="section-note service-description">停止后手表和 Agent 暂时无法收发指令；配对信息与历史记录保留。</p>
    </section>
    {error && <p className="inline-error workbench-feedback" role="alert">{error}</p>}
    {message && <p className="positive workbench-feedback" role="status">{message}</p>}
    {pairingPanel}
    <section className="connection-test" aria-label="手表连接测试">
      <div><h2>连接测试</h2><p className="section-note">发送一条测试消息，在手表上确认即可验证双向通信。</p></div>
      <button className="primary" disabled={!running || !server?.watch_paired || pairingFailed || Boolean(busy) || waiting} onClick={testConnection}>
        {busy === "test" || waiting ? <RefreshCw size={14} className="spinning" /> : <Send size={14} />}
        {busy === "test" ? "发送中…" : waiting ? "等待手表确认…" : "测试手表连接"}
      </button>
      <p className={`connection-test-result ${resultClass}`} role="status">{testResult}</p>
    </section>
    <section className="command-section" aria-label="指令记录">
      <div className="section-heading"><h2>指令记录</h2><div className="control-actions"><select aria-label="筛选指令状态" value={filter} onChange={event => setFilter(event.target.value)}><option value="all">全部</option><option value="pending">待回复</option><option value="replied">已回复</option><option value="expired">已过期</option></select><button className="ghost icon-button" onClick={() => void refresh()} aria-label="刷新指令记录"><RefreshCw size={14} /></button></div></div>
      <div className="command-list">{commands.filter(command => filter === "all" || command.status === filter).map(command => <details className="command-item" key={command.id}>
        <summary><span className="command-summary"><strong>{command.title}</strong><span className="command-meta">{command.source} · {new Date(command.created_at * 1000).toLocaleString("zh-CN")}</span></span><span className={`command-status ${command.status === "pending" ? "pending" : command.status === "replied" ? "positive" : ""}`}>{command.status === "pending" ? "待回复" : command.status === "replied" ? "已回复" : "已过期"}</span><ChevronDown size={14} /></summary>
        <div className="command-body"><p>{command.content}</p><div className="command-options">{command.actions.map(action => <span key={action.id}>{action.text}</span>)}</div>{command.reply && <p className="reply-result">{command.reply.device_id === "desktop-simulator" ? "桌面模拟回复" : "手表回复"}：{replyText(command)}</p>}<small className="command-id">{command.id} · 有效至 {new Date(command.expires_at * 1000).toLocaleTimeString("zh-CN")}</small></div>
      </details>)}</div>
      {!commands.some(command => filter === "all" || command.status === filter) && <div className="quiet-empty"><Radio size={24} /><p>{filter === "all" ? "尚无指令，可点击上方按钮测试手表连接。" : "没有符合筛选条件的指令。"}</p></div>}
    </section>
  </div>;
}
