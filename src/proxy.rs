use anyhow::Result;
use iroh::endpoint::{RecvStream, SendStream};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

pub async fn bidirectional(
    mut send: SendStream,
    mut recv: RecvStream,
    tcp: TcpStream,
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

    let result = tokio::try_join!(quic_to_tcp, tcp_to_quic);
    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}
