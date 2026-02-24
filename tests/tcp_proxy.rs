use anyhow::Result;
use iroh::endpoint::{RecvStream, SendStream};
use iroh::{Endpoint, SecretKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const ALPN: &[u8] = b"punch/0";

/// End-to-end test: set up two iroh endpoints and a local TCP echo server,
/// proxy a bidi stream through them, and verify data + EOF propagation.
#[tokio::test]
async fn tcp_proxy_roundtrip() -> Result<()> {
    // Start a local TCP echo server.
    let echo_listener = TcpListener::bind("127.0.0.1:0").await?;
    let echo_port = echo_listener.local_addr()?.port();

    tokio::spawn(async move {
        loop {
            let (mut sock, _) = echo_listener.accept().await.unwrap();
            tokio::spawn(async move {
                let (mut r, mut w) = sock.split();
                tokio::io::copy(&mut r, &mut w).await.ok();
            });
        }
    });

    // Create server endpoint (accepts connections).
    let server_key = SecretKey::generate(&mut rand::rng());
    let server_endpoint = Endpoint::builder()
        .secret_key(server_key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;

    let server_addr = server_endpoint.addr();

    // Spawn server handler (endpoint stays alive in outer scope).
    let allowed_port = echo_port;
    let server_task = tokio::spawn(async move {
        let incoming = server_endpoint.accept().await.unwrap();
        let conn = incoming.await.unwrap();
        let (send, mut recv) = conn.accept_bi().await.unwrap();

        let mut port_buf = [0u8; 2];
        recv.read_exact(&mut port_buf).await.unwrap();
        let port = u16::from_be_bytes(port_buf);
        assert_eq!(port, allowed_port);

        let tcp = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        proxy_bidi(send, recv, tcp).await.unwrap();

        // Keep connection alive until client is done.
        conn.closed().await;
    });

    // Create client endpoint.
    let client_key = SecretKey::generate(&mut rand::rng());
    let client_endpoint = Endpoint::builder()
        .secret_key(client_key)
        .bind()
        .await?;

    let conn = client_endpoint.connect(server_addr, ALPN).await?;

    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(&echo_port.to_be_bytes()).await?;

    let payload = b"hello punch";
    send.write_all(payload).await?;
    send.finish()?;

    let response = recv.read_to_end(4096).await?;
    assert_eq!(response, payload);

    conn.close(0u32.into(), b"done");
    client_endpoint.close().await;
    server_task.await?;

    Ok(())
}

/// Test that connecting to a port not in the expose list resets the stream.
#[tokio::test]
async fn tcp_proxy_refused_port() -> Result<()> {
    let server_key = SecretKey::generate(&mut rand::rng());
    let server_endpoint = Endpoint::builder()
        .secret_key(server_key)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await?;

    let server_addr = server_endpoint.addr();

    let server_task = tokio::spawn(async move {
        let incoming = server_endpoint.accept().await.unwrap();
        let conn = incoming.await.unwrap();
        let (_send, mut recv) = conn.accept_bi().await.unwrap();

        let mut port_buf = [0u8; 2];
        recv.read_exact(&mut port_buf).await.unwrap();
        // Port not in expose list — drop streams to reset.
    });

    let client_key = SecretKey::generate(&mut rand::rng());
    let client_endpoint = Endpoint::builder()
        .secret_key(client_key)
        .bind()
        .await?;

    let conn = client_endpoint.connect(server_addr, ALPN).await?;
    let (mut send, mut recv) = conn.open_bi().await?;

    send.write_all(&9999u16.to_be_bytes()).await?;

    let result = recv.read_to_end(4096).await;
    assert!(result.is_err());

    server_task.await?;
    conn.close(0u32.into(), b"done");
    client_endpoint.close().await;

    Ok(())
}

async fn proxy_bidi(
    mut send: SendStream,
    mut recv: RecvStream,
    tcp: tokio::net::TcpStream,
) -> Result<()> {
    let (mut tcp_read, mut tcp_write) = tcp.into_split();

    let quic_to_tcp = async {
        let mut buf = [0u8; 8192];
        loop {
            match recv.read(&mut buf).await? {
                Some(n) => tcp_write.write_all(&buf[..n]).await?,
                None => {
                    tcp_write.shutdown().await?;
                    break;
                }
            }
        }
        anyhow::Ok(())
    };

    let tcp_to_quic = async {
        let mut buf = [0u8; 8192];
        loop {
            let n = tcp_read.read(&mut buf).await?;
            if n == 0 {
                send.finish()?;
                break;
            }
            send.write_all(&buf[..n]).await?;
        }
        anyhow::Ok(())
    };

    tokio::try_join!(quic_to_tcp, tcp_to_quic)?;
    Ok(())
}
