mod key;
mod proxy;

use anyhow::{bail, Context, Result};
use clap::Parser;
use iroh::{Endpoint, EndpointId, SecretKey};
use std::collections::HashSet;
use std::str::FromStr;
use tokio::net::{TcpListener, TcpStream};

const ALPN: &[u8] = b"punch/0";

#[derive(Parser)]
#[command(about = "Peer-to-peer TCP and UDP port forwarding over iroh")]
enum Cli {
    /// Expose local ports to remote peers
    Out {
        /// Ports to expose (e.g. 8080 22)
        #[arg(required = true)]
        ports: Vec<String>,
    },
    /// Connect to a remote peer
    In {
        /// Remote peer's endpoint ID (base32)
        pubkey: String,
        /// Mappings (e.g. 4000:8080)
        #[arg(required = true)]
        mappings: Vec<String>,
    },
}

/// A validated port number (1–65535).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Port(u16);

impl Port {
    pub fn get(self) -> u16 {
        self.0
    }
}

impl FromStr for Port {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let n: u16 = s.parse().context("invalid port number")?;
        if n == 0 {
            bail!("port must be 1–65535, got 0");
        }
        Ok(Port(n))
    }
}

/// A local:remote port mapping for `punch in`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mapping {
    pub local: u16,
    pub remote: u16,
}

impl FromStr for Mapping {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (l, r) = s.split_once(':').context("mapping must be <local>:<remote>")?;
        let local: Port = l.parse().context("invalid local port")?;
        let remote: Port = r.parse().context("invalid remote port")?;
        Ok(Mapping {
            local: local.get(),
            remote: remote.get(),
        })
    }
}

fn parse_ports(args: &[String]) -> Result<Vec<Port>> {
    let mut ports = Vec::with_capacity(args.len());
    for arg in args {
        let port: Port = arg.parse()?;
        if ports.contains(&port) {
            bail!("duplicate port: {}", port.get());
        }
        ports.push(port);
    }
    Ok(ports)
}

fn parse_mappings(args: &[String]) -> Result<Vec<Mapping>> {
    let mut mappings = Vec::with_capacity(args.len());
    for arg in args {
        let mapping: Mapping = arg.parse()?;
        mappings.push(mapping);
    }
    Ok(mappings)
}

async fn run_server(ports: Vec<Port>, secret_key: SecretKey) -> Result<()> {
    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;

    eprintln!("public key: {}", endpoint.id());

    let allowed: HashSet<u16> = ports.iter().map(|p| p.get()).collect();

    while let Some(incoming) = endpoint.accept().await {
        let allowed = allowed.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(incoming, &allowed).await {
                eprintln!("connection error: {e}");
            }
        });
    }

    Ok(())
}

async fn handle_connection(
    incoming: iroh::endpoint::Incoming,
    allowed: &HashSet<u16>,
) -> Result<()> {
    let conn = incoming.await?;
    loop {
        let (send, recv) = conn.accept_bi().await?;
        let allowed = allowed.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_stream(send, recv, &allowed).await {
                eprintln!("stream error: {e}");
            }
        });
    }
}

async fn handle_stream(
    send: iroh::endpoint::SendStream,
    mut recv: iroh::endpoint::RecvStream,
    allowed: &HashSet<u16>,
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

async fn run_client(
    endpoint_id: EndpointId,
    mappings: Vec<Mapping>,
    secret_key: SecretKey,
) -> Result<()> {
    let endpoint = Endpoint::builder()
        .secret_key(secret_key)
        .bind()
        .await?;

    let conn = endpoint.connect(endpoint_id, ALPN).await?;

    let mut tasks = Vec::new();
    for mapping in mappings {
        let conn = conn.clone();
        tasks.push(tokio::spawn(async move {
            run_listener(conn, mapping).await
        }));
    }

    // Exit on connection loss or listener failure.
    tokio::select! {
        _ = conn.closed() => bail!("connection to remote peer lost"),
        result = async {
            for task in tasks {
                task.await??;
            }
            Ok::<_, anyhow::Error>(())
        } => result,
    }
}

async fn run_listener(conn: iroh::endpoint::Connection, mapping: Mapping) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", mapping.local)).await?;
    loop {
        let (tcp, _) = listener.accept().await?;
        let conn = conn.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client_stream(conn, mapping.remote, tcp).await {
                eprintln!("stream error: {e}");
            }
        });
    }
}

async fn handle_client_stream(
    conn: iroh::endpoint::Connection,
    remote_port: u16,
    tcp: TcpStream,
) -> Result<()> {
    let (mut send, recv) = conn.open_bi().await?;
    send.write_all(&remote_port.to_be_bytes()).await?;
    proxy::bidirectional(send, recv, tcp).await
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse() {
        Cli::Out { ports } => {
            let ports = parse_ports(&ports)?;
            let secret_key = key::load_or_generate()?;
            run_server(ports, secret_key).await
        }
        Cli::In { pubkey, mappings } => {
            let endpoint_id: EndpointId = pubkey.parse().context("invalid endpoint ID")?;
            let mappings = parse_mappings(&mappings)?;
            let secret_key = key::load_or_generate()?;
            run_client(endpoint_id, mappings, secret_key).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_valid() {
        assert_eq!("8080".parse::<Port>().unwrap().get(), 8080);
        assert_eq!("22".parse::<Port>().unwrap().get(), 22);
        assert_eq!("1".parse::<Port>().unwrap().get(), 1);
        assert_eq!("65535".parse::<Port>().unwrap().get(), 65535);
    }

    #[test]
    fn port_invalid() {
        assert!("0".parse::<Port>().is_err());
        assert!("70000".parse::<Port>().is_err());
        assert!("abc".parse::<Port>().is_err());
        assert!("".parse::<Port>().is_err());
    }

    #[test]
    fn port_duplicate_detection() {
        let args: Vec<String> = vec!["80".into(), "443".into(), "80".into()];
        assert!(parse_ports(&args).is_err());
    }

    #[test]
    fn mapping_valid() {
        let m: Mapping = "4000:8080".parse().unwrap();
        assert_eq!(m.local, 4000);
        assert_eq!(m.remote, 8080);
    }

    #[test]
    fn mapping_invalid() {
        assert!("0:80".parse::<Mapping>().is_err());
        assert!("80".parse::<Mapping>().is_err());
        assert!("abc:80".parse::<Mapping>().is_err());
        assert!("80:0".parse::<Mapping>().is_err());
    }
}
