import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Download, RefreshCw, X, ChevronDown } from "lucide-react";
import desktopPackage from "../package.json";
import {
  DesktopUpdateController, updateIsBusy,
  type DesktopUpdateInfo, type AvailableDesktopUpdate,
  type DesktopUpdateProgress, type DesktopUpdateSnapshot,
} from "./desktopUpdateModel";

const CHECK_INTERVAL = 6 * 60 * 60 * 1000;

function createController() {
  return new DesktopUpdateController({
    info: () => invoke<DesktopUpdateInfo>("desktop_update_info"),
    check: () => invoke<AvailableDesktopUpdate | null>("desktop_update_check"),
    install: () => invoke<void>("desktop_update_install"),
    listen: (progress) => listen<DesktopUpdateProgress>("desktop-update-progress", (event) => progress(event.payload)),
  }, desktopPackage.version);
}

export function useDesktopUpdates() {
  const ref = useRef<DesktopUpdateController | null>(null);
  const [dismissedVersion, dismiss] = useState<string | null>(null);
  const [state, setState] = useState<DesktopUpdateSnapshot>(() => createController().snapshot());
  useEffect(() => {
    const controller = createController();
    ref.current = controller;
    const unsubscribe = controller.subscribe(setState);
    void controller.initialize();
    const timer = setInterval(() => void controller.check(), CHECK_INTERVAL);
    const onVisible = () => {
      if (document.visibilityState === "visible" && Date.now() - controller.snapshot().lastAttempt >= CHECK_INTERVAL) void controller.check();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
      unsubscribe();
      controller.dispose();
      if (ref.current === controller) ref.current = null;
    };
  }, []);
  return { state, dismissedVersion, dismiss, check: () => void ref.current?.check(), install: () => void ref.current?.install() };
}

function progressLabel(state: DesktopUpdateSnapshot): string {
  if (state.phase === "verifying") return "正在校验更新包…";
  if (state.phase === "installing") return "正在安装更新…";
  if (state.phase === "restarting") return "正在重启应用…";
  const size = `${(state.downloaded / 1024 / 1024).toFixed(1)} MB`;
  return state.total && state.total > 0
    ? `正在下载 · ${Math.min(100, Math.floor(state.downloaded / state.total * 100))}%`
    : `正在下载 · ${size}`;
}

type UpdateProps = {
  state: DesktopUpdateSnapshot;
  install: () => void;
  blocked: boolean;
};

function UpdateProgress({ state }: { state: DesktopUpdateSnapshot }) {
  if (!["downloading", "verifying", "installing", "restarting"].includes(state.phase)) return null;
  return <div className="desktop-update-progress" role="status">
    <span>{progressLabel(state)}</span>
    <progress aria-label="更新下载进度" max={100} value={state.phase === "downloading" ? (state.total ? Math.min(100, state.downloaded / state.total * 100) : undefined) : 100} />
  </div>;
}

export function DesktopUpdateNotice({ state, install, blocked, onDetails, dismissedVersion, dismiss }: UpdateProps & { onDetails: () => void; dismissedVersion: string | null; dismiss: (version: string) => void }) {
  const busy = updateIsBusy(state.phase);
  if (!state.available || (dismissedVersion === state.available.version && !busy)) return null;
  return <aside className="desktop-update-notice" aria-label="桌面应用更新">
    <Download size={18} />
    <div className="desktop-update-summary">
      <strong>桌面应用有新版本 · v{state.available.version}</strong>
      <span>更新完成后应用将重启。</span>
      <UpdateProgress state={state} />
      {blocked && !busy && <span>请先完成手表安装或待回复指令。</span>}
      {state.error && <span className="desktop-update-error">{state.error}</span>}
    </div>
    <div className="desktop-update-actions">
      <button className="ghost" onClick={onDetails}>更新说明</button>
      <button className="primary" disabled={busy || blocked} onClick={install}>{busy ? "更新中…" : "立即更新"}</button>
      {!busy && <button className="ghost icon-button" aria-label="稍后更新" onClick={() => dismiss(state.available!.version)}><X size={15} /></button>}
    </div>
  </aside>;
}

export function DesktopUpdatePanel({ state, install, blocked, check }: UpdateProps & { check: () => void }) {
  const busy = updateIsBusy(state.phase);
  return <section className="desktop-update-panel" aria-labelledby="desktop-update-title">
    <div className="desktop-update-heading">
      <div>
        <h3 id="desktop-update-title">软件更新</h3>
        <p>{!state.enabled ? state.message : state.phase === "checking" ? "正在检查新版本…" : state.available ? `可更新至 v${state.available.version}` : state.lastChecked && !state.error ? "已是最新版本" : "启动时自动检查，每 6 小时再次检查。"}</p>
      </div>
      <button className="secondary" disabled={!state.enabled || busy} onClick={check}>
        <RefreshCw size={14} className={state.phase === "checking" ? "spinning" : ""} />
        {state.phase === "checking" ? "检查中…" : "检查更新"}
      </button>
    </div>
    {state.error && <p className="desktop-update-error" role="alert">{state.error}</p>}
    {state.available && <>
      <details className="disclosure desktop-update-notes" open>
        <summary><span>v{state.available.version} 更新说明</span><ChevronDown size={14} /></summary>
        <pre>{state.available.notes || "此版本包含改进与问题修复。"}</pre>
      </details>
      <UpdateProgress state={state} />
      <div className="desktop-update-footer">
        <p>{blocked ? "请先完成手表安装或待回复指令。" : "更新将重启桌面应用。手表应用可在连接后单独升级。"}</p>
        <button className="primary" disabled={busy || blocked} onClick={install}><Download size={15} />{busy ? "更新中…" : "立即更新"}</button>
      </div>
    </>}
  </section>;
}
