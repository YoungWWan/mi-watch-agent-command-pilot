export interface DesktopUpdateInfo {
  version: string;
  enabled: boolean;
  message: string;
}

export interface AvailableDesktopUpdate {
  version: string;
  notes: string;
  date: string | null;
}

export type UpdatePhase = "idle" | "checking" | "available" | "downloading" | "verifying" | "installing" | "restarting" | "error";

export interface DesktopUpdateProgress {
  phase: "downloading" | "verifying" | "installing" | "restarting";
  downloaded: number;
  total: number | null;
}

export interface DesktopUpdateSnapshot extends DesktopUpdateInfo {
  phase: UpdatePhase;
  available: AvailableDesktopUpdate | null;
  downloaded: number;
  total: number | null;
  error: string;
  lastChecked: number | null;
  lastAttempt: number;
}

export interface DesktopUpdateClient {
  info(): Promise<DesktopUpdateInfo>;
  check(): Promise<AvailableDesktopUpdate | null>;
  install(): Promise<void>;
  listen(progress: (event: DesktopUpdateProgress) => void): Promise<() => void>;
}

export function updateIsBusy(phase: UpdatePhase): boolean {
  return ["checking", "downloading", "verifying", "installing", "restarting"].includes(phase);
}

export function updateErrorMessage(error: unknown): string {
  const message = String(error);
  if (/signature|minisign|签名/i.test(message)) return "更新包签名校验失败，未安装更新。请重试或联系维护者。";
  if (/404/.test(message)) return "暂未找到可用更新，请稍后重试。";
  if (/network|connection|connect|timeout|timed out|dns|request|网络/i.test(message)) return "无法连接更新服务，请检查网络后重试。";
  return `更新未完成：${message.slice(0, 200)}`;
}

// Keep check/install operations serialized and preserve a discovered update if
// a later network check fails. A failed check never reports "latest version".
export class DesktopUpdateController {
  private state: DesktopUpdateSnapshot;
  private listeners = new Set<(state: DesktopUpdateSnapshot) => void>();
  private disposed = false;

  constructor(private client: DesktopUpdateClient, version: string, private now = Date.now) {
    this.state = { version, enabled: false, message: "正在读取更新设置…", phase: "idle", available: null, downloaded: 0, total: null, error: "", lastChecked: null, lastAttempt: 0 };
  }

  snapshot = (): DesktopUpdateSnapshot => this.state;

  subscribe(listener: (state: DesktopUpdateSnapshot) => void): () => void {
    this.listeners.add(listener);
    listener(this.state);
    return () => this.listeners.delete(listener);
  }

  private publish(changes: Partial<DesktopUpdateSnapshot>): void {
    if (this.disposed) return;
    this.state = { ...this.state, ...changes };
    this.listeners.forEach((listener) => listener(this.state));
  }

  async initialize(): Promise<void> {
    try {
      const info = await this.client.info();
      if (this.disposed) return;
      this.publish(info);
      if (info.enabled) await this.check();
    } catch (error) {
      this.publish({ phase: "error", error: updateErrorMessage(error), message: "无法读取更新设置" });
    }
  }

  async check(): Promise<void> {
    if (this.disposed || !this.state.enabled || updateIsBusy(this.state.phase)) return;
    this.publish({ phase: "checking", error: "", lastAttempt: this.now() });
    try {
      const available = await this.client.check();
      this.publish({ available, phase: available ? "available" : "idle", lastChecked: this.now() });
    } catch (error) {
      this.publish({ phase: "error", error: updateErrorMessage(error) });
    }
  }

  async install(): Promise<void> {
    if (this.disposed || !this.state.enabled || !this.state.available || updateIsBusy(this.state.phase)) return;
    this.publish({ phase: "downloading", downloaded: 0, total: null, error: "" });
    let stopListening: (() => void) | undefined;
    try {
      // Register before invoking so even the first native progress event is kept.
      stopListening = await this.client.listen((event) => this.publish(event));
      if (this.disposed) return;
      await this.client.install();
      this.publish({ phase: "restarting" });
    } catch (error) {
      this.publish({ phase: "error", error: updateErrorMessage(error) });
    } finally {
      stopListening?.();
    }
  }

  dispose(): void {
    this.disposed = true;
    this.listeners.clear();
  }
}
