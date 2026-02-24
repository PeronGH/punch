use crate::parse::Mapping;
use crate::proxy;
use anyhow::{bail, Result};
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointId, SecretKey};
use tokio::net::{TcpListener, TcpStream};

const ALPN: &[u8] = b"punch/0";

pub async fn run(
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

async fn run_listener(conn: Connection, mapping: Mapping) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", mapping.local)).await?;
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
