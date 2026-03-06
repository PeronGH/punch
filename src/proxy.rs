use anyhow::Result;
use iroh::endpoint::{RecvStream, SendStream};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

pub async fn bidirectional(
    mut send: SendStream,
    mut recv: RecvStream,
    tcp: TcpStream,
) -> Result<()> {
    let (mut tcp_read, mut tcp_write) = tcp.into_split();
    bridge(&mut send, &mut recv, &mut tcp_read, &mut tcp_write).await
}

pub async fn bridge<R, W>(
    send: &mut SendStream,
    recv: &mut RecvStream,
    local_read: &mut R,
    local_write: &mut W,
) -> Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let quic_to_local = async {
        let mut buf = [0u8; 8192];
        loop {
            match recv.read(&mut buf).await? {
                Some(n) => local_write.write_all(&buf[..n]).await?,
                None => {
                    local_write.shutdown().await?;
                    break;
                }
            }
        }
        anyhow::Ok(())
    };

    let local_to_quic = async {
        let mut buf = [0u8; 8192];
        loop {
            let n = local_read.read(&mut buf).await?;
            if n == 0 {
                send.finish()?;
                break;
            }
            send.write_all(&buf[..n]).await?;
        }
        anyhow::Ok(())
    };

    let result = tokio::try_join!(quic_to_local, local_to_quic);
    match result {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}
