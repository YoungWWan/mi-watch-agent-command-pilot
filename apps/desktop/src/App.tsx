import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { WatchIllustration } from "./WatchIllustration";
import { CommandCenter } from "./CommandCenter";
import { useDesktopUpdates, DesktopUpdateNotice, DesktopUpdatePanel } from "./DesktopUpdates";
import { updateIsBusy } from "./desktopUpdateModel";
import { IntegrationManagement } from "./IntegrationManagement";
import { IntegrationStatus } from "./IntegrationStatus";
import { AgentTabs, type AgentSection } from "./AgentTabs";
import { watchPairingState, type ServerInfo, type CommandItem } from "./commandModel";
import {
  bundledApp,
  findWatchApp,
  needsWatchAppUpgrade,
  watchAppVersion,
  type QuickApp,
} from "./watchApp";
import {
  monitorDeviceConnection,
  type DeviceDisconnected,
  type DeviceConnectionStatus,
} from "./deviceConnection";
import {
  Watch,
  Bluetooth,
  Wifi,
  Terminal,
  CheckCircle2,
  RefreshCw,
  Copy,
  Check,
  Shield,
  Package,
  Radio,
  Info,
  ChevronDown,
  Power,
  QrCode,
  LogOut,
  Sun,
  Moon,
  User,
  X,
  Lock,
  AlertCircle,
} from "lucide-react";

interface InstallProgress {
  mac?: string;
  stage: string;
  sent_bytes: number;
  total_bytes: number;
  percent: number;
}

interface AgentStatus {
  key: string;
  name: string;
  installed: boolean;
  configured: boolean;
  needs_repair: boolean;
  message: string;
  config_path: string;
}

interface XiaomiDevice {
  id: string;
  name: string;
  model: string;
  mac: string;
  fw_ver?: string;
  has_authkey: boolean;
  is_verified: boolean;
  compatibility_note: string;
}

interface XiaomiStatus {
  logged_in: boolean;
  user_id?: string;
  device_count: number;
}

interface XiaomiQrSession {
  qr_url: string;
  qr_data_url: string;
  login_url: string;
  lp_url: string;
  timeout: number;
}

interface QrPollResponse {
  status: string;
  message?: string;
  user_id?: string;
  devices: XiaomiDevice[];
  verification_id?: string;
}

export default function App() {
  const [activeTab, setActiveTab] = useState<
    "device" | "service" | "integrations" | "about"
  >("device");
  const [activeAgentSection, setActiveAgentSection] = useState<AgentSection>("mcp");
  const workspaceRef = useRef<HTMLElement>(null);
  const desktopUpdates = useDesktopUpdates();

  // 主题模式 (浅色 / 深色)
  const [theme, setTheme] = useState<"dark" | "light">(() => {
    const saved = localStorage.getItem("theme");
    if (saved === "light" || saved === "dark") return saved;
    return window.matchMedia &&
      window.matchMedia("(prefers-color-scheme: light)").matches
      ? "light"
      : "dark";
  });

  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("theme", theme);
  }, [theme]);

  const toggleTheme = () => {
    setTheme((prev) => (prev === "dark" ? "light" : "dark"));
  };

  // 小米账号与云端设备同步
  const [xiaomiStatus, setXiaomiStatus] = useState<XiaomiStatus>({
    logged_in: false,
    device_count: 0,
  });
  const [cloudDevices, setCloudDevices] = useState<XiaomiDevice[]>([]);
  const [cloudDevicesError, setCloudDevicesError] = useState("");
  const [isLoadingAccount, setIsLoadingAccount] = useState(true);
  const [isSyncingCloud, setIsSyncingCloud] = useState<boolean>(false);
  const [showQrModal, setShowQrModal] = useState<boolean>(false);
  const [loginTab, setLoginTab] = useState<"qr" | "password">("qr");
  const [pwdUsername, setPwdUsername] = useState("");
  const [pwdPassword, setPwdPassword] = useState("");
  const [pwdLoggingIn, setPwdLoggingIn] = useState(false);
  const [pwdErrorMsg, setPwdErrorMsg] = useState("");
  const [pwdVerification, setPwdVerification] = useState<{
    id: string;
    message: string;
  } | null>(null);

  useEffect(() => {
    if (!pwdVerification) return;
    const id = pwdVerification.id;
    let cancelled = false;
    if (!showQrModal || loginTab !== "password") {
      setPwdVerification(null);
      invoke("account_xiaomi_cancel_verification", {
        verificationId: id,
      }).catch(console.error);
      return;
    }
    // The backend waits for the official completion page, then resumes the same session.
    invoke<QrPollResponse>("account_xiaomi_complete_verification", {
      verificationId: id,
    })
      .then((res) => {
        if (!cancelled) applyPasswordResult(res);
      })
      .catch((err) => {
        if (!cancelled) {
          setPwdVerification(null);
          setPwdErrorMsg(`登录未完成: ${err}`);
        }
      });
    return () => {
      cancelled = true;
      invoke("account_xiaomi_cancel_verification", {
        verificationId: id,
      }).catch(console.error);
    };
  }, [pwdVerification?.id, showQrModal, loginTab]);
  const [qrSession, setQrSession] = useState<XiaomiQrSession | null>(null);
  const [qrPollingStatus, setQrPollingStatus] = useState<string>("等待就绪");
  const [isQrPolling, setIsQrPolling] = useState<boolean>(false);

  // 硬件连接与安装
  const [mac, setMac] = useState<string>("");
  const [connectionStatus, setConnectionStatus] = useState<
    "idle" | "connecting" | "connected" | "error"
  >("idle");
  const [connectionError, setConnectionError] = useState("");
  const [connectedAddr, setConnectedAddr] = useState<string>("");
  const [installStatus, setInstallStatus] = useState<string>("等待就绪");
  const [installProgress, setInstallProgress] =
    useState<InstallProgress | null>(null);
  const [isInstalling, setIsInstalling] = useState(false);
  const [watchApp, setWatchApp] = useState<QuickApp | null>(null);
  const [hasCheckedWatchApp, setHasCheckedWatchApp] = useState(false);
  const [watchAppError, setWatchAppError] = useState("");
  const [isLoadingApps, setIsLoadingApps] = useState(false);
  const selectedDevice = cloudDevices.find(
    (d) => d.mac.toUpperCase() === mac.toUpperCase(),
  );
  const connectedDevice = cloudDevices.find(
    (d) => d.mac.toUpperCase() === connectedAddr.toUpperCase(),
  );
  const connectionEpoch = useRef(0);
  const installationEpoch = useRef<number | null>(null);
  const appQueryEpoch = useRef<number | null>(null);
  const macAddressRef = useRef("");

  const resetDeviceConnection = () => {
    connectionEpoch.current += 1;
    installationEpoch.current = null;
    appQueryEpoch.current = null;
    macAddressRef.current = "";
    setConnectionStatus("idle");
    setConnectionError("");
    setConnectedAddr("");
    setWatchApp(null);
    setHasCheckedWatchApp(false);
    setWatchAppError("");
    setIsLoadingApps(false);
    setIsInstalling(false);
    setInstallProgress(null);
    setInstallStatus("等待就绪");
  };

  useEffect(() => {
    if (connectionStatus !== "connecting" && connectionStatus !== "connected")
      return;
    const address =
      connectionStatus === "connecting"
        ? mac.trim().toUpperCase()
        : connectedAddr;
    if (!address) return;
    const epoch = connectionEpoch.current;
    return monitorDeviceConnection(address, {
      poll: connectionStatus === "connected",
      getStatus: () =>
        invoke<DeviceConnectionStatus>("get_device_connection_status", {
          mac: address,
        }),
      listen: (callback) =>
        listen<DeviceDisconnected>("device-disconnected", (event) =>
          callback(event.payload),
        ),
      onDisconnected: () => {
        if (epoch !== connectionEpoch.current) return;
        resetDeviceConnection();
        setConnectionStatus("error");
        setConnectionError(
          connectionStatus === "connecting"
            ? "连接过程中蓝牙已断开，请重试"
            : "蓝牙连接已断开，请重新连接手表",
        );
      },
      onError: (error) =>
        console.warn("[Bluetooth] Connection status check failed:", error),
    });
  }, [connectionStatus, connectedAddr, mac]);

  // 局域网服务与手腕配对
  const [serverInfo, setServerInfo] = useState<ServerInfo | null>(null);
  const [pairingCode, setPairingCode] = useState<string>("");
  const [commands, setCommands] = useState<CommandItem[]>([]);

  // Agent 集成
  const [agents, setAgents] = useState<AgentStatus[]>([]);
  const [agentConfigError, setAgentConfigError] = useState("");
  const [expandedAgents, setExpandedAgents] = useState<Record<string, boolean>>({});

  const loginModalRef = useRef<HTMLElement>(null);
  const [clipboardError, setClipboardError] = useState("");
  const closeLoginModal = () => {
    setShowQrModal(false);
    setIsQrPolling(false);
  };

  useEffect(() => {
    if (!showQrModal) return;
    const previousFocus = document.activeElement as HTMLElement | null;
    const modal = loginModalRef.current;
    modal?.querySelector<HTMLButtonElement>("button")?.focus();
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        closeLoginModal();
        return;
      }
      if (event.key !== "Tab" || !modal) return;
      const focusable = Array.from(
        modal.querySelectorAll<HTMLElement>(
          'button:not(:disabled), input:not(:disabled), a[href], [tabindex="0"]',
        ),
      );
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last?.focus();
      }
      if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first?.focus();
      }
    };
    document.addEventListener("keydown", handleKey);
    return () => {
      document.removeEventListener("keydown", handleKey);
      previousFocus?.focus();
    };
  }, [showQrModal]);

  // 复制反馈提示
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  const copyToClipboard = async (text: string, keyName: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setCopiedKey(keyName);
      setClipboardError("");
      setTimeout(
        () => setCopiedKey((current) => (current === keyName ? null : current)),
        2000,
      );
    } catch {
      setClipboardError("复制失败，请重试。");
    }
  };

  const loadServerInfo = async () => {
    try {
      const info = await invoke<ServerInfo>("service_get_info");
      setServerInfo(info);
      setPairingCode(info.active_pairing_code || "");
    } catch (e) {
      console.error(e);
    }
  };

  const loadCommands = async () => {
    try {
      const list = await invoke<CommandItem[]>("command_list_all");
      setCommands(list);
    } catch (e) {
      console.error(e);
    }
  };

  const loadAgents = async () => {
    try {
      const list = await invoke<AgentStatus[]>("integration_get_statuses");
      setAgents(list);
      setAgentConfigError("");
    } catch (e) {
      setAgentConfigError(String(e));
    }
  };

  const loadXiaomiAccount = async () => {
    setCloudDevicesError("");
    try {
      const status = await invoke<XiaomiStatus>("account_xiaomi_get_status");
      setXiaomiStatus(status);
      if (status.logged_in) {
        try {
          const devs = await invoke<XiaomiDevice[]>(
            "account_xiaomi_get_devices",
          );
          setCloudDevices(devs);
          if (devs.length > 0) {
            const watch =
              devs.find((d) => d.mac.toUpperCase() === mac.toUpperCase()) ||
              devs.find((d) => d.is_verified && d.has_authkey) ||
              devs.find((d) => d.has_authkey) ||
              devs[0];
            if (watch && !mac) {
              handleSelectCloudDevice(watch);
            }
          }
        } catch (fetchErr) {
          console.warn("[Xiaomi] Devices fetch failed:", fetchErr);
          setCloudDevicesError(String(fetchErr));
          const updated = await invoke<XiaomiStatus>(
            "account_xiaomi_get_status",
          );
          setXiaomiStatus(updated);
          setCloudDevices([]);
        }
      }
    } catch (e) {
      console.error("[Xiaomi] Load status error:", e);
      setCloudDevicesError(String(e));
    } finally {
      setIsLoadingAccount(false);
    }
  };

  const handleOpenLoginModal = (tab: "qr" | "password" = "qr") => {
    setLoginTab(tab);
    setPwdErrorMsg("");
    setShowQrModal(true);
    if (tab === "qr" && (!qrSession || !isQrPolling)) {
      handleStartQrLogin();
    }
  };

  const handleStartQrLogin = async () => {
    try {
      setLoginTab("qr");
      setShowQrModal(true);
      setIsQrPolling(true);
      setQrPollingStatus("正在加载二维码…");
      const session = await invoke<XiaomiQrSession>("account_xiaomi_get_qr");
      setQrSession(session);
      setQrPollingStatus("用小米运动健康或米家扫码登录。");

      // 启动长轮询
      pollQrLoop(session.lp_url);
    } catch (e: any) {
      console.error("[Xiaomi] QR login failed:", e);
      setQrPollingStatus("无法获取二维码，请重试。");
      setIsQrPolling(false);
    }
  };

  const pollQrLoop = async (lp_url: string) => {
    const startTime = Date.now();
    while (true) {
      if (Date.now() - startTime > 300000) {
        setQrPollingStatus("二维码已过期，请刷新。");
        setIsQrPolling(false);
        break;
      }
      try {
        const res = await invoke<QrPollResponse>("account_xiaomi_check_qr", {
          lpUrl: lp_url,
        });
        if (res.status === "success") {
          setQrPollingStatus("登录成功，正在同步设备…");
          setCloudDevicesError("");
          setXiaomiStatus({
            logged_in: true,
            user_id: res.user_id,
            device_count: res.devices.length,
          });
          setCloudDevices(res.devices);
          if (res.devices.length > 0) {
            const watch =
              res.devices.find((d) => d.is_verified && d.has_authkey) ||
              res.devices.find((d) => d.has_authkey) ||
              res.devices[0];
            handleSelectCloudDevice(watch);
          }
          setIsQrPolling(false);
          setTimeout(() => {
            setShowQrModal(false);
          }, 1200);
          break;
        } else if (res.status === "waiting") {
          if (res.message) setQrPollingStatus(res.message);
        } else {
          setQrPollingStatus(res.message || "登录失败，请刷新二维码。");
          setIsQrPolling(false);
          break;
        }
      } catch (err) {
        console.error("[Xiaomi] QR polling failed:", err);
        setQrPollingStatus("登录失败，请刷新二维码。");
        setIsQrPolling(false);
        break;
      }
      await new Promise((r) => setTimeout(r, 1800));
    }
  };

  const handleSelectCloudDevice = async (dev: XiaomiDevice) => {
    if (connectionStatus === "connecting" || connectionStatus === "connected")
      return;
    setMac(dev.mac);
  };

  const handleSyncCloudDevices = async () => {
    try {
      setIsSyncingCloud(true);
      setCloudDevicesError("");
      const devs = await invoke<XiaomiDevice[]>("account_xiaomi_get_devices");
      setCloudDevices(devs);
      setXiaomiStatus((prev) => ({ ...prev, device_count: devs.length }));
      setIsSyncingCloud(false);
      if (devs.length > 0) {
        const watch =
          devs.find((d) => d.mac.toUpperCase() === mac.toUpperCase()) ||
          devs.find((d) => d.is_verified && d.has_authkey) ||
          devs.find((d) => d.has_authkey) ||
          devs[0];
        handleSelectCloudDevice(watch);
      }
    } catch (e: any) {
      setIsSyncingCloud(false);
      setCloudDevicesError(String(e));
      const updated = await invoke<XiaomiStatus>("account_xiaomi_get_status");
      setXiaomiStatus(updated);
      if (!updated.logged_in) {
        setCloudDevices([]);
      }
    }
  };

  const handleLogoutXiaomi = async () => {
    try {
      if (connectedAddr)
        await invoke("disconnect_device", { mac: connectedAddr });
      await invoke("account_xiaomi_logout");
      resetDeviceConnection();
      setXiaomiStatus({ logged_in: false, device_count: 0 });
      setCloudDevices([]);
      setCloudDevicesError("");
      setMac("");
      setQrSession(null);
      setIsQrPolling(false);
      setQrPollingStatus("等待就绪");
      setPwdUsername("");
      setPwdPassword("");
      setPwdErrorMsg("");
    } catch (e) {
      console.error("[Xiaomi] Logout error:", e);
    }
  };

  const applyPasswordResult = (res: QrPollResponse) => {
    if (res.status === "verification_required" && res.verification_id) {
      setPwdVerification({
        id: res.verification_id,
        message: res.message || "请完成小米官方安全验证",
      });
      setPwdPassword("");
    } else if (res.status === "success") {
      setPwdVerification(null);
      setPwdPassword("");
      setCloudDevicesError("");
      setXiaomiStatus({
        logged_in: true,
        user_id: res.user_id,
        device_count: res.devices.length,
      });
      setCloudDevices(res.devices);
      if (res.devices.length > 0) {
        const watch =
          res.devices.find((d) => d.is_verified && d.has_authkey) ||
          res.devices.find((d) => d.has_authkey) ||
          res.devices[0];
        handleSelectCloudDevice(watch);
      }
      setShowQrModal(false);
    } else {
      setPwdVerification(null);
      setPwdErrorMsg(res.message || "登录验证失败，请重试");
    }
  };

  const handlePasswordLogin = async (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    if (pwdLoggingIn || pwdVerification) return;
    if (!pwdUsername.trim() || !pwdPassword) {
      setPwdErrorMsg("请输入小米账号与密码");
      return;
    }
    try {
      setPwdLoggingIn(true);
      setPwdErrorMsg("");
      const res = await invoke<QrPollResponse>(
        "account_xiaomi_password_login",
        {
          user: pwdUsername.trim(),
          pass: pwdPassword,
        },
      );
      applyPasswordResult(res);
    } catch (err: any) {
      console.error("[Xiaomi] Password login failed:", err);
      setPwdErrorMsg("登录失败，请重试或扫码登录。");
    } finally {
      setPwdLoggingIn(false);
    }
  };

  useEffect(() => {
    loadServerInfo();
    loadCommands();
    loadAgents();
    loadXiaomiAccount();

    const interval = setInterval(() => {
      loadServerInfo();
      loadCommands();
    }, 3000);

    const unlistenProgress = listen<InstallProgress>(
      "install-progress",
      (event) => {
        if (
          installationEpoch.current === null ||
          installationEpoch.current !== connectionEpoch.current ||
          (event.payload.mac !== undefined &&
            event.payload.mac !== macAddressRef.current)
        )
          return;
        setInstallProgress(event.payload);
        if (event.payload.stage === "COMPLETE") {
          setInstallStatus("安装完成");
        } else if (event.payload.stage === "CHECKING") {
          setInstallStatus("正在检查指令助手…");
        } else if (event.payload.stage === "UNINSTALLING") {
          setWatchApp(null);
          setHasCheckedWatchApp(false);
          setInstallStatus("正在卸载旧版指令助手…");
        } else if (event.payload.stage === "VERIFYING") {
          setInstallStatus("正在确认安装结果…");
        } else {
          setInstallStatus(`正在传输安装包 ${event.payload.percent}%`);
        }
      },
    );

    return () => {
      clearInterval(interval);
      unlistenProgress.then((f) => f());
    };
  }, []);

  const handleConnect = async () => {
    if (!xiaomiStatus.logged_in) {
      alert("请先登录小米账号以获取手表配对许可");
      return;
    }
    if (!mac.trim()) {
      alert("请选择要连接的手表设备");
      return;
    }
    const device = cloudDevices.find(
      (d) => d.mac.toUpperCase() === mac.toUpperCase(),
    );
    if (!device?.has_authkey) {
      alert("请选择当前账号下已授权的设备，或刷新设备列表");
      return;
    }
    resetDeviceConnection();
    const epoch = connectionEpoch.current;
    try {
      setConnectionError("");
      setConnectionStatus("connecting");
      const res = await invoke<{ success: boolean; message: string }>(
        "connect_and_auth",
        {
          mac: mac.trim(),
        },
      );
      if (epoch !== connectionEpoch.current) return;
      if (res.success) {
        const address = mac.trim().toUpperCase();
        macAddressRef.current = address;
        setConnectionStatus("connected");
        setConnectedAddr(address);
        void queryWatchApp(address, epoch);
      } else {
        setConnectionStatus("error");
        setConnectionError(`连接失败: ${res.message}`);
      }
    } catch (err: any) {
      if (epoch !== connectionEpoch.current) return;
      setConnectionStatus("error");
      setConnectionError(`连接失败: ${err}`);
      try {
        const updated = await invoke<XiaomiStatus>("account_xiaomi_get_status");
        if (epoch !== connectionEpoch.current) return;
        setXiaomiStatus(updated);
        if (!updated.logged_in) {
          setCloudDevices([]);
          setCloudDevicesError(String(err));
        }
      } catch (statusError) {
        console.error("[Xiaomi] Load status error:", statusError);
      }
    }
  };

  const handleDisconnect = async () => {
    const epoch = connectionEpoch.current;
    try {
      await invoke("disconnect_device", { mac: connectedAddr });
      if (epoch === connectionEpoch.current) resetDeviceConnection();
    } catch (err) {
      console.error(err);
    }
  };

  const handleInstallRpk = async () => {
    if (updateIsBusy(desktopUpdates.state.phase)) return;
    if (connectionStatus !== "connected") {
      alert("请先连接手表蓝牙设备");
      return;
    }
    if (installationEpoch.current !== null || appQueryEpoch.current !== null)
      return;
    const epoch = connectionEpoch.current;
    installationEpoch.current = epoch;
    try {
      setIsInstalling(true);
      setInstallStatus("正在检查指令助手…");
      setInstallProgress({
        stage: "CHECKING",
        sent_bytes: 0,
        total_bytes: 0,
        percent: 0,
      });

      const res = await invoke<{ success: boolean; message: string }>(
        "install_bundled_rpk",
        {
          mac: connectedAddr,
        },
      );
      if (epoch !== connectionEpoch.current) return;
      if (res.success) {
        setInstallStatus(
          `指令助手 v${bundledApp.versionName} 已成功安装至 ${connectedDevice?.name || "所连设备"}！`,
        );
      } else {
        setInstallStatus(`安装未完成: ${res.message}`);
        alert(`安装失败: ${res.message}`);
      }
    } catch (err: any) {
      if (epoch !== connectionEpoch.current) return;
      setInstallStatus(`安装异常: ${err}`);
      alert(`安装异常: ${err}`);
    } finally {
      if (epoch === connectionEpoch.current) {
        // A failure may occur after removal; always refresh the actual state.
        await queryWatchApp(connectedAddr, epoch);
        if (epoch === connectionEpoch.current) {
          installationEpoch.current = null;
          setIsInstalling(false);
        }
      }
    }
  };

  const queryWatchApp = async (address: string, epoch: number) => {
    if (epoch !== connectionEpoch.current || appQueryEpoch.current !== null)
      return;
    appQueryEpoch.current = epoch;
    try {
      setIsLoadingApps(true);
      setWatchAppError("");
      const list = await invoke<QuickApp[]>("query_device_apps", {
        mac: address,
      });
      if (epoch !== connectionEpoch.current) return;
      setWatchApp(findWatchApp(list));
      setHasCheckedWatchApp(true);
    } catch (err) {
      if (epoch !== connectionEpoch.current) return;
      setWatchApp(null);
      setHasCheckedWatchApp(false);
      setWatchAppError(`无法检查指令助手，请重试：${err}`);
      console.error(err);
    } finally {
      if (epoch === connectionEpoch.current) {
        appQueryEpoch.current = null;
        setIsLoadingApps(false);
      }
    }
  };

  const handleQueryApps = () => {
    if (
      connectionStatus !== "connected" ||
      !connectedAddr ||
      installationEpoch.current !== null
    )
      return;
    void queryWatchApp(connectedAddr, connectionEpoch.current);
  };

  const handleGeneratePairingCode = async () => {
    try {
      const code = await invoke<string>("pairing_create_code");
      setPairingCode(code);
    } catch (e: any) {
      alert(`生成配对码失败：${e}`);
    }
  };

  const handleToggleAgent = async (key: string, currentConfigured: boolean) => {
    try {
      const targetState = !currentConfigured;
      await invoke("integration_toggle", { key, enable: targetState });
      await loadAgents();
    } catch (e: any) {
      alert(`配置 ${key} 失败: ${e}`);
    }
  };

  const configuredCount = agents.filter((a) => a.configured).length;
  const isWatchInstalled = hasCheckedWatchApp && watchApp !== null;
  const needsUpgrade = hasCheckedWatchApp && needsWatchAppUpgrade(watchApp);

  const pendingCount = commands.filter(
    (command) => command.status === "pending",
  ).length;
  const pageTitle = {
    device: "设备",
    service: "指令",
    integrations: "Agent",
    about: "关于",
  }[activeTab];
  const deviceLocked =
    connectionStatus === "connecting" || connectionStatus === "connected";
  const isLoadingDevices = isLoadingAccount || isSyncingCloud;
  const pairingState = watchPairingState(serverInfo);
  const pairingPanel = (
    <details className="pairing-disclosure" open={pairingState.expanded}>
      <summary>
        <span>手表配对</span>
        <span className="pairing-state">
          <span
            className={`status-dot ${pairingState.online ? "online" : pairingState.failed ? "failed" : ""}`}
          />
          {pairingState.label}
          <ChevronDown size={14} />
        </span>
      </summary>
      <section className="pairing-panel" aria-label="手表配对">
        <div className="pairing-main">
          <h2>配对码</h2>
          <p className={pairingState.failed ? "inline-error" : "section-note"} role={pairingState.failed ? "alert" : undefined}>
            {pairingState.note}
          </p>
          <div className="pairing-code-row">
            <span className={`pairing-code ${!pairingCode ? "muted" : ""}`}>
              {pairingCode || "------"}
            </span>
            <button
              className="ghost icon-button"
              onClick={() => copyToClipboard(pairingCode, "code")}
              disabled={!pairingCode}
              aria-label={copiedKey === "code" ? "配对码已复制" : "复制配对码"}
              title="复制配对码"
            >
              {copiedKey === "code" ? <Check size={16} /> : <Copy size={16} />}
            </button>
          </div>
          <div className="pairing-footer">
            <button className="secondary" onClick={handleGeneratePairingCode} disabled={serverInfo?.status !== "running"}>
              <RefreshCw size={14} />
              {pairingCode ? "重新生成" : "生成配对码"}
            </button>
            <span className="page-meta">5 分钟内有效</span>
          </div>
        </div>
        <div className="network-info">
          <Wifi size={20} strokeWidth={1.5} />
          <h3>电脑地址</h3>
          <div className="address-row">
            <span>{serverInfo?.lan_ip || "—"}</span>
            <button
              className="ghost icon-button"
              disabled={!serverInfo?.lan_ip}
              onClick={() => copyToClipboard(serverInfo?.lan_ip || "", "ip")}
              aria-label={
                copiedKey === "ip" ? "电脑地址已复制" : "复制电脑地址"
              }
              title="复制电脑地址"
            >
              {copiedKey === "ip" ? <Check size={14} /> : <Copy size={14} />}
            </button>
          </div>
          <p className="section-note">手表与电脑需在同一局域网。</p>
          <details className="disclosure network-details">
            <summary>
              <span>连接详情</span>
              <ChevronDown size={13} />
            </summary>
            <div className="network-url">
              <code>{serverInfo?.server_url || "服务尚未就绪"}</code>
              <button
                className="ghost icon-button"
                disabled={!serverInfo?.server_url}
                onClick={() =>
                  copyToClipboard(serverInfo?.server_url || "", "url")
                }
                aria-label="复制服务地址"
              >
                {copiedKey === "url" ? <Check size={13} /> : <Copy size={13} />}
              </button>
            </div>
            <p className="section-note">端口 {serverInfo?.port || "—"}</p>
          </details>
        </div>
      </section>
    </details>
  );

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <span className="brand-mark">
            <img src="/app-icon.png" alt="" width={40} height={40} />
          </span>
          <span>
            小米手表<small>AI 指令助手</small>
          </span>
        </div>
        <nav className="main-nav" aria-label="主导航">
          <button
            className={`nav-item ${activeTab === "device" ? "active" : ""}`}
            aria-current={activeTab === "device" ? "page" : undefined}
            aria-label="设备"
            onClick={() => setActiveTab("device")}
          >
            <Watch size={17} />
            <span>设备</span>
          </button>
          <button
            className={`nav-item ${activeTab === "service" ? "active" : ""}`}
            aria-current={activeTab === "service" ? "page" : undefined}
            aria-label={
              pendingCount ? `指令，${pendingCount} 条待回复` : "指令"
            }
            onClick={() => setActiveTab("service")}
          >
            <Radio size={17} />
            <span>指令</span>
            {pendingCount > 0 && (
              <span className="nav-count">{pendingCount}</span>
            )}
          </button>
          <button
            className={`nav-item ${activeTab === "integrations" ? "active" : ""}`}
            aria-current={activeTab === "integrations" ? "page" : undefined}
            aria-label="Agent"
            onClick={() => setActiveTab("integrations")}
          >
            <Terminal size={17} />
            <span>Agent</span>
          </button>
        </nav>
        <div className="sidebar-bottom">
          <div className="service-status">
            <span
              className={`status-dot ${serverInfo?.status === "running" ? "online" : ""}`}
            />
            {serverInfo?.status === "running"
              ? "本地服务已就绪"
              : "本地服务未就绪"}
          </div>
          <div className="sidebar-tools">
            <button
              className={`ghost about-link ${activeTab === "about" ? "active" : ""}`}
              aria-current={activeTab === "about" ? "page" : undefined}
              onClick={() => setActiveTab("about")}
            >
              <Info size={15} />
              关于
            </button>
            <button
              className="ghost icon-button"
              onClick={toggleTheme}
              aria-label={theme === "dark" ? "切换浅色模式" : "切换深色模式"}
              title={theme === "dark" ? "切换浅色模式" : "切换深色模式"}
            >
              {theme === "dark" ? <Sun size={16} /> : <Moon size={16} />}
            </button>
          </div>
        </div>
      </aside>

      <main className="workspace" key={activeTab} ref={workspaceRef}>
        <div className="page-content">
          <header className="page-header">
            <h1>{pageTitle}</h1>
            {activeTab === "device" && (
              <div className="header-actions">
                {xiaomiStatus.logged_in ? (
                  <>
                    <button
                      className="ghost icon-button"
                      onClick={handleSyncCloudDevices}
                      disabled={isSyncingCloud}
                      aria-label="刷新设备"
                      title="刷新设备"
                    >
                      <RefreshCw
                        size={16}
                        className={isSyncingCloud ? "spinning" : ""}
                      />
                    </button>
                    <details className="account-menu">
                      <summary aria-label="小米账号">
                        <User size={15} />
                        <span>小米账号</span>
                        <ChevronDown size={12} />
                      </summary>
                      <div className="account-popover">
                        <small>已登录</small>
                        <strong>{xiaomiStatus.user_id || "小米账号"}</strong>
                        <button
                          className="ghost"
                          onClick={handleLogoutXiaomi}
                          disabled={
                            connectionStatus === "connecting" || isInstalling
                          }
                        >
                          <LogOut size={14} />
                          退出登录
                        </button>
                      </div>
                    </details>
                  </>
                ) : (
                  <button
                    className="secondary"
                    onClick={() => handleOpenLoginModal("qr")}
                  >
                    <User size={15} />
                    登录小米账号
                  </button>
                )}
              </div>
            )}
            {activeTab === "integrations" && (
              <span className="page-meta">{configuredCount} 个已接入</span>
            )}
            {activeTab === "service" && (
              <span className="page-meta">
                {pendingCount > 0 ? `${pendingCount} 条待回复` : "自动同步"}
              </span>
            )}
          </header>

          {activeTab !== "about" && <DesktopUpdateNotice state={desktopUpdates.state} install={desktopUpdates.install} blocked={isInstalling || pendingCount > 0} onDetails={() => setActiveTab("about")} dismissedVersion={desktopUpdates.dismissedVersion} dismiss={desktopUpdates.dismiss} />}

          {activeTab === "device" && (
            <section className="device-workbench" aria-label="设备管理">
              {cloudDevicesError && (
                <p className="inline-error" role="alert">
                  {cloudDevicesError}
                </p>
              )}
              {xiaomiStatus.logged_in && cloudDevices.length > 0 ? (
                <>
                  <div
                    className="device-selector"
                    role="group"
                    aria-label="选择设备"
                  >
                    {cloudDevices.map((device) => (
                      <button
                        key={device.mac}
                        className={`device-option ${mac.toUpperCase() === device.mac.toUpperCase() ? "selected" : ""}`}
                        aria-pressed={
                          mac.toUpperCase() === device.mac.toUpperCase()
                        }
                        disabled={deviceLocked || isSyncingCloud}
                        onClick={() => handleSelectCloudDevice(device)}
                      >
                        <Watch size={15} />
                        <span>{device.name}</span>
                      </button>
                    ))}
                  </div>
                  <div className="device-stage">
                    <div className="device-visual">
                      <WatchIllustration
                        connected={connectionStatus === "connected"}
                      />
                    </div>
                    <div className="device-focus">
                      <h2>{selectedDevice?.name || "选择一台设备"}</h2>
                      <div className="connection-label" role="status">
                        <span
                          className={`status-dot ${connectionStatus === "connected" ? "online" : ""}`}
                        />
                        {connectionStatus === "connected"
                          ? "蓝牙已连接"
                          : connectionStatus === "connecting"
                            ? "正在连接…"
                            : "蓝牙未连接"}
                      </div>
                      {connectionError ? (
                        <p className="inline-error" role="alert">
                          {connectionError}
                        </p>
                      ) : (
                        <p className="device-hint">
                          {connectionStatus === "connected"
                            ? "连接后自动检查指令助手的安装状态与版本。"
                            : "将设备放在附近，并开启蓝牙。"}
                        </p>
                      )}
                      {selectedDevice && !selectedDevice.has_authkey && (
                        <p className="inline-error">
                          缺少连接凭据，请刷新设备。
                        </p>
                      )}
                      {selectedDevice && !selectedDevice.is_verified && (
                        <p className="compatibility-note">
                          <Info size={13} />
                          <span>
                            {selectedDevice.compatibility_note ||
                              "此机型尚未验证，可尝试连接与安装。"}
                          </span>
                        </p>
                      )}
                      <div className="device-primary-action">
                        {connectionStatus === "connected" ? (
                          <button
                            className="secondary"
                            onClick={handleDisconnect}
                            disabled={isInstalling}
                          >
                            <Power size={15} />
                            断开连接
                          </button>
                        ) : (
                          <button
                            className="primary"
                            onClick={handleConnect}
                            disabled={
                              connectionStatus === "connecting" ||
                              isSyncingCloud ||
                              !selectedDevice?.has_authkey
                            }
                          >
                            {connectionStatus === "connecting" ? (
                              <RefreshCw size={15} className="spinning" />
                            ) : (
                              <Bluetooth size={15} />
                            )}
                            {connectionStatus === "connecting"
                              ? "连接中…"
                              : "连接设备"}
                          </button>
                        )}
                      </div>
                    </div>
                  </div>
                  {selectedDevice && (
                    <details className="disclosure device-details">
                      <summary>
                        <span>设备详情</span>
                        <ChevronDown size={14} />
                      </summary>
                      <dl className="device-specs">
                        <div>
                          <dt>型号</dt>
                          <dd>{selectedDevice.model || "未知"}</dd>
                        </div>
                        <div>
                          <dt>蓝牙地址</dt>
                          <dd>{selectedDevice.mac}</dd>
                        </div>
                        <div>
                          <dt>固件</dt>
                          <dd>
                            {selectedDevice.fw_ver
                              ? `v${selectedDevice.fw_ver}`
                              : "未知"}
                          </dd>
                        </div>
                        <div>
                          <dt>兼容性</dt>
                          <dd>
                            {selectedDevice.is_verified ? "已验证" : "尚未验证"}
                          </dd>
                        </div>
                      </dl>
                    </details>
                  )}
                </>
              ) : (
                <div className="device-empty">
                  <WatchIllustration compact />
                  <h2>
                    {isLoadingDevices
                      ? "正在同步设备…"
                      : cloudDevicesError
                        ? xiaomiStatus.logged_in
                          ? "设备同步失败"
                          : "请重新登录小米账号"
                      : xiaomiStatus.logged_in
                        ? "还没有发现设备"
                        : "从你的手表开始"}
                  </h2>
                  <p>
                    {isLoadingDevices
                      ? "稍等片刻，正在读取设备列表。"
                      : cloudDevicesError
                        ? xiaomiStatus.logged_in
                          ? "请检查网络后刷新设备。"
                          : "重新扫码授权后即可同步设备。"
                      : xiaomiStatus.logged_in
                        ? "在小米运动健康中绑定设备，再刷新列表。"
                        : "登录小米账号，同步你的穿戴设备。"}
                  </p>
                  <button
                    className="primary"
                    onClick={
                      xiaomiStatus.logged_in
                        ? handleSyncCloudDevices
                        : () => handleOpenLoginModal("qr")
                    }
                    disabled={isLoadingDevices}
                  >
                    {isLoadingDevices || xiaomiStatus.logged_in ? (
                      <RefreshCw
                        size={15}
                        className={isLoadingDevices ? "spinning" : ""}
                      />
                    ) : (
                      <User size={15} />
                    )}
                    {isLoadingDevices
                      ? "正在同步…"
                      : xiaomiStatus.logged_in
                        ? "刷新设备"
                        : "登录小米账号"}
                  </button>
                </div>
              )}

              <div className="installation-section">
                <div className="app-install-row">
                  <span className="app-icon">
                    <Terminal size={21} strokeWidth={1.6} />
                  </span>
                  <div className="install-copy">
                    <h3>
                      指令助手
                      {isWatchInstalled && !isLoadingApps && !needsUpgrade && (
                        <CheckCircle2
                          size={14}
                          className="positive"
                          aria-label="已安装"
                        />
                      )}
                    </h3>
                    <p>
                      {connectionStatus !== "connected"
                        ? `手表应用 · v${bundledApp.versionName}`
                        : isLoadingApps
                          ? "正在检查安装状态与版本…"
                          : !hasCheckedWatchApp
                            ? "安装状态待检查"
                            : watchApp
                              ? `已安装 · ${watchAppVersion(watchApp)}${needsUpgrade ? " · 有新版本" : watchApp.version_code === bundledApp.versionCode ? " · 当前版本" : ""}`
                              : `尚未安装 · 可安装 v${bundledApp.versionName}`}
                      {connectionStatus !== "connected" && (
                        <>
                          <span className="meta-separator">·</span>
                          连接后可安装
                        </>
                      )}
                    </p>
                  </div>
                  <div className="install-actions">
                    <button
                      className="ghost icon-button"
                      onClick={handleQueryApps}
                      disabled={
                        connectionStatus !== "connected" ||
                        isLoadingApps ||
                        isInstalling
                      }
                      aria-label="检查指令助手版本"
                      title="检查指令助手版本"
                    >
                      <RefreshCw
                        size={15}
                        className={isLoadingApps ? "spinning" : ""}
                      />
                    </button>
                    <button
                      className="secondary"
                      onClick={handleInstallRpk}
                      disabled={
                        connectionStatus !== "connected" ||
                        isInstalling ||
                        isLoadingApps ||
                        updateIsBusy(desktopUpdates.state.phase)
                      }
                    >
                      <Package size={15} />
                      {isInstalling
                        ? "安装中…"
                        : needsUpgrade
                          ? "升级到新版本"
                          : isWatchInstalled
                            ? "重新安装"
                            : "安装到设备"}
                    </button>
                  </div>
                </div>
                {needsUpgrade && !isLoadingApps && !isInstalling && (
                  <p className="install-notice" role="status">
                    手表上的指令助手版本较旧，请升级至 v{bundledApp.versionName}
                    。升级会先卸载旧版，再安装新版。
                  </p>
                )}
                {watchAppError && (
                  <p className="inline-error" role="alert">
                    {watchAppError}
                  </p>
                )}
                {isInstalling && installProgress && (
                  <div className="install-progress" role="status">
                    <div>
                      <span>{installStatus}</span>
                      <span>{Math.round(installProgress.percent)}%</span>
                    </div>
                    <progress
                      value={installProgress.percent}
                      max={100}
                      aria-label="应用安装进度"
                    />
                  </div>
                )}
                {!isInstalling && installStatus !== "等待就绪" && (
                  <p
                    className={`install-result ${installStatus.includes("成功") || installStatus === "安装完成" ? "positive" : "inline-error"}`}
                    role="status"
                  >
                    {installStatus}
                  </p>
                )}
              </div>
            </section>
          )}

          {activeTab === "service" && (
            <CommandCenter server={serverInfo} commands={commands} pairingPanel={pairingPanel}
              refresh={async () => { await Promise.all([loadServerInfo(), loadCommands()]); }} />
          )}

          {activeTab === "integrations" && (
            <div className="agent-page">
              <AgentTabs activeSection={activeAgentSection} onChange={section => {
                if (section === activeAgentSection) return;
                setActiveAgentSection(section);
                workspaceRef.current?.scrollTo({ top: 0 });
              }} />
              <div id="agent-panel-mcp" role="tabpanel" aria-labelledby="agent-tab-mcp" tabIndex={0} hidden={activeAgentSection !== "mcp"}>
                {agentConfigError && <p className="inline-error workbench-feedback" role="alert">{agentConfigError}</p>}
                <section className="integration-panel" aria-label="Agent 接入">
              <div className="section-heading">
                <div>
                  <h2>MCP 选择题工具</h2>
                  <p className="section-note">将选择题送到手腕，回复后继续任务。</p>
                </div>
                <button
                  className="ghost icon-button"
                  onClick={loadAgents}
                  aria-label="刷新 Agent 状态"
                  title="刷新 Agent 状态"
                >
                  <RefreshCw size={15} />
                </button>
              </div>
              {agents.length > 0 ? (
                <div className="agent-list">
                  {agents.map((agent) => (
                    <div className="agent-item" key={agent.key}>
                      <div className="agent-info">
                        <h3>{agent.name}</h3>
                        <div className="agent-state">
                          <IntegrationStatus state={agent.configured ? "active" : agent.needs_repair ? "warning" : "inactive"}>
                            {agent.configured
                              ? "已接入"
                              : agent.needs_repair
                                ? "需要修复"
                                : agent.installed
                                  ? "未接入"
                                  : "未检测到安装"}
                          </IntegrationStatus>
                        </div>
                      </div>
                      <button
                        className="ghost agent-details-toggle"
                        aria-label={`${agent.name} 配置详情`}
                        aria-expanded={Boolean(expandedAgents[agent.key])}
                        aria-controls={`agent-details-${agent.key}`}
                        onClick={() =>
                          setExpandedAgents((previous) => ({
                            ...previous,
                            [agent.key]: !previous[agent.key],
                          }))
                        }
                      >
                        配置详情
                        <ChevronDown
                          size={12}
                          className={
                            expandedAgents[agent.key] ? "rotated" : undefined
                          }
                        />
                      </button>
                      <button
                        className={`secondary agent-action ${agent.configured ? "" : agent.needs_repair ? "agent-action-repair" : "agent-action-enable"}`}
                        aria-label={`${agent.configured ? "停用" : agent.needs_repair ? "修复" : "接入"} ${agent.name}`}
                        onClick={() =>
                          handleToggleAgent(agent.key, agent.configured)
                        }
                      >
                        {agent.configured ? "停用" : agent.needs_repair ? "修复" : "接入"}
                      </button>
                      <div
                        className="agent-details"
                        id={`agent-details-${agent.key}`}
                        hidden={!expandedAgents[agent.key]}
                      >
                        <div className="agent-path">
                          <code>{agent.config_path}</code>
                          <button
                            className="ghost icon-button"
                            onClick={() =>
                              copyToClipboard(agent.config_path, agent.key)
                            }
                            aria-label={`复制 ${agent.name} 配置路径`}
                          >
                            {copiedKey === agent.key ? (
                              <Check size={12} />
                            ) : (
                              <Copy size={12} />
                            )}
                          </button>
                        </div>
                        {agent.message && (
                          <p className="section-note">{agent.message}</p>
                        )}
                      </div>
                    </div>
                  ))}
                </div>
              ) : (
                <div className="quiet-empty">
                  <Terminal size={26} strokeWidth={1.3} />
                  <h3>还未读取到 Agent</h3>
                  <button className="secondary" onClick={loadAgents}>
                    重新检测
                  </button>
                </div>
              )}
              <details className="disclosure integration-help">
                <summary>
                  <span>使用说明</span>
                  <ChevronDown size={14} />
                </summary>
                <p>
                  接入后重开 Agent 会话，即可通过{" "}
                  <code>ask_watch_question</code>{" "}
                  向手表发送选择题。点击手表上的选项，Agent
                  会收到回复并继续执行。
                </p>
              </details>
                </section>
              </div>
              <IntegrationManagement activeSection={activeAgentSection} />
            </div>
          )}

          {activeTab === "about" && (
            <section className="about-panel">
              <div className="about-intro">
                <span className="brand-mark large">
                  <img src="/app-icon.png" alt="" width={80} height={80} />
                </span>
                <h2>小米手表 AI 指令助手</h2>
                <p>
                  版本 {desktopUpdates.state.version}
                </p>
                <p className="about-description">
                  AI 提问、操作审批、完成提醒，手表直接回复。
                </p>
                <a
                  className="about-repository"
                  href="https://github.com/YoungWWan/mi-watch-agent-command-pilot"
                  target="_blank"
                  rel="noopener noreferrer"
                  aria-label="在浏览器中打开 GitHub 仓库"
                  title="GitHub 仓库"
                >
                  {/* GitHub Octicons mark-github; MIT notice: docs/licenses/octicons-MIT.txt. */}
                  <svg
                    width={24}
                    height={24}
                    viewBox="0 0 16 16"
                    fill="currentColor"
                    aria-hidden="true"
                    focusable="false"
                  >
                    <path d="M6.766 11.328c-2.063-.25-3.516-1.734-3.516-3.656 0-.781.281-1.625.75-2.188-.203-.515-.172-1.609.063-2.062.625-.078 1.468.25 1.968.703.594-.187 1.219-.281 1.985-.281.765 0 1.39.094 1.953.265.484-.437 1.344-.765 1.969-.687.218.422.25 1.515.046 2.047.5.593.766 1.39.766 2.203 0 1.922-1.453 3.375-3.547 3.64.531.344.89 1.094.89 1.954v1.625c0 .468.391.734.86.547C13.781 14.359 16 11.53 16 8.03 16 3.61 12.406 0 7.984 0 3.563 0 0 3.61 0 8.031a7.88 7.88 0 0 0 5.172 7.422c.422.156.828-.125.828-.547v-1.25c-.219.094-.5.156-.75.156-1.031 0-1.64-.562-2.078-1.609-.172-.422-.36-.672-.719-.719-.187-.015-.25-.093-.25-.187 0-.188.313-.328.625-.328.453 0 .844.281 1.25.86.313.452.64.655 1.031.655s.641-.14 1-.5c.266-.265.47-.5.657-.656" />
                  </svg>
                </a>
              </div>
              <DesktopUpdatePanel state={desktopUpdates.state} check={desktopUpdates.check} install={desktopUpdates.install} blocked={isInstalling || pendingCount > 0} />
              <div className="about-license">
                <h3>开源许可</h3>
                <p>
                  本软件以 GNU AGPL-3.0 分发，使用 AstroBox-NG 公共组件。感谢
                  AstralSight Studios 与开源贡献者。
                </p>
                <details className="disclosure">
                  <summary>
                    <span>组件与许可证</span>
                    <ChevronDown size={14} />
                  </summary>
                  <div className="license-table-wrap">
                    <table className="license-table">
                      <thead>
                        <tr>
                          <th>组件</th>
                          <th>用途</th>
                          <th>许可</th>
                        </tr>
                      </thead>
                      <tbody>
                        <tr>
                          <td>AstroBox Core</td>
                          <td>鉴权、MASS 与 RPK 部署 · 静态嵌入</td>
                          <td>AGPL-3.0</td>
                        </tr>
                        <tr>
                          <td>AstroBox BtClassicSpp</td>
                          <td>经典蓝牙通信 · Tauri 插件</td>
                          <td>AGPL-3.0</td>
                        </tr>
                        <tr>
                          <td>AstroBox Pb / MsgPack</td>
                          <td>协议序列化 · 静态依赖</td>
                          <td>AGPL-3.0</td>
                        </tr>
                        <tr>
                          <td>Axum HTTP Engine</td>
                          <td>局域网服务与配对 · Rust 内嵌</td>
                          <td>MIT</td>
                        </tr>
                      </tbody>
                    </table>
                  </div>
                </details>
                <p className="copyright">
                  © 2026 Young.
                  <br />
                  Portions © AstralSight Studios (AstroBox-NG)
                </p>
              </div>
            </section>
          )}
        </div>
      </main>

      {showQrModal && (
        <div className="modal-backdrop" onClick={closeLoginModal}>
          <section
            className="login-modal"
            role="dialog"
            aria-modal="true"
            aria-labelledby="login-title"
            ref={loginModalRef}
            onClick={(event) => event.stopPropagation()}
          >
            <div className="modal-heading">
              <div>
                <h2 id="login-title">登录小米账号</h2>
                <p className="section-note">同步设备与连接凭据。</p>
              </div>
              <button
                className="ghost icon-button"
                onClick={closeLoginModal}
                aria-label="关闭登录窗口"
              >
                <X size={18} />
              </button>
            </div>
            <div className="login-tabs" role="group" aria-label="登录方式">
              <button
                className={loginTab === "qr" ? "active" : ""}
                aria-pressed={loginTab === "qr"}
                onClick={() => {
                  setLoginTab("qr");
                  if (!qrSession && !isQrPolling) handleStartQrLogin();
                }}
              >
                <QrCode size={15} />
                扫码登录
              </button>
              <button
                className={loginTab === "password" ? "active" : ""}
                aria-pressed={loginTab === "password"}
                onClick={() => setLoginTab("password")}
              >
                <Lock size={14} />
                密码登录
              </button>
            </div>
            {loginTab === "qr" ? (
              <div className="qr-login">
                <div className="qr-code">
                  {qrSession?.qr_data_url ? (
                    <img src={qrSession.qr_data_url} alt="小米账号登录二维码" />
                  ) : (
                    <div className="qr-placeholder">
                      <RefreshCw
                        size={26}
                        className={isQrPolling ? "spinning" : ""}
                      />
                      <span>
                        {isQrPolling ? "加载二维码…" : "二维码加载失败"}
                      </span>
                    </div>
                  )}
                </div>
                <p
                  className={
                    qrPollingStatus.includes("失败")
                      ? "inline-error"
                      : "qr-status"
                  }
                  role="status"
                >
                  {qrPollingStatus}
                </p>
                <button
                  className="ghost"
                  onClick={handleStartQrLogin}
                  disabled={isQrPolling}
                >
                  <RefreshCw size={14} />
                  刷新二维码
                </button>
              </div>
            ) : (
              <form className="password-login" onSubmit={handlePasswordLogin}>
                <label htmlFor="xiaomi-user">小米账号 / 手机号 / 邮箱</label>
                <input
                  id="xiaomi-user"
                  autoComplete="username"
                  placeholder="输入账号"
                  value={pwdUsername}
                  onChange={(event) => setPwdUsername(event.target.value)}
                  disabled={pwdLoggingIn || !!pwdVerification}
                />
                <label htmlFor="xiaomi-password">密码</label>
                <input
                  id="xiaomi-password"
                  type="password"
                  autoComplete="current-password"
                  placeholder="输入密码"
                  value={pwdPassword}
                  onChange={(event) => setPwdPassword(event.target.value)}
                  disabled={pwdLoggingIn || !!pwdVerification}
                />
                {pwdVerification && (
                  <div className="verification-message" role="status">
                    <strong>等待小米安全验证</strong>
                    <p>{pwdVerification.message}</p>
                    <p className="section-note">完成后将自动登录。</p>
                    <button
                      type="button"
                      className="ghost"
                      onClick={() => setPwdVerification(null)}
                    >
                      取消验证
                    </button>
                  </div>
                )}
                {pwdErrorMsg && (
                  <p className="inline-error" role="alert">
                    <AlertCircle size={14} />
                    {pwdErrorMsg}
                  </p>
                )}
                <button
                  className="primary"
                  type="submit"
                  disabled={
                    pwdLoggingIn ||
                    !!pwdVerification ||
                    !pwdUsername.trim() ||
                    !pwdPassword
                  }
                >
                  {pwdLoggingIn ? (
                    <RefreshCw size={15} className="spinning" />
                  ) : (
                    <Lock size={14} />
                  )}
                  {pwdLoggingIn ? "登录中…" : "登录"}
                </button>
              </form>
            )}
            <p className="login-privacy">
              <Shield size={13} />
              通过小米官方通道登录，凭据仅存于本机。
            </p>
          </section>
        </div>
      )}
      {clipboardError && (
        <div className="feedback-toast" role="alert">
          {clipboardError}
          <button
            className="ghost icon-button"
            onClick={() => setClipboardError("")}
            aria-label="关闭提示"
          >
            <X size={14} />
          </button>
        </div>
      )}
    </div>
  );
}
