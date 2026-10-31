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
    Register {
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
    Pickup {
        entity_id: u64,
    },
    Consume {
        item_id: u64,
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

/// Максимальный размер кадра.
///
/// Снапшот текущей карты — около 1 КиБ на игрока плюс семь сущностей, так что
/// 64 КиБ с большим запасом. Раньше длина из сети шла прямо в `vec![0u8; len]`,
/// и один пакет мог попросить до 4 ГиБ. Если число сущностей вырастет, поднимать
/// здесь — константа одна на всех.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

#[derive(Debug)]
pub enum FrameError {
    Io(std::io::Error),
    TooLarge(usize),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Io(e) => write!(f, "ошибка чтения кадра: {e}"),
            FrameError::TooLarge(n) => {
                write!(f, "кадр {n} байт превышает лимит {MAX_FRAME_BYTES}")
            }
        }
    }
}

impl std::error::Error for FrameError {}

/// Читает кадр с 4-байтовым префиксом длины, отказываясь выделять память
/// под размер, который клиент не имеет права присылать.
pub async fn read_frame<R>(reader: &mut R) -> Result<Vec<u8>, FrameError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(FrameError::Io)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(FrameError::TooLarge(len));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await.map_err(FrameError::Io)?;
    Ok(buf)
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
    let buf = read_frame(stream).await?;
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

    #[tokio::test]
    async fn frame_size_guard_rejects_oversized_length() {
        let mut data = Vec::new();
        data.extend_from_slice(&((MAX_FRAME_BYTES as u32) + 1).to_le_bytes());
        data.extend_from_slice(&[0u8; 16]);
        let mut cursor: &[u8] = &data;
        match read_frame(&mut cursor).await {
            Err(FrameError::TooLarge(n)) => assert_eq!(n, MAX_FRAME_BYTES + 1),
            other => panic!("ожидали TooLarge, получили {other:?}"),
        }
    }

    #[tokio::test]
    async fn frame_reads_valid_payload() {
        let pkt = ClientPacket::Consume { item_id: 7 };
        let data = encode(&pkt).unwrap();
        let mut cursor: &[u8] = &data;
        let buf = read_frame(&mut cursor).await.unwrap();
        match bincode::deserialize::<ClientPacket>(&buf).unwrap() {
            ClientPacket::Consume { item_id } => assert_eq!(item_id, 7),
            _ => panic!("wrong variant"),
        }
    }
}
