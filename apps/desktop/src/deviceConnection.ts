export interface DeviceDisconnected {
  mac: string;
}

export type DeviceConnectionStatus = "connecting" | "connected" | "disconnected";

interface ConnectionMonitor {
  getStatus: () => Promise<DeviceConnectionStatus>;
  listen: (callback: (event: DeviceDisconnected) => void) => Promise<() => void>;
  poll?: boolean;
  onDisconnected: () => void;
  onError: (error: unknown) => void;
}

export function monitorDeviceConnection(mac: string, monitor: ConnectionMonitor): () => void {
  let disposed = false;
  let disconnected = false;
  let checking = false;
  let checkAgain = false;
  let unlisten: (() => void) | undefined;

  const check = async () => {
    if (disposed || disconnected) return;
    if (checking) {
      checkAgain = true;
      return;
    }
    checking = true;
    try {
      const status = await monitor.getStatus();
      if (!disposed && !disconnected && status === "disconnected") {
        disconnected = true;
        if (interval !== undefined) clearInterval(interval);
        monitor.onDisconnected();
      }
    } catch (error) {
      if (!disposed) monitor.onError(error);
    } finally {
      checking = false;
      if (checkAgain && !disposed && !disconnected) {
        checkAgain = false;
        void check();
      }
    }
  };

  // Before connect_and_auth has registered its session, a status query could
  // report a false disconnect. Monitor attempts through events and the
  // command's cancellation/timeout; poll after authentication succeeds.
  const interval = monitor.poll === false ? undefined : setInterval(() => void check(), 2000);
  monitor.listen((event) => {
    if (event.mac.trim().toUpperCase() === mac.trim().toUpperCase()) {
      // Verify the current transport so a delayed event from an old session
      // cannot mark a successfully reconnected watch as disconnected.
      void check();
    }
  }).then((stop) => {
    if (disposed) {
      stop();
    } else {
      unlisten = stop;
      if (monitor.poll !== false) void check();
    }
  }).catch((error) => {
    if (!disposed) {
      monitor.onError(error);
      if (monitor.poll !== false) void check();
    }
  });

  return () => {
    disposed = true;
    if (interval !== undefined) clearInterval(interval);
    unlisten?.();
  };
}
