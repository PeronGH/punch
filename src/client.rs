use crate::parse::Mapping;
use crate::proxy;
use anyhow::{bail, Result};
use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointId, SecretKey};
use std::future::Future;
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;

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

    let mut tasks = JoinSet::new();
    for mapping in mappings {
        let conn = conn.clone();
        tasks.spawn(async move { run_listener(conn, mapping).await });
    }

    supervise_listeners(conn.closed(), tasks).await
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

async fn supervise_listeners(
    conn_closed: impl Future,
    mut tasks: JoinSet<Result<()>>,
) -> Result<()> {
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
    use super::supervise_listeners;
    use anyhow::Result;
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio::task::JoinSet;
    use tokio::time::{Duration, timeout};

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
            supervise_listeners(std::future::pending::<()>(), tasks),
        )
        .await;

        assert!(result.is_ok(), "listener supervision hung");
        assert!(result.unwrap().is_err(), "bind failure should surface");
        Ok(())
    }
}
