use crate::parse::{PortSpec, Protocol};
use crate::proxy;
use crate::udp;
use anyhow::{Result, bail};
use iroh::endpoint::{Connection, Incoming, RecvStream, SendStream};
use iroh::{Endpoint, SecretKey};
use std::collections::HashMap;
use std::collections::HashSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::{Mutex, oneshot};
use tokio::task::JoinSet;

const ALPN: &[u8] = b"punch/0";

#[derive(Clone, Debug)]
pub(crate) struct AllowedPorts {
    tcp: Arc<HashSet<u16>>,
    udp: Arc<HashSet<u16>>,
}

impl AllowedPorts {
    pub(crate) fn from_ports(ports: &[PortSpec]) -> Self {
        let tcp = ports
            .iter()
            .filter(|port| port.protocol == Protocol::Tcp)
            .map(|port| port.port())
            .collect();
        let udp = ports
            .iter()
            .filter(|port| port.protocol == Protocol::Udp)
            .map(|port| port.port())
            .collect();

        Self {
            tcp: Arc::new(tcp),
            udp: Arc::new(udp),
        }
    }
}

pub async fn run(ports: Vec<PortSpec>, secret_key: SecretKey) -> Result<()> {
    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;

    eprintln!("public key: {}", endpoint.id());

    let allowed = AllowedPorts::from_ports(&ports);

    while let Some(incoming) = endpoint.accept().await {
        let allowed = allowed.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(incoming, allowed).await {
                eprintln!("connection error: {e}");
            }
        });
    }

    Ok(())
}

async fn handle_connection(incoming: Incoming, allowed: AllowedPorts) -> Result<()> {
    let conn = incoming.await?;
    serve_connection(conn, allowed).await
}

pub(crate) async fn serve_connection(conn: Connection, allowed: AllowedPorts) -> Result<()> {
    let mut tasks = JoinSet::new();

    let tcp_allowed = allowed.tcp.clone();
    let tcp_conn = conn.clone();
    tasks.spawn(async move { run_tcp_accept_loop(tcp_conn, tcp_allowed).await });

    if !allowed.udp.is_empty() {
        let state = Arc::new(Mutex::new(ServerUdpState::default()));

        let udp_conn = conn.clone();
        let udp_allowed = allowed.udp.clone();
        let udp_state = state.clone();
        tasks.spawn(async move { run_udp_datagrams(udp_conn, udp_allowed, udp_state).await });

        tasks.spawn(async move { run_udp_cleanup(state).await });
    }

    supervise_tasks(conn.closed(), tasks).await
}

async fn run_tcp_accept_loop(conn: Connection, allowed: Arc<HashSet<u16>>) -> Result<()> {
    loop {
        let (send, recv) = conn.accept_bi().await?;
        let allowed = allowed.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_stream(send, recv, allowed).await {
                eprintln!("stream error: {e}");
            }
        });
    }
}

async fn handle_stream(
    send: SendStream,
    mut recv: RecvStream,
    allowed: Arc<HashSet<u16>>,
) -> Result<()> {
    let mut port_buf = [0u8; 2];
    recv.read_exact(&mut port_buf).await?;
    let port = u16::from_be_bytes(port_buf);

    if !allowed.contains(&port) {
        bail!("port {port} not in expose list");
    }

    let tcp = TcpStream::connect(("127.0.0.1", port)).await?;
    proxy::bidirectional(send, recv, tcp).await
}

#[derive(Default)]
struct ServerUdpState {
    flows: HashMap<u16, ServerUdpFlow>,
}

struct ServerUdpFlow {
    socket: Arc<UdpSocket>,
    last_activity: Instant,
    send_error_logged: bool,
    shutdown: Option<oneshot::Sender<()>>,
}

impl ServerUdpState {
    fn touch(&mut self, flow_id: u16, now: Instant) -> Option<Arc<UdpSocket>> {
        let flow = self.flows.get_mut(&flow_id)?;
        flow.last_activity = now;
        Some(flow.socket.clone())
    }

    fn mark_send_error(&mut self, flow_id: u16, now: Instant) -> bool {
        let Some(flow) = self.flows.get_mut(&flow_id) else {
            return false;
        };

        flow.last_activity = now;
        if flow.send_error_logged {
            return false;
        }

        flow.send_error_logged = true;
        true
    }

    fn expire_inactive(&mut self, now: Instant) -> Vec<oneshot::Sender<()>> {
        let expired: Vec<u16> = self
            .flows
            .iter()
            .filter_map(|(flow_id, flow)| {
                udp::is_expired(flow.last_activity, now).then_some(*flow_id)
            })
            .collect();

        expired
            .into_iter()
            .filter_map(|flow_id| self.flows.remove(&flow_id))
            .filter_map(|mut flow| flow.shutdown.take())
            .collect()
    }
}

async fn run_udp_datagrams(
    conn: Connection,
    allowed: Arc<HashSet<u16>>,
    state: Arc<Mutex<ServerUdpState>>,
) -> Result<()> {
    loop {
        let datagram = conn.read_datagram().await?;
        let datagram = match udp::decode_client_datagram(&datagram) {
            Ok(datagram) => datagram,
            Err(e) => {
                eprintln!("udp datagram error: {e}");
                continue;
            }
        };

        if !allowed.contains(&datagram.dest_port) {
            continue;
        }

        let socket =
            get_or_create_flow_socket(conn.clone(), state.clone(), datagram.flow_id).await?;
        let now = Instant::now();

        match socket
            .send_to(datagram.payload, ("127.0.0.1", datagram.dest_port))
            .await
        {
            Ok(_) => {
                let mut state = state.lock().await;
                if let Some(flow) = state.flows.get_mut(&datagram.flow_id) {
                    flow.last_activity = now;
                }
            }
            Err(e) => {
                let should_log = {
                    let mut state = state.lock().await;
                    state.mark_send_error(datagram.flow_id, now)
                };
                if should_log {
                    eprintln!("udp send error for flow {}: {e}", datagram.flow_id);
                }
            }
        }
    }
}

async fn get_or_create_flow_socket(
    conn: Connection,
    state: Arc<Mutex<ServerUdpState>>,
    flow_id: u16,
) -> Result<Arc<UdpSocket>> {
    let now = Instant::now();
    if let Some(socket) = {
        let mut state = state.lock().await;
        state.touch(flow_id, now)
    } {
        return Ok(socket);
    }

    let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await?);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    {
        let mut state = state.lock().await;
        if let Some(socket) = state.touch(flow_id, now) {
            return Ok(socket);
        }
        state.flows.insert(
            flow_id,
            ServerUdpFlow {
                socket: socket.clone(),
                last_activity: now,
                send_error_logged: false,
                shutdown: Some(shutdown_tx),
            },
        );
    }

    let reply_socket = socket.clone();
    tokio::spawn(async move {
        run_flow_replies(conn, state, flow_id, reply_socket, shutdown_rx).await;
    });

    Ok(socket)
}

async fn run_flow_replies(
    conn: Connection,
    state: Arc<Mutex<ServerUdpState>>,
    flow_id: u16,
    socket: Arc<UdpSocket>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    let mut buf = [0u8; udp::MAX_UDP_PACKET_SIZE];

    loop {
        tokio::select! {
            _ = conn.closed() => break,
            _ = &mut shutdown_rx => break,
            result = socket.recv_from(&mut buf) => match result {
                Ok((len, _)) => {
                    let now = Instant::now();
                    let active = {
                        let mut state = state.lock().await;
                        match state.flows.get_mut(&flow_id) {
                            Some(flow) => {
                                flow.last_activity = now;
                                true
                            }
                            None => false,
                        }
                    };

                    if !active {
                        break;
                    }

                    if let Err(e) = udp::send_server_datagram(&conn, flow_id, &buf[..len]) {
                        eprintln!("udp datagram error: {e}");
                    }
                }
                Err(e) => eprintln!("udp recv error: {e}"),
            }
        }
    }
}

async fn run_udp_cleanup(state: Arc<Mutex<ServerUdpState>>) -> Result<()> {
    let mut interval = tokio::time::interval(udp::FLOW_SWEEP_INTERVAL);
    loop {
        interval.tick().await;
        let shutdowns = {
            let mut state = state.lock().await;
            state.expire_inactive(Instant::now())
        };

        for shutdown in shutdowns {
            let _ = shutdown.send(());
        }
    }
}

async fn supervise_tasks(conn_closed: impl Future, mut tasks: JoinSet<Result<()>>) -> Result<()> {
    tokio::select! {
        _ = conn_closed => bail!("connection to remote peer lost"),
        result = tasks.join_next() => match result {
            Some(result) => result?,
            None => Ok(()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::{ServerUdpFlow, ServerUdpState};
    use crate::udp;
    use std::sync::Arc;
    use std::time::Instant;
    use tokio::net::UdpSocket;
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn expire_inactive_flows_removes_stale_entries() {
        let now = Instant::now();
        let socket = Arc::new(UdpSocket::bind("127.0.0.1:0").await.unwrap());
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();

        let mut state = ServerUdpState::default();
        state.flows.insert(
            1,
            ServerUdpFlow {
                socket,
                last_activity: now - udp::FLOW_IDLE_TIMEOUT - std::time::Duration::from_secs(1),
                send_error_logged: false,
                shutdown: Some(shutdown_tx),
            },
        );

        let shutdowns = state.expire_inactive(now);
        assert_eq!(shutdowns.len(), 1);
        assert!(state.flows.is_empty());
    }
}
