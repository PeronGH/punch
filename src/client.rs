use crate::parse::{LocalTarget, Mapping, Protocol};
use crate::proxy;
use crate::udp;
use anyhow::{Result, bail};
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointId, SecretKey};
use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::Mutex;
use tokio::task::JoinSet;

const ALPN: &[u8] = b"punch/0";

pub async fn run(
    endpoint_id: EndpointId,
    mappings: Vec<Mapping>,
    secret_key: SecretKey,
) -> Result<()> {
    let endpoint = Endpoint::builder().secret_key(secret_key).bind().await?;

    let conn = endpoint.connect(endpoint_id, ALPN).await?;
    run_connection(conn, mappings).await
}

pub(crate) async fn run_connection(conn: Connection, mappings: Vec<Mapping>) -> Result<()> {
    let mut tasks = JoinSet::new();
    let mut udp_mappings = Vec::new();

    for mapping in mappings {
        match (mapping.local, mapping.protocol) {
            (LocalTarget::Port(_), Protocol::Tcp) => {
                let conn = conn.clone();
                tasks.spawn(async move { run_listener(conn, mapping).await });
            }
            (LocalTarget::Port(local_port), Protocol::Udp) => {
                let socket = Arc::new(UdpSocket::bind(("127.0.0.1", local_port)).await?);
                udp_mappings.push(UdpMappingState {
                    local_port,
                    remote_port: mapping.remote,
                    socket,
                });
            }
            (LocalTarget::Stdio, Protocol::Tcp) => bail!("stdio mappings are not supported yet"),
            (LocalTarget::Stdio, Protocol::Udp) => unreachable!("udp stdio mappings are rejected during parsing"),
        }
    }

    if !udp_mappings.is_empty() {
        let udp_mappings = Arc::new(udp_mappings);
        let state = Arc::new(Mutex::new(ClientUdpState::default()));

        for (mapping_index, mapping) in udp_mappings.iter().cloned().enumerate() {
            let conn = conn.clone();
            let state = state.clone();
            tasks.spawn(async move { run_udp_mapping(conn, mapping, mapping_index, state).await });
        }

        let udp_conn = conn.clone();
        let udp_state = state.clone();
        let udp_mappings_reader = udp_mappings.clone();
        tasks
            .spawn(async move { run_udp_receiver(udp_conn, udp_mappings_reader, udp_state).await });

        tasks.spawn(async move { run_udp_cleanup(state).await });
    }

    supervise_tasks(conn.closed(), tasks).await
}

async fn run_listener(conn: Connection, mapping: Mapping) -> Result<()> {
    let LocalTarget::Port(local_port) = mapping.local else {
        bail!("stdio mappings are not supported yet");
    };

    let listener = TcpListener::bind(("127.0.0.1", local_port)).await?;
    loop {
        let (tcp, _) = listener.accept().await?;
        let conn = conn.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_stream(conn, mapping.remote, tcp).await {
                eprintln!("stream error: {e}");
            }
        });
    }
}

async fn handle_stream(conn: Connection, remote_port: u16, tcp: TcpStream) -> Result<()> {
    let (mut send, recv) = conn.open_bi().await?;
    send.write_all(&remote_port.to_be_bytes()).await?;
    proxy::bidirectional(send, recv, tcp).await
}

#[derive(Clone)]
struct UdpMappingState {
    local_port: u16,
    remote_port: u16,
    socket: Arc<UdpSocket>,
}

#[derive(Default)]
struct ClientUdpState {
    next_flow_id: u16,
    by_id: HashMap<u16, ClientUdpFlow>,
    by_sender: HashMap<(usize, SocketAddr), u16>,
}

struct ClientUdpFlow {
    mapping_index: usize,
    client_addr: SocketAddr,
    last_activity: Instant,
}

impl ClientUdpState {
    fn flow_id_for_sender(
        &mut self,
        mapping_index: usize,
        client_addr: SocketAddr,
        now: Instant,
    ) -> Option<u16> {
        if let Some(flow_id) = self.by_sender.get(&(mapping_index, client_addr)).copied() {
            if let Some(flow) = self.by_id.get_mut(&flow_id) {
                flow.last_activity = now;
                return Some(flow_id);
            }
            self.by_sender.remove(&(mapping_index, client_addr));
        }

        for _ in 0..=u16::MAX as usize {
            let flow_id = self.next_flow_id;
            self.next_flow_id = self.next_flow_id.wrapping_add(1);

            if self.by_id.contains_key(&flow_id) {
                continue;
            }

            self.by_id.insert(
                flow_id,
                ClientUdpFlow {
                    mapping_index,
                    client_addr,
                    last_activity: now,
                },
            );
            self.by_sender.insert((mapping_index, client_addr), flow_id);
            return Some(flow_id);
        }

        None
    }

    fn route_reply(&mut self, flow_id: u16, now: Instant) -> Option<(usize, SocketAddr)> {
        let flow = self.by_id.get_mut(&flow_id)?;
        flow.last_activity = now;
        Some((flow.mapping_index, flow.client_addr))
    }

    fn expire_inactive(&mut self, now: Instant) {
        let expired: Vec<(u16, usize, SocketAddr)> = self
            .by_id
            .iter()
            .filter_map(|(flow_id, flow)| {
                udp::is_expired(flow.last_activity, now).then_some((
                    *flow_id,
                    flow.mapping_index,
                    flow.client_addr,
                ))
            })
            .collect();

        for (flow_id, mapping_index, client_addr) in expired {
            self.by_id.remove(&flow_id);
            self.by_sender.remove(&(mapping_index, client_addr));
        }
    }
}

async fn run_udp_mapping(
    conn: Connection,
    mapping: UdpMappingState,
    mapping_index: usize,
    state: Arc<Mutex<ClientUdpState>>,
) -> Result<()> {
    let mut buf = [0u8; udp::MAX_UDP_PACKET_SIZE];

    loop {
        let (len, client_addr) = match mapping.socket.recv_from(&mut buf).await {
            Ok(result) => result,
            Err(e) => {
                eprintln!("udp recv error: {e}");
                continue;
            }
        };

        let flow_id = {
            let mut state = state.lock().await;
            match state.flow_id_for_sender(mapping_index, client_addr, Instant::now()) {
                Some(flow_id) => flow_id,
                None => {
                    eprintln!(
                        "udp flow table exhausted for local port {}",
                        mapping.local_port
                    );
                    continue;
                }
            }
        };

        if let Err(e) = udp::send_client_datagram(&conn, flow_id, mapping.remote_port, &buf[..len])
        {
            eprintln!("udp datagram error: {e}");
        }
    }
}

async fn run_udp_receiver(
    conn: Connection,
    mappings: Arc<Vec<UdpMappingState>>,
    state: Arc<Mutex<ClientUdpState>>,
) -> Result<()> {
    loop {
        let datagram = conn.read_datagram().await?;
        let datagram = match udp::decode_server_datagram(&datagram) {
            Ok(datagram) => datagram,
            Err(e) => {
                eprintln!("udp datagram error: {e}");
                continue;
            }
        };

        let Some((mapping_index, client_addr)) = ({
            let mut state = state.lock().await;
            state.route_reply(datagram.flow_id, Instant::now())
        }) else {
            continue;
        };

        if let Err(e) = mappings[mapping_index]
            .socket
            .send_to(datagram.payload, client_addr)
            .await
        {
            eprintln!("udp send error: {e}");
        }
    }
}

async fn run_udp_cleanup(state: Arc<Mutex<ClientUdpState>>) -> Result<()> {
    let mut interval = tokio::time::interval(udp::FLOW_SWEEP_INTERVAL);
    loop {
        interval.tick().await;
        let mut state = state.lock().await;
        state.expire_inactive(Instant::now());
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
    use super::{ClientUdpState, run_connection, supervise_tasks};
    use crate::parse::{Mapping, PortSpec};
    use crate::server::{self, AllowedPorts};
    use crate::udp;
    use anyhow::Result;
    use iroh::{Endpoint, SecretKey};
    use std::net::{Ipv4Addr, SocketAddr};
    use std::time::{Duration, Instant};
    use tokio::net::{TcpListener, UdpSocket};
    use tokio::sync::oneshot;
    use tokio::task::JoinSet;
    use tokio::time::{sleep, timeout};

    #[tokio::test]
    async fn bind_failure_is_not_hidden_by_first_listener() -> Result<()> {
        let probe = TcpListener::bind("127.0.0.1:0").await?;
        let local_port = probe.local_addr()?.port();
        drop(probe);

        let (ready_tx, ready_rx) = oneshot::channel();
        let mut tasks = JoinSet::new();

        tasks.spawn(async move {
            let _listener = TcpListener::bind(("127.0.0.1", local_port)).await?;
            ready_tx.send(()).ok();
            std::future::pending::<Result<()>>().await
        });

        ready_rx.await.expect("first listener should bind");

        tasks.spawn(async move {
            let _listener = TcpListener::bind(("127.0.0.1", local_port)).await?;
            Ok(())
        });

        let result = timeout(
            Duration::from_secs(1),
            supervise_tasks(std::future::pending::<()>(), tasks),
        )
        .await;

        assert!(result.is_ok(), "listener supervision hung");
        assert!(result.unwrap().is_err(), "bind failure should surface");
        Ok(())
    }

    #[test]
    fn udp_flow_timeout_evicts_inactive_senders() {
        let now = Instant::now();
        let sender = SocketAddr::from((Ipv4Addr::LOCALHOST, 42_000));
        let mut state = ClientUdpState::default();

        let flow_id = state.flow_id_for_sender(0, sender, now).unwrap();
        assert_eq!(state.route_reply(flow_id, now).unwrap(), (0, sender));

        state.expire_inactive(now + udp::FLOW_IDLE_TIMEOUT - Duration::from_secs(1));
        assert!(state.by_id.contains_key(&flow_id));

        state.expire_inactive(now + udp::FLOW_IDLE_TIMEOUT + Duration::from_secs(1));
        assert!(state.by_id.is_empty());
        assert!(state.by_sender.is_empty());
    }

    #[tokio::test]
    async fn udp_mapping_routes_replies_to_the_correct_sender() -> Result<()> {
        let echo_socket = UdpSocket::bind("127.0.0.1:0").await?;
        let echo_port = echo_socket.local_addr()?.port();
        let echo_task = tokio::spawn(async move {
            let mut buf = [0u8; udp::MAX_UDP_PACKET_SIZE];
            loop {
                let (len, addr) = echo_socket.recv_from(&mut buf).await.unwrap();
                echo_socket.send_to(&buf[..len], addr).await.unwrap();
            }
        });

        let server_key = SecretKey::generate(&mut rand::rng());
        let server_endpoint = Endpoint::builder()
            .secret_key(server_key)
            .alpns(vec![super::ALPN.to_vec()])
            .bind()
            .await?;

        let allowed = AllowedPorts::from_ports(&[format!("{echo_port}/udp").parse::<PortSpec>()?]);

        let server_addr = server_endpoint.addr();
        let server_task = tokio::spawn(async move {
            let incoming = server_endpoint.accept().await.unwrap();
            let conn = incoming.await.unwrap();
            let _ = server::serve_connection(conn, allowed).await;
        });

        let client_key = SecretKey::generate(&mut rand::rng());
        let client_endpoint = Endpoint::builder().secret_key(client_key).bind().await?;

        let conn = client_endpoint.connect(server_addr, super::ALPN).await?;

        let probe = UdpSocket::bind("127.0.0.1:0").await?;
        let local_port = probe.local_addr()?.port();
        drop(probe);

        let mapping: Mapping = format!("{local_port}:{echo_port}/udp").parse()?;
        let client_task = tokio::spawn(async move {
            let _ = run_connection(conn, vec![mapping]).await;
        });

        sleep(Duration::from_millis(100)).await;

        let sender_one = UdpSocket::bind("127.0.0.1:0").await?;
        let sender_two = UdpSocket::bind("127.0.0.1:0").await?;

        sender_one
            .send_to(b"alpha", ("127.0.0.1", local_port))
            .await?;
        sender_two
            .send_to(b"beta", ("127.0.0.1", local_port))
            .await?;

        let mut buf = [0u8; udp::MAX_UDP_PACKET_SIZE];
        let (len, _) = timeout(Duration::from_secs(5), sender_one.recv_from(&mut buf)).await??;
        assert_eq!(&buf[..len], b"alpha");

        let (len, _) = timeout(Duration::from_secs(5), sender_two.recv_from(&mut buf)).await??;
        assert_eq!(&buf[..len], b"beta");

        client_endpoint.close().await;
        client_task.abort();
        let _ = client_task.await;
        server_task.abort();
        let _ = server_task.await;
        echo_task.abort();
        let _ = echo_task.await;

        Ok(())
    }
}
