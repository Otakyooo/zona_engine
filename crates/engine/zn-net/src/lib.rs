//! zn-net: netcode (snapshot/delta, prediction, lag-compensation).

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::net::SocketAddr;
use zn_sim::{InputCommand, Inventory, PlayerProfile, Snapshot};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClientPacket {
    Hello {
        name: String,
    },
    Login {
        username: String,
        password: String,
    },
    Connect {
        token: String,
    },
    Input(InputCommand),
    Shot {
        seq: u32,
        origin: [f32; 3],
        dir: [f32; 3],
        client_time: u64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ServerPacket {
    Welcome {
        id: u64,
    },
    LoginResult {
        ok: bool,
        token: Option<String>,
        error: Option<String>,
    },
    Profile {
        profile: PlayerProfile,
        inventory: Inventory,
    },
    Snapshot(Snapshot),
    ShotResult {
        hit: Option<u64>,
    },
}

pub fn encode<T: Serialize>(packet: &T) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let data = bincode::serialize(packet)?;
    let len = data.len() as u32;
    let mut out = Vec::with_capacity(4 + data.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&data);
    Ok(out)
}

pub fn decode_client(bytes: &[u8]) -> Result<ClientPacket, Box<dyn std::error::Error>> {
    Ok(bincode::deserialize(bytes)?)
}

pub struct UdpEndpoint {
    pub socket: tokio::net::UdpSocket,
}

impl UdpEndpoint {
    pub async fn bind(addr: &str) -> anyhow::Result<Self> {
        let socket = tokio::net::UdpSocket::bind(addr)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(Self { socket })
    }
    pub async fn send_to(&self, data: &[u8], to: SocketAddr) -> anyhow::Result<()> {
        self.socket
            .send_to(data, to)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        Ok(())
    }
    pub async fn recv_from(&self) -> anyhow::Result<(Vec<u8>, SocketAddr)> {
        let mut buf = vec![0u8; 65535];
        let (n, addr) = self
            .socket
            .recv_from(&mut buf)
            .await
            .map_err(|e| anyhow::anyhow!(e))?;
        buf.truncate(n);
        Ok((buf, addr))
    }
}

#[derive(Clone, Debug)]
pub struct ReliableChannel {
    pub next_seq: u32,
    pub acked: std::collections::HashSet<u32>,
    pub pending: VecDeque<(u32, Vec<u8>, std::time::Instant)>,
    pub timeout: std::time::Duration,
}

impl Default for ReliableChannel {
    fn default() -> Self {
        Self {
            next_seq: 0,
            acked: std::collections::HashSet::new(),
            pending: VecDeque::new(),
            timeout: std::time::Duration::from_millis(100),
        }
    }
}

impl ReliableChannel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn send_reliable(&mut self, data: Vec<u8>) -> u32 {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.pending
            .push_back((seq, data, std::time::Instant::now()));
        seq
    }
    pub fn ack(&mut self, seq: u32) {
        self.acked.insert(seq);
        self.pending.retain(|(s, _, _)| *s != seq);
    }
    pub fn resend_due(&mut self) -> Vec<(u32, Vec<u8>)> {
        let now = std::time::Instant::now();
        let mut out = Vec::new();
        for (seq, data, sent) in self.pending.iter_mut() {
            if now.duration_since(*sent) >= self.timeout {
                out.push((*seq, data.clone()));
                *sent = now;
            }
        }
        out
    }
    pub fn is_acked(&self, seq: u32) -> bool {
        self.acked.contains(&seq)
    }
}

pub async fn send_packet<W, T>(stream: &mut W, packet: &T) -> Result<(), Box<dyn std::error::Error>>
where
    W: tokio::io::AsyncWrite + Unpin,
    T: Serialize,
{
    use tokio::io::AsyncWriteExt;
    let data = encode(packet)?;
    stream.write_all(&data).await?;
    Ok(())
}

pub async fn recv_packet<T: for<'de> Deserialize<'de>>(
    stream: &mut tokio::net::TcpStream,
) -> Result<T, Box<dyn std::error::Error>> {
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(bincode::deserialize(&buf)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_decode_roundtrip() {
        let pkt = ClientPacket::Hello {
            name: "test".into(),
        };
        let data = encode(&pkt).unwrap();
        // strip 4-byte length prefix
        let client: ClientPacket = bincode::deserialize(&data[4..]).unwrap();
        match client {
            ClientPacket::Hello { name } => assert_eq!(name, "test"),
            _ => panic!("wrong variant"),
        }
    }
}
