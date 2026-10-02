export interface ServerInfo {
  status: "running" | "stopped" | "error";
  last_error: string | null;
  lan_ip: string;
  port: number;
  server_url: string;
  active_pairing_code: string | null;
  watch_paired: boolean;
  watch_connection_status?: "unpaired" | "waiting" | "online" | "offline" | "auth_failed";
  watch_last_seen_at?: number | null;
}

export function parseServicePort(value: string): number | null {
  const input = value.trim();
  if (!/^\d+$/.test(input)) return null;
  const port = Number(input);
  return Number.isInteger(port) && port >= 1 && port <= 65535 ? port : null;
}

export function watchPairingState(server: ServerInfo | null) {
  if (!server) return { label: "正在读取状态", note: "正在读取手表配对信息。", online: false, failed: false, expanded: false };
  if (server.watch_connection_status === "auth_failed") {
    return { label: "配对异常", note: "收到未通过配对验证的请求。请确认手表电脑地址与下方一致；若手表提示需重新配对，请生成新配对码。", online: false, failed: true, expanded: true };
  }
  if (!server.watch_paired) return { label: "尚未配对", note: "在手表「指令助手」中输入下方配对码。", online: false, failed: false, expanded: true };
  if (server.status !== "running") return { label: "配对已保存 · 手表离线", note: "配对信息已保留。启动指令服务后，在手表上打开「指令助手」。", online: false, failed: false, expanded: false };
  if (server.watch_connection_status === "online") return { label: "手表已连接", note: "近期已收到手表通过配对验证的请求。更换设备时可生成新配对码。", online: true, failed: false, expanded: false };
  if (server.watch_connection_status === "offline") return { label: "配对已保存 · 手表离线", note: "暂未收到手表请求，配对信息仍保留。请打开手表「指令助手」并检查局域网连接。", online: false, failed: false, expanded: false };
  return { label: "配对已保存 · 等待手表连接", note: "电脑已保存配对信息，尚未验证手表连接。请打开手表「指令助手」；若提示需重新配对，请生成新配对码。", online: false, failed: false, expanded: false };
}
export interface CommandAction { id: string; text: string; style?: string }
export interface CommandItem {
  id: string; title: string; content: string; source: string;
  priority: string; status: string; created_at: number; expires_at: number;
  is_question: boolean; is_multiselect: boolean; allow_custom_input: boolean;
  actions: CommandAction[];
  reply?: { action_id: string; action_ids?: string[]; custom_input?: string; device_id?: string };
}
export interface CommandCreate {
  title: string; content: string; source: string; priority: string;
  timeout_seconds: number; is_question: boolean; is_multiselect: boolean;
  allow_custom_input: boolean; actions: CommandAction[];
}
export function createConnectionTest(): CommandCreate {
  return {
    title: "手表连接测试",
    content: "收到这条消息后，请点击「确认连接」，完成电脑与手表的双向通信测试。",
    source: "桌面连接测试",
    priority: "normal",
    timeout_seconds: 60,
    is_question: false,
    is_multiselect: false,
    allow_custom_input: false,
    actions: [{ id: "confirm_connection", text: "确认连接", style: "primary" }],
  };
}
export function replyText(command: CommandItem): string {
  const ids = command.reply?.action_ids?.length ? command.reply.action_ids : [command.reply?.action_id];
  return command.reply?.custom_input || command.actions.filter(action => ids.includes(action.id)).map(action => action.text).join("、") || command.reply?.action_id || "";
}
