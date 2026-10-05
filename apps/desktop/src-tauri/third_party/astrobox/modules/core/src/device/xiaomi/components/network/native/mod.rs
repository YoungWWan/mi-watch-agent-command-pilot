use std::{
    fs::{self, File},
    future::Future,
    net::SocketAddr,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use anyhow::Result;
use chrono::Local;
use etherparse::{Icmpv4Header, Icmpv4Type};
use ipstack::{IpNumber, IpStack, IpStackConfig, IpStackStream};
use pb::xiaomi::protocol;
use pcap_file::pcap::PcapWriter;
use tokio::sync::mpsc::error::TrySendError;
use tokio::{
    io::{self, AsyncRead, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    runtime::Handle,
    sync::{mpsc, watch},
    task::JoinSet,
    time::timeout,
};
use tokio_util::sync::PollSender;
use udp_stream::UdpStream;

use crate::{
    anyhow_site,
    device::xiaomi::{
        XiaomiDevice,
        config::NetworkConfig,
        packet::{
            self,
            v2::layer2::{L2Channel, L2OpCode, L2Packet},
        },
        system::{XiaomiSystemExt, register_xiaomi_system_ext_on_l2packet},
    },
    ecs::{Component, access::with_device_component_mut},
};
use parking_lot::Mutex;

mod dhcp;
mod meter;
mod tun;

use dhcp::maybe_build_reply;
use meter::BandwidthMeter;
use tun::MiWearTunDevice;

const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
struct SessionAddress {
    id: usize,
    local: SocketAddr,
    remote: SocketAddr,
}

struct ActiveSession {
    protocol: &'static str,
    address: SessionAddress,
    counter: Arc<AtomicUsize>,
}

impl ActiveSession {
    fn new(protocol: &'static str, address: SessionAddress, counter: Arc<AtomicUsize>) -> Self {
        let SessionAddress { id, local, remote } = address;
        let count = counter.fetch_add(1, Ordering::Relaxed) + 1;
        log::info!(
            "[NetworkRuntime] {protocol}#{id} established {local} -> {remote}, sessions={count}"
        );
        Self {
            protocol,
            address,
            counter,
        }
    }
}

impl Drop for ActiveSession {
    fn drop(&mut self) {
        let protocol = self.protocol;
        let SessionAddress { id, local, remote } = self.address;
        let remaining = self.counter.fetch_sub(1, Ordering::Relaxed) - 1;
        log::info!(
            "[NetworkRuntime] {protocol}#{id} closed, sessions={remaining} ({local} -> {remote})"
        );
    }
}

// Schedule the dial as well as the relay: awaiting a dial in the accept loop
// stalls all subsequent TCP, UDP and ICMP traffic from this watch.
fn spawn_tcp_session<S, F>(
    sessions: &mut JoinSet<()>,
    mut tcp: S,
    connect: F,
    address: SessionAddress,
    counter: Arc<AtomicUsize>,
    connect_timeout: Duration,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    F: Future<Output = io::Result<TcpStream>> + Send + 'static,
{
    sessions.spawn(async move {
        let SessionAddress { id, local, remote } = address;
        let mut peer = match timeout(connect_timeout, connect).await {
            Ok(Ok(peer)) => peer,
            Ok(Err(err)) => {
                log::warn!("[NetworkRuntime] TCP#{id} connect failed {local} -> {remote}: {err}");
                return;
            }
            Err(_) => {
                log::warn!(
                    "[NetworkRuntime] TCP#{id} connect timed out after {}s ({local} -> {remote})",
                    connect_timeout.as_secs()
                );
                return;
            }
        };
        let _session = ActiveSession::new("TCP", address, counter);
        if let Err(err) = io::copy_bidirectional(&mut tcp, &mut peer).await {
            log::info!("[NetworkRuntime] TCP#{id} ended with error: {err} ({local} -> {remote})");
        }
        let _ = peer.shutdown().await;
        let _ = tcp.shutdown().await;
    });
}

#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct NetWorkSpeed {
    pub write: f64,
    pub read: f64,
}

#[derive(Component, serde::Serialize)]
pub struct NetworkComponent {
    #[serde(skip_serializing)]
    config: NetworkConfig,
}

impl NetworkComponent {
    pub fn new(config: NetworkConfig) -> Self {
        Self { config }
    }

    pub fn config(&self) -> &NetworkConfig {
        &self.config
    }
}

#[derive(Component)]
pub struct NetworkSystem {
    owner_id: String,
    runtime: Mutex<Option<NetworkRuntime>>,
    meter: Mutex<Option<BandwidthMeter>>,
}

impl Default for NetworkSystem {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl NetworkSystem {
    pub fn new(owner_id: String) -> Self {
        register_xiaomi_system_ext_on_l2packet::<Self>();
        Self {
            owner_id,
            runtime: Mutex::new(None),
            meter: Mutex::new(None),
        }
    }

    pub fn ensure_runtime(&mut self, handle: Handle, config: NetworkConfig) -> Result<()> {
        if self.runtime.lock().is_some() {
            return Ok(());
        }
        if self.owner_id.is_empty() {
            return Err(anyhow_site!("NetworkSystem missing owner"));
        }
        let meter_window = Duration::from_secs(config.meter_window_secs.max(1));
        let meter = BandwidthMeter::new(meter_window);
        let runtime = NetworkRuntime::new(self.owner_id.clone(), config, handle, &meter)?;
        *self.runtime.lock() = Some(runtime);
        *self.meter.lock() = Some(meter);
        Ok(())
    }

    pub fn sync_network_status(&mut self) -> Result<()> {
        with_device_component_mut::<XiaomiDevice, _, _>(self.owner_id.clone(), |dev| {
            packet::cipher::enqueue_pb_packet(
                dev,
                build_sync_network_status(),
                "NetworkComponent::sync_network_status",
            );
        })
        .map_err(|err| anyhow_site!("failed to access resource config: {:?}", err))?;

        Ok(())
    }

    pub fn get_speed(&self) -> NetWorkSpeed {
        let meter = self.meter.lock().as_ref().unwrap().clone();
        NetWorkSpeed {
            write: meter.write_speed(),
            read: meter.read_speed(),
        }
    }
}

impl XiaomiSystemExt for NetworkSystem {
    fn on_layer2_packet(&mut self, channel: L2Channel, _opcode: L2OpCode, payload: &[u8]) {
        if channel != L2Channel::Network {
            return;
        }

        if log::log_enabled!(log::Level::Trace) {
            log::trace!(
                "[NetworkSystem] received network packet: {}",
                crate::tools::to_hex_string(payload)
            );
        } else {
            log::debug!(
                "[NetworkSystem] received network packet ({} bytes)",
                payload.len()
            );
        }

        if let Some(runtime) = self.runtime.lock().as_ref() {
            if let Err(err) = runtime.push_inbound(payload.to_vec()) {
                match err {
                    IngressError::Closed => {
                        log::debug!("[NetworkSystem] ingress closed, dropping packet");
                    }
                    IngressError::Backpressure => {
                        log::warn!("[NetworkSystem] ingress overwhelmed, dropping packet");
                    }
                }
            }
        } else {
            log::trace!("[NetworkSystem] runtime not ready; dropping network packet");
        }
    }
}

fn build_sync_network_status() -> protocol::WearPacket {
    let network_status = protocol::NetworkStatus { capability: 2 };

    let pkt_payload = protocol::System {
        payload: Some(protocol::system::Payload::NetworkStatus(network_status)),
    };

    let pkt = protocol::WearPacket {
        r#type: protocol::wear_packet::Type::System as i32,
        id: protocol::system::SystemId::SyncNetworkStatus as u32,
        payload: Some(protocol::wear_packet::Payload::System(pkt_payload)),
    };

    pkt
}

struct NetworkRuntime {
    ingress_tx: mpsc::Sender<Vec<u8>>,
    shutdown: watch::Sender<bool>,
    tasks: Vec<crate::asyncrt::TaskHandle>,
}

impl NetworkRuntime {
    fn new(
        owner: String,
        config: NetworkConfig,
        handle: Handle,
        meter: &BandwidthMeter,
    ) -> Result<Self> {
        let ingress_capacity = config.ingress_buffer.max(1);
        let tun_capacity = config.tun_buffer.max(1);
        let outbound_capacity = config.outbound_buffer.max(1);

        let (ingress_tx, mut ingress_rx) = mpsc::channel::<Vec<u8>>(ingress_capacity);
        let (tun_tx, tun_rx) = mpsc::channel::<Vec<u8>>(tun_capacity);
        let (send_tx, mut send_rx) = mpsc::channel::<Vec<u8>>(outbound_capacity);
        let capture = prepare_capture_writer(&owner, &config);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let mut tasks = Vec::new();

        // 入口循环：设备 -> 协议栈（带 DHCP 处理）
        {
            let owner_clone = owner.clone();
            let mut shutdown = shutdown_rx.clone();
            tasks.push(crate::asyncrt::spawn_with_handle(
                async move {
                    loop {
                        tokio::select! {
                            packet = ingress_rx.recv() => {
                                match packet {
                                    Some(data) => {
                                        match maybe_build_reply(&data) {
                                            Ok(Some(reply)) => {
                                                if let Err(err) = enqueue_network_payload(&owner_clone, reply).await {
                                                    log::error!("[NetworkRuntime] failed to send DHCP reply: {err:?}");
                                                }
                                                continue;
                                            }
                                            Ok(None) => {}
                                            Err(err) => log::warn!("[NetworkRuntime] DHCP parse error: {err:?}"),
                                        }

                                        if let Err(err) = tun_tx.send(data).await {
                                            log::warn!("[NetworkRuntime] tun channel closed: {err}");
                                            break;
                                        }
                                    }
                                    None => break,
                                }
                            }
                            changed = shutdown.changed() => {
                                if changed.is_ok() {
                                    break;
                                }
                            }
                        }
                    }
                },
                handle.clone(),
            ));
        }

        // 发送循环：协议栈 -> 设备
        {
            let owner_clone = owner.clone();
            let mut shutdown = shutdown_rx.clone();
            tasks.push(crate::asyncrt::spawn_with_handle(
                async move {
                    loop {
                        tokio::select! {
                            packet = send_rx.recv() => {
                                match packet {
                                    Some(payload) => {
                                        if let Err(err) = enqueue_network_payload(&owner_clone, payload).await {
                                            log::error!("[NetworkRuntime] failed to send network payload: {err:?}");
                                            break;
                                        }
                                    }
                                    None => break,
                                }
                            }
                            changed = shutdown.changed() => {
                                if changed.is_ok() {
                                    break;
                                }
                            }
                        }
                    }
                },
                handle.clone(),
            ));
        }

        // IpStack 处理循环
        {
            let owner_clone = owner.clone();
            let mut shutdown = shutdown_rx.clone();
            let meter_for_stack = meter.clone();
            let capture = capture;
            let send_tx_clone = send_tx.clone();
            let config_for_stack = config.clone();
            tasks.push(crate::asyncrt::spawn_with_handle(
                async move {
                    let session_count = Arc::new(AtomicUsize::new(0));
                    let serial = Arc::new(AtomicUsize::new(0));
                    // Dropping the stack task also aborts every pending dial and
                    // active relay, so no sessions survive a device disconnect.
                    let mut sessions = JoinSet::new();
                    let poll_sender = PollSender::new(send_tx_clone);
                    let tun_device = MiWearTunDevice {
                        rx: tun_rx,
                        tx_send: poll_sender,
                        capture,
                        meter: meter_for_stack,
                    };
                    let mut stack_cfg = IpStackConfig::default();
                    stack_cfg.mtu(config_for_stack.mtu);
                    let mut ip_stack = IpStack::new(stack_cfg, tun_device);
                    log::info!(
                        "[NetworkRuntime] network stack started for {} (mtu={})",
                        owner_clone,
                        config_for_stack.mtu
                    );
                    loop {
                        tokio::select! {
                            accept_res = ip_stack.accept() => {
                                match accept_res {
                                    Ok(stream) => {
                                        let id = serial.fetch_add(1, Ordering::Relaxed);
                                        match stream {
                                            IpStackStream::Tcp(tcp) => {
                                                let address = SessionAddress {
                                                    id,
                                                    local: tcp.local_addr(),
                                                    remote: tcp.peer_addr(),
                                                };
                                                spawn_tcp_session(
                                                    &mut sessions,
                                                    tcp,
                                                    TcpStream::connect(address.remote),
                                                    address,
                                                    session_count.clone(),
                                                    TCP_CONNECT_TIMEOUT,
                                                );
                                            }
                                            IpStackStream::Udp(mut udp) => {
                                                let local_addr = udp.local_addr();
                                                let remote_addr = udp.peer_addr();
                                                let counter = session_count.clone();
                                                sessions.spawn(async move {
                                                    let mut peer = match UdpStream::connect(remote_addr).await {
                                                        Ok(stream) => stream,
                                                        Err(err) => {
                                                            log::warn!("[NetworkRuntime] UDP#{id} connect failed {local_addr} -> {remote_addr}: {err}");
                                                            return;
                                                        }
                                                    };
                                                    let _session = ActiveSession::new(
                                                        "UDP",
                                                        SessionAddress { id, local: local_addr, remote: remote_addr },
                                                        counter,
                                                    );
                                                    if let Err(err) = io::copy_bidirectional(&mut udp, &mut peer).await {
                                                        log::info!(
                                                            "[NetworkRuntime] UDP#{id} ended with error: {err} ({} -> {})",
                                                            local_addr,
                                                            remote_addr
                                                        );
                                                    }
                                                    peer.shutdown();
                                                    let _ = udp.shutdown().await;
                                                });
                                            }
                                            IpStackStream::UnknownTransport(pkt) => {
                                                if pkt.src_addr().is_ipv4()
                                                    && pkt.ip_protocol() == IpNumber::ICMP
                                                {
                                                    if let Ok((header, payload)) =
                                                        Icmpv4Header::from_slice(pkt.payload())
                                                    {
                                                        if let Icmpv4Type::EchoRequest(echo) =
                                                            header.icmp_type
                                                        {
                                                            let mut response = Icmpv4Header::new(
                                                                Icmpv4Type::EchoReply(echo),
                                                            );
                                                            response.update_checksum(payload);
                                                            let mut bytes =
                                                                response.to_bytes().to_vec();
                                                            bytes.extend_from_slice(payload);
                                                            if let Err(err) = pkt.send(bytes) {
                                                                log::warn!(
                                                                    "[NetworkRuntime] ICMP send failed: {err}"
                                                                );
                                                            } else {
                                                                log::info!(
                                                                    "[NetworkRuntime] ICMP echo replied"
                                                                );
                                                            }
                                                            continue;
                                                        }
                                                    }
                                                }
                                                log::debug!(
                                                    "[NetworkRuntime] unknown transport {:?}",
                                                    pkt.ip_protocol()
                                                );
                                            }
                                            IpStackStream::UnknownNetwork(pkt) => {
                                                log::debug!("[NetworkRuntime] unknown network payload ({} bytes)", pkt.len());
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        log::error!("[NetworkRuntime] IpStack halted with error: {err}");
                                        break;
                                    }
                                }
                            }
                            result = sessions.join_next(), if !sessions.is_empty() => {
                                if let Some(Err(err)) = result {
                                    log::warn!("[NetworkRuntime] session task failed: {err}");
                                }
                            }
                            _ = shutdown.changed() => {
                                log::info!("[NetworkRuntime] shutting down network stack for {}", owner_clone);
                                break;
                            }
                        }
                    }
                },
                handle,
            ));
        }

        Ok(Self {
            ingress_tx,
            shutdown: shutdown_tx,
            tasks,
        })
    }

    fn push_inbound(&self, packet: Vec<u8>) -> Result<(), IngressError> {
        self.ingress_tx.try_send(packet).map_err(|err| match err {
            TrySendError::Full(_) => IngressError::Backpressure,
            TrySendError::Closed(_) => IngressError::Closed,
        })
    }
}

impl Drop for NetworkRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        for task in self.tasks.drain(..) {
            #[cfg(not(target_arch = "wasm32"))]
            task.abort();
        }
    }
}

#[derive(Debug)]
enum IngressError {
    Backpressure,
    Closed,
}

async fn enqueue_network_payload(owner: &str, payload: Vec<u8>) -> Result<()> {
    let owner_id = owner.to_string();
    crate::ecs::with_rt_mut(move |rt| {
        rt.with_device_mut(&owner_id, |world, entity| {
            let dev = world
                .get_mut::<XiaomiDevice>(entity)
                .ok_or_else(|| anyhow_site!("device {} not found for network send", owner_id))?;
            dev.sar
                .lock()
                .enqueue(L2Packet::new(L2Channel::Network, L2OpCode::Write, payload).to_bytes());
            Ok(())
        })
        .ok_or_else(|| anyhow_site!("device {} not found for network send", owner_id))?
    })
    .await
}

fn prepare_capture_writer(owner: &str, config: &NetworkConfig) -> Option<PcapWriter<File>> {
    if !config.enable_capture {
        return None;
    }
    let base_dir: PathBuf = config
        .capture_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("astrobox_rslogs"));
    if let Err(err) = fs::create_dir_all(&base_dir) {
        log::warn!(
            "[NetworkRuntime] failed to create pcap dir {}: {}",
            base_dir.display(),
            err
        );
        return None;
    }
    let timestamp = Local::now().format("%Y%m%d_%H%M%S");
    let sanitized_owner = owner.replace(':', "_");
    let file_path = base_dir.join(format!("{sanitized_owner}_{timestamp}.pcap"));
    match File::create(&file_path) {
        Ok(file) => match PcapWriter::new(file) {
            Ok(writer) => Some(writer),
            Err(err) => {
                log::warn!(
                    "[NetworkRuntime] failed to initialise pcap writer {}: {}",
                    file_path.display(),
                    err
                );
                None
            }
        },
        Err(err) => {
            log::warn!(
                "[NetworkRuntime] failed to create pcap file {}: {}",
                file_path.display(),
                err
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{pending, ready};
    use tokio::{io::AsyncReadExt, net::TcpListener, sync::oneshot};

    const TEST_DEADLINE: Duration = Duration::from_secs(2);

    fn address(id: usize, remote: SocketAddr) -> SessionAddress {
        SessionAddress {
            id,
            local: "10.1.10.2:20968".parse().unwrap(),
            remote,
        }
    }

    #[tokio::test]
    async fn stalled_connect_does_not_block_another_tcp_session() {
        let mut sessions = JoinSet::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let (_slow_watch, slow_tcp) = io::duplex(64);
        let (started, wait_started) = oneshot::channel();
        spawn_tcp_session(
            &mut sessions,
            slow_tcp,
            async move {
                let _ = started.send(());
                pending().await
            },
            address(0, "127.0.0.1:1".parse().unwrap()),
            counter.clone(),
            TCP_CONNECT_TIMEOUT,
        );
        timeout(TEST_DEADLINE, wait_started).await.unwrap().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote = listener.local_addr().unwrap();
        let (mut watch, tcp) = io::duplex(64);
        spawn_tcp_session(
            &mut sessions,
            tcp,
            TcpStream::connect(remote),
            address(1, remote),
            counter.clone(),
            TCP_CONNECT_TIMEOUT,
        );
        timeout(TEST_DEADLINE, async {
            let server = async {
                let (mut peer, _) = listener.accept().await.unwrap();
                let mut request = [0; 4];
                peer.read_exact(&mut request).await.unwrap();
                assert_eq!(&request, b"ping");
                peer.write_all(b"pong").await.unwrap();
                peer.shutdown().await.unwrap();
            };
            let client = async {
                watch.write_all(b"ping").await.unwrap();
                watch.shutdown().await.unwrap();
                let mut response = Vec::new();
                watch.read_to_end(&mut response).await.unwrap();
                assert_eq!(&response, b"pong");
            };
            tokio::join!(server, client);
            sessions.join_next().await.unwrap().unwrap();
        })
        .await
        .expect("a stalled dial must not delay another session");
        assert_eq!(counter.load(Ordering::Relaxed), 0);
        assert_eq!(sessions.len(), 1, "the slow dial should still be pending");
        sessions.shutdown().await;
    }

    #[tokio::test]
    async fn connect_timeout_closes_watch_stream_without_counting_a_session() {
        let mut sessions = JoinSet::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let (mut watch, tcp) = io::duplex(64);
        spawn_tcp_session(
            &mut sessions,
            tcp,
            pending(),
            address(0, "127.0.0.1:1".parse().unwrap()),
            counter.clone(),
            Duration::from_millis(20),
        );
        timeout(TEST_DEADLINE, sessions.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let mut buf = [0; 1];
        assert_eq!(
            timeout(TEST_DEADLINE, watch.read(&mut buf))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn failed_connect_closes_watch_stream_without_counting_a_session() {
        let mut sessions = JoinSet::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let (mut watch, tcp) = io::duplex(64);
        spawn_tcp_session(
            &mut sessions,
            tcp,
            ready(Err(io::Error::from(std::io::ErrorKind::ConnectionRefused))),
            address(0, "127.0.0.1:1".parse().unwrap()),
            counter.clone(),
            TCP_CONNECT_TIMEOUT,
        );
        timeout(TEST_DEADLINE, sessions.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let mut buf = [0; 1];
        assert_eq!(
            timeout(TEST_DEADLINE, watch.read(&mut buf))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn dropping_sessions_cancels_pending_connect() {
        let mut sessions = JoinSet::new();
        let (mut watch, tcp) = io::duplex(64);
        let (started, wait_started) = oneshot::channel();
        spawn_tcp_session(
            &mut sessions,
            tcp,
            async move {
                let _ = started.send(());
                pending().await
            },
            address(0, "127.0.0.1:1".parse().unwrap()),
            Arc::new(AtomicUsize::new(0)),
            TCP_CONNECT_TIMEOUT,
        );
        timeout(TEST_DEADLINE, wait_started).await.unwrap().unwrap();
        drop(sessions);
        let mut buf = [0; 1];
        assert_eq!(
            timeout(TEST_DEADLINE, watch.read(&mut buf))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn dropping_sessions_closes_active_relay_and_releases_session_count() {
        let mut sessions = JoinSet::new();
        let counter = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let remote = listener.local_addr().unwrap();
        let (mut watch, tcp) = io::duplex(64);
        spawn_tcp_session(
            &mut sessions,
            tcp,
            TcpStream::connect(remote),
            address(0, remote),
            counter.clone(),
            TCP_CONNECT_TIMEOUT,
        );
        let (mut peer, _) = timeout(TEST_DEADLINE, listener.accept())
            .await
            .unwrap()
            .unwrap();
        watch.write_all(b"ping").await.unwrap();
        let mut request = [0; 4];
        timeout(TEST_DEADLINE, peer.read_exact(&mut request))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(&request, b"ping");
        assert_eq!(counter.load(Ordering::Relaxed), 1);

        drop(sessions);
        let mut buf = [0; 1];
        assert_eq!(
            timeout(TEST_DEADLINE, watch.read(&mut buf))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(
            timeout(TEST_DEADLINE, peer.read(&mut buf))
                .await
                .unwrap()
                .unwrap(),
            0
        );
        assert_eq!(counter.load(Ordering::Relaxed), 0);
    }
}
