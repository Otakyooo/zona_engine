//! zn-net: netcode (snapshot/delta, prediction, lag-compensation).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use zn_sim::{HitZone, InputCommand, Inventory, PlayerProfile, Snapshot};

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
    /// Поля этого варианта меняются только вместе с клиентом и сервером:
    /// bincode раскладывает вариант позиционно, и старая сборка прочитает
    /// новый пакет как мусор.
    ShotResult {
        hit: Option<u64>,
        /// Попадание добило цель — клиент показывает отметку в killfeed.
        killed: bool,
        /// Куда пришлось попадание: клиент отмечает хедшот.
        zone: Option<HitZone>,
        /// Урон, уже посчитанный симуляцией; клиент его только отображает.
        damage: f32,
    },
    /// Новый вариант добавляется только в конец: дискриминанты bincode
    /// позиционные, вставка в середину молча переинтерпретирует пакеты.
    Inventory(Inventory),
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

/// Ограничитель частоты пакетов ввода на игрока.
///
/// Клиент шлёт `Input` каждый кадр, поэтому лимит должен быть заметно выше
/// частоты кадров: 120 Гц резали бы честную игру на 144/165-герцовых
/// мониторах. 240 Гц никому не мешают и при этом срезают флуд, которым можно
/// переписывать скорость игрока тысячи раз в секунду.
pub struct InputRateLimiter {
    min_interval: Duration,
    last: HashMap<u64, Instant>,
}

impl InputRateLimiter {
    pub const DEFAULT_RATE: u32 = 240;

    pub fn new(max_per_sec: u32) -> Self {
        let max = max_per_sec.max(1);
        Self {
            min_interval: Duration::from_secs_f64(1.0 / max as f64),
            last: HashMap::new(),
        }
    }

    /// Пропускает пакет, если с предыдущего принятого прошёл интервал.
    ///
    /// Метка времени обновляется только на принятом пакете: иначе серия
    /// быстрых пакетов бесконечно сдвигала бы окно и флуд всё равно проходил.
    /// `now` — параметр, а не `Instant::now()`: тесты сдвигают время руками.
    pub fn allow(&mut self, player_id: u64, now: Instant) -> bool {
        match self.last.get(&player_id) {
            Some(prev) if now.saturating_duration_since(*prev) < self.min_interval => false,
            _ => {
                self.last.insert(player_id, now);
                true
            }
        }
    }

    /// Забывает игрока при отключении, чтобы карта не росла вечно.
    pub fn forget(&mut self, player_id: u64) {
        self.last.remove(&player_id);
    }
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

    #[test]
    fn inventory_packet_roundtrip() {
        let mut inv = Inventory::default();
        inv.add_item(1, 3);
        let data = encode(&ServerPacket::Inventory(inv)).unwrap();
        match bincode::deserialize::<ServerPacket>(&data[4..]).unwrap() {
            ServerPacket::Inventory(got) => {
                assert_eq!(got.items.len(), 1);
                assert_eq!(got.items[0].quantity, 3);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn udp_payload_is_raw_bincode_without_prefix() {
        let pkt = ClientPacket::Input(InputCommand {
            seq: 1,
            yaw: 0.0,
            pitch: 0.0,
            move_x: 1.0,
            move_z: 0.0,
            dt: 0.016,
        });
        let udp = bincode::serialize(&pkt).unwrap();
        let tcp = encode(&pkt).unwrap();

        // Регрессия на баг, из-за которого UDP молча не работал: префикс длины
        // принадлежит TCP-фреймингу и в датаграмме его быть не должно.
        assert_eq!(
            tcp.len(),
            udp.len() + 4,
            "TCP-кадр = префикс + сырой bincode"
        );
        assert_eq!(&tcp[4..], &udp[..], "префикс — единственное отличие");
        assert!(
            matches!(decode_client(&udp).unwrap(), ClientPacket::Input(_)),
            "датаграмма разбирается без префикса"
        );
    }

    #[tokio::test]
    async fn frame_accepts_payload_at_exact_limit() {
        let mut data = Vec::with_capacity(4 + MAX_FRAME_BYTES);
        data.extend_from_slice(&(MAX_FRAME_BYTES as u32).to_le_bytes());
        data.extend(std::iter::repeat_n(0u8, MAX_FRAME_BYTES));
        let mut cursor: &[u8] = &data;

        let buf = read_frame(&mut cursor)
            .await
            .expect("кадр ровно по лимиту должен читаться");
        assert_eq!(buf.len(), MAX_FRAME_BYTES);
    }

    #[test]
    fn server_packet_inventory_keeps_variant_index() {
        let data = bincode::serialize(&ServerPacket::Inventory(Inventory::default())).unwrap();
        let idx = u32::from_le_bytes(data[..4].try_into().unwrap());
        // bincode кодирует дискриминант позиционно: вставка варианта в середину
        // enum молча переинтерпретирует старые пакеты, поэтому только в конец.
        assert_eq!(
            idx, 5,
            "новые варианты ServerPacket добавляются только в конец"
        );
    }

    #[test]
    fn limiter_allows_first_packet() {
        let mut lim = InputRateLimiter::new(240);
        assert!(lim.allow(1, Instant::now()));
    }

    #[test]
    fn limiter_drops_packet_inside_interval() {
        let mut lim = InputRateLimiter::new(240);
        let t0 = Instant::now();
        assert!(lim.allow(1, t0));
        assert!(
            !lim.allow(1, t0 + Duration::from_millis(1)),
            "пакет внутри окна отбрасывается"
        );
    }

    #[test]
    fn limiter_allows_packet_after_interval() {
        let mut lim = InputRateLimiter::new(100);
        let t0 = Instant::now();
        assert!(lim.allow(1, t0));
        assert!(
            lim.allow(1, t0 + Duration::from_millis(11)),
            "после истечения интервала пакет проходит"
        );
    }

    #[test]
    fn limiter_forget_resets_player() {
        let mut lim = InputRateLimiter::new(1);
        let t0 = Instant::now();
        assert!(lim.allow(1, t0));
        assert!(!lim.allow(1, t0), "второй пакет в том же окне отброшен");
        lim.forget(1);
        assert!(lim.allow(1, t0), "после forget состояние игрока сброшено");
    }
}
