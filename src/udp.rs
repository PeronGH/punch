use anyhow::{Context, Result, bail};
use iroh::endpoint::{Connection, SendDatagramError};
use std::time::{Duration, Instant};

pub const FLOW_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);
pub const FLOW_SWEEP_INTERVAL: Duration = Duration::from_secs(30);
pub const MAX_UDP_PACKET_SIZE: usize = 65_535;

const CLIENT_HEADER_LEN: usize = 4;
const SERVER_HEADER_LEN: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientDatagram<'a> {
    pub flow_id: u16,
    pub dest_port: u16,
    pub payload: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerDatagram<'a> {
    pub flow_id: u16,
    pub payload: &'a [u8],
}

pub fn encode_client_datagram(flow_id: u16, dest_port: u16, payload: &[u8]) -> Vec<u8> {
    let mut datagram = Vec::with_capacity(CLIENT_HEADER_LEN + payload.len());
    datagram.extend_from_slice(&flow_id.to_be_bytes());
    datagram.extend_from_slice(&dest_port.to_be_bytes());
    datagram.extend_from_slice(payload);
    datagram
}

pub fn decode_client_datagram(datagram: &[u8]) -> Result<ClientDatagram<'_>> {
    if datagram.len() < CLIENT_HEADER_LEN {
        bail!("client datagram too short");
    }

    Ok(ClientDatagram {
        flow_id: u16::from_be_bytes([datagram[0], datagram[1]]),
        dest_port: u16::from_be_bytes([datagram[2], datagram[3]]),
        payload: &datagram[CLIENT_HEADER_LEN..],
    })
}

pub fn encode_server_datagram(flow_id: u16, payload: &[u8]) -> Vec<u8> {
    let mut datagram = Vec::with_capacity(SERVER_HEADER_LEN + payload.len());
    datagram.extend_from_slice(&flow_id.to_be_bytes());
    datagram.extend_from_slice(payload);
    datagram
}

pub fn decode_server_datagram(datagram: &[u8]) -> Result<ServerDatagram<'_>> {
    if datagram.len() < SERVER_HEADER_LEN {
        bail!("server datagram too short");
    }

    Ok(ServerDatagram {
        flow_id: u16::from_be_bytes([datagram[0], datagram[1]]),
        payload: &datagram[SERVER_HEADER_LEN..],
    })
}

pub fn is_expired(last_activity: Instant, now: Instant) -> bool {
    now.saturating_duration_since(last_activity) >= FLOW_IDLE_TIMEOUT
}

fn ensure_datagram_fits(limit: Option<usize>, datagram_len: usize) -> Result<()> {
    let limit = limit.context("udp datagrams are not available on this connection")?;
    if datagram_len > limit {
        bail!("udp payload exceeds current QUIC datagram size limit");
    }
    Ok(())
}

pub fn send_client_datagram(
    conn: &Connection,
    flow_id: u16,
    dest_port: u16,
    payload: &[u8],
) -> Result<()> {
    let datagram = encode_client_datagram(flow_id, dest_port, payload);
    send_datagram(conn, datagram)
}

pub fn send_server_datagram(conn: &Connection, flow_id: u16, payload: &[u8]) -> Result<()> {
    let datagram = encode_server_datagram(flow_id, payload);
    send_datagram(conn, datagram)
}

fn send_datagram(conn: &Connection, datagram: Vec<u8>) -> Result<()> {
    ensure_datagram_fits(conn.max_datagram_size(), datagram.len())?;
    conn.send_datagram(datagram.into())
        .map_err(map_send_datagram_error)?;
    Ok(())
}

fn map_send_datagram_error(err: SendDatagramError) -> anyhow::Error {
    match err {
        SendDatagramError::UnsupportedByPeer => {
            anyhow::anyhow!("udp datagrams are not supported by the peer")
        }
        SendDatagramError::Disabled => anyhow::anyhow!("udp datagrams are disabled locally"),
        SendDatagramError::TooLarge => {
            anyhow::anyhow!("udp payload exceeds current QUIC datagram size limit")
        }
        SendDatagramError::ConnectionLost(err) => err.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_datagram_roundtrip() {
        let datagram = encode_client_datagram(7, 53, b"hello");
        let decoded = decode_client_datagram(&datagram).unwrap();
        assert_eq!(decoded.flow_id, 7);
        assert_eq!(decoded.dest_port, 53);
        assert_eq!(decoded.payload, b"hello");
    }

    #[test]
    fn server_datagram_roundtrip() {
        let datagram = encode_server_datagram(9, b"world");
        let decoded = decode_server_datagram(&datagram).unwrap();
        assert_eq!(decoded.flow_id, 9);
        assert_eq!(decoded.payload, b"world");
    }

    #[test]
    fn datagram_size_limit_rejects_oversize_payloads() {
        assert!(ensure_datagram_fits(Some(8), 8).is_ok());
        assert!(ensure_datagram_fits(Some(8), 9).is_err());
        assert!(ensure_datagram_fits(None, 1).is_err());
    }

    #[test]
    fn flow_expiration_uses_idle_timeout() {
        let now = Instant::now();
        assert!(!is_expired(
            now,
            now + FLOW_IDLE_TIMEOUT - Duration::from_secs(1)
        ));
        assert!(is_expired(now, now + FLOW_IDLE_TIMEOUT));
    }
}
