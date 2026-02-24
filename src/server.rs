use crate::parse::Port;
use crate::proxy;
use anyhow::{bail, Result};
use iroh::endpoint::{Incoming, RecvStream, SendStream};
use iroh::{Endpoint, SecretKey};
use std::collections::HashSet;
use tokio::net::TcpStream;

const ALPN: &[u8] = b"punch/0";

pub async fn run(ports: Vec<Port>, secret_key: SecretKey) -> Result<()> {
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

async fn handle_connection(incoming: Incoming, allowed: &HashSet<u16>) -> Result<()> {
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
    send: SendStream,
    mut recv: RecvStream,
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
