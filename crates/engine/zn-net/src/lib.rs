//! zn-net: netcode (snapshot/delta, prediction, lag-compensation).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::time::{Duration, Instant};
use zn_sim::{
    Aabb, Anomaly, Artifact, Bounds, Creature, HitZone, InputCommand, Inventory, InventoryItem,
    LootItem, PlayerProfile, PlayerState, Snapshot,
};

/// Версия игрового протокола.
///
/// Bincode не несёт имён полей: клиент другой сборки либо не разберёт пакет,
/// либо, что хуже, разберёт его как мусор. Версия едет в `Welcome`, и клиент
/// обязан сказать об этом вслух, а не искать баг в движении.
pub const PROTOCOL_VERSION: u16 = 7;

/// Дельта снапшота: только изменившиеся сущности и списки удалённых.
///
/// Клиент применяет её к своему последнему кадру; `base_tick` обязан совпасть.
/// Сервер хранит историю кадров, поэтому потеря дельты не ломает состояние:
/// следующая считается от того же подтверждённого тика и содержит накопления.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SnapshotDelta {
    pub base_tick: u64,
    pub tick: u64,
    pub players: Vec<PlayerState>,
    pub removed_players: Vec<u64>,
    pub anomalies: Vec<Anomaly>,
    pub removed_anomalies: Vec<u64>,
    pub artifacts: Vec<Artifact>,
    pub removed_artifacts: Vec<u64>,
    pub loot: Vec<LootItem>,
    pub removed_loot: Vec<u64>,
    pub creatures: Vec<Creature>,
    pub removed_creatures: Vec<u64>,
}

fn collect_updates<T: Clone + PartialEq>(
    base: &[T],
    current: &[T],
    id: impl Fn(&T) -> u64,
) -> (Vec<T>, Vec<u64>) {
    let mut updates = Vec::new();
    for item in current {
        match base.iter().find(|old| id(old) == id(item)) {
            Some(old) if old == item => {}
            _ => updates.push(item.clone()),
        }
    }
    let mut removed: Vec<u64> = Vec::new();
    for old in base {
        if !current.iter().any(|item| id(item) == id(old)) {
            removed.push(id(old));
        }
    }
    removed.sort_unstable();
    (updates, removed)
}

fn apply_updates<T: Clone>(
    target: &mut Vec<T>,
    updates: &[T],
    removed: &[u64],
    id: impl Fn(&T) -> u64,
) {
    target.retain(|item| !removed.contains(&id(item)));
    for update in updates {
        if let Some(pos) = target.iter().position(|item| id(item) == id(update)) {
            target[pos] = update.clone();
        } else {
            target.push(update.clone());
        }
    }
}

impl SnapshotDelta {
    /// Сравнивает два полных кадра и оставляет только различия.
    pub fn between(base: &Snapshot, current: &Snapshot) -> Self {
        let (players, removed_players) = collect_updates(&base.players, &current.players, |p| p.id);
        let (anomalies, removed_anomalies) =
            collect_updates(&base.anomalies, &current.anomalies, |a| a.id);
        let (artifacts, removed_artifacts) =
            collect_updates(&base.artifacts, &current.artifacts, |a| a.id);
        let (loot, removed_loot) = collect_updates(&base.loot, &current.loot, |l| l.id);
        let (creatures, removed_creatures) =
            collect_updates(&base.creatures, &current.creatures, |c| c.id);
        Self {
            base_tick: base.tick,
            tick: current.tick,
            players,
            removed_players,
            anomalies,
            removed_anomalies,
            artifacts,
            removed_artifacts,
            loot,
            removed_loot,
            creatures,
            removed_creatures,
        }
    }

    /// Применяет дельту к последнему полному кадру клиента.
    pub fn apply(&self, snapshot: &mut Snapshot) {
        apply_updates(
            &mut snapshot.players,
            &self.players,
            &self.removed_players,
            |p| p.id,
        );
        apply_updates(
            &mut snapshot.anomalies,
            &self.anomalies,
            &self.removed_anomalies,
            |a| a.id,
        );
        apply_updates(
            &mut snapshot.artifacts,
            &self.artifacts,
            &self.removed_artifacts,
            |a| a.id,
        );
        apply_updates(&mut snapshot.loot, &self.loot, &self.removed_loot, |l| l.id);
        apply_updates(
            &mut snapshot.creatures,
            &self.creatures,
            &self.removed_creatures,
            |c| c.id,
        );
        snapshot.tick = self.tick;
    }

    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
            && self.removed_players.is_empty()
            && self.anomalies.is_empty()
            && self.removed_anomalies.is_empty()
            && self.artifacts.is_empty()
            && self.removed_artifacts.is_empty()
            && self.loot.is_empty()
            && self.removed_loot.is_empty()
            && self.creatures.is_empty()
            && self.removed_creatures.is_empty()
    }
}

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
    /// Проверка живости канала и замер RTT. UDP-сокет не даёт события
    /// «соединение закрыто», поэтому живность подтверждается явно.
    Ping {
        nonce: u64,
    },
    /// Выбор оружия из каталога. Сервер проверяет id и присылает `Loadout`.
    SetLoadout {
        weapon_id: u64,
    },
    /// Забрать предмет из схрона в рейдовый инвентарь.
    TakeFromStash {
        item_id: u64,
        quantity: u32,
    },
    /// Начать новый рейд после эвакуации или смерти.
    StartRaid,
    /// Клиент подтверждает, какой кадр у него сейчас. Сервер считает
    /// следующую дельту от него, поэтому потеря пакета не рвёт состояние.
    SnapshotAck {
        tick: u64,
    },
    /// Ответ на серверный замер RTT.
    Pong {
        nonce: u64,
    },
}

/// Почему закончился рейд.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RaidEndReason {
    Extracted,
    Died,
}

/// Итог рейда: причина, полученный опыт, вынесенное или потерянное и время.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RaidResult {
    pub reason: RaidEndReason,
    pub xp_gained: i64,
    pub items: Vec<InventoryItem>,
    pub duration_secs: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ServerPacket {
    Welcome {
        id: u64,
        /// Версия протокола сервера; несовпадение — повод не начинать игру.
        protocol: u16,
        /// Границы и стены карты: клиент предсказывает движение той же
        /// функцией, что и сервер, и обязан видеть те же препятствия.
        bounds: Bounds,
        obstacles: Vec<Aabb>,
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
    Pong {
        nonce: u64,
    },
    /// Подтверждение выбранного оружия и остатка патронов.
    Loadout {
        weapon_id: u64,
        ammo: u32,
    },
    /// Дельта схрона: столько добавилось после эвакуации. Клиент прибавляет
    /// к своему списку и показывает тост.
    Stash {
        items: Vec<InventoryItem>,
    },
    /// Полное содержимое схрона (подключение и изъятие).
    StashContents {
        items: Vec<InventoryItem>,
    },
    /// Сервер отказал в выборе оружия: его нет в инвентаре.
    LoadoutDenied {
        weapon_id: u64,
        reason: String,
    },
    /// Рейд закончился: клиент показывает экран результата, а не меню.
    RaidResult {
        result: RaidResult,
    },
    /// Изменения относительно подтверждённого кадра.
    SnapshotDelta(SnapshotDelta),
    /// Серверный замер RTT: клиент обязан немедленно ответить `Pong`.
    Ping {
        nonce: u64,
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

    #[test]
    fn welcome_carries_protocol_and_terrain() {
        let pkt = ServerPacket::Welcome {
            id: 7,
            protocol: PROTOCOL_VERSION,
            bounds: Bounds::default(),
            obstacles: vec![Aabb::new(Bounds::default().min, Bounds::default().max)],
        };
        let data = bincode::serialize(&pkt).unwrap();
        match bincode::deserialize::<ServerPacket>(&data).unwrap() {
            ServerPacket::Welcome {
                id,
                protocol,
                obstacles,
                ..
            } => {
                assert_eq!(id, 7);
                assert_eq!(protocol, PROTOCOL_VERSION);
                assert_eq!(obstacles.len(), 1, "стены обязаны доехать до клиента");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn ping_pong_and_loadout_roundtrip() {
        let ping = bincode::serialize(&ClientPacket::Ping { nonce: 42 }).unwrap();
        match decode_client(&ping).unwrap() {
            ClientPacket::Ping { nonce } => assert_eq!(nonce, 42),
            _ => panic!("wrong variant"),
        }

        let loadout = bincode::serialize(&ClientPacket::SetLoadout { weapon_id: 12 }).unwrap();
        match decode_client(&loadout).unwrap() {
            ClientPacket::SetLoadout { weapon_id } => assert_eq!(weapon_id, 12),
            _ => panic!("wrong variant"),
        }

        let pong = bincode::serialize(&ServerPacket::Pong { nonce: 42 }).unwrap();
        match bincode::deserialize::<ServerPacket>(&pong).unwrap() {
            ServerPacket::Pong { nonce } => assert_eq!(nonce, 42),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn stash_packets_roundtrip() {
        let take = bincode::serialize(&ClientPacket::TakeFromStash {
            item_id: 11,
            quantity: 1,
        })
        .unwrap();
        match decode_client(&take).unwrap() {
            ClientPacket::TakeFromStash { item_id, quantity } => {
                assert_eq!((item_id, quantity), (11, 1));
            }
            _ => panic!("wrong variant"),
        }

        let contents = bincode::serialize(&ServerPacket::StashContents {
            items: vec![InventoryItem {
                item_id: 11,
                quantity: 1,
                slot: 0,
            }],
        })
        .unwrap();
        match bincode::deserialize::<ServerPacket>(&contents).unwrap() {
            ServerPacket::StashContents { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].item_id, 11);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn raid_packets_roundtrip() {
        let start = bincode::serialize(&ClientPacket::StartRaid).unwrap();
        assert!(matches!(
            decode_client(&start).unwrap(),
            ClientPacket::StartRaid
        ));

        let result = RaidResult {
            reason: RaidEndReason::Extracted,
            xp_gained: 100,
            items: vec![InventoryItem {
                item_id: 1,
                quantity: 2,
                slot: 0,
            }],
            duration_secs: 42,
        };
        let bytes = bincode::serialize(&ServerPacket::RaidResult {
            result: result.clone(),
        })
        .unwrap();
        match bincode::deserialize::<ServerPacket>(&bytes).unwrap() {
            ServerPacket::RaidResult { result: decoded } => assert_eq!(decoded, result),
            _ => panic!("wrong variant"),
        }
    }
}

#[cfg(test)]
mod delta_tests {
    use super::*;

    fn snapshot(tick: u64) -> Snapshot {
        Snapshot {
            tick,
            players: vec![PlayerState {
                id: 1,
                name: "p".into(),
                position: Default::default(),
                health: 100.0,
                radiation: 0.0,
                hunger: 100.0,
                thirst: 100.0,
                yaw: 0.0,
                pitch: 0.0,
                level: 1,
                xp: 0,
                velocity: Default::default(),
                weapon_id: 10,
                ammo: 90,
                last_input_seq: 0,
            }],
            anomalies: Vec::new(),
            artifacts: Vec::new(),
            loot: Vec::new(),
            creatures: Vec::new(),
            extraction_zones: Vec::new(),
        }
    }

    #[test]
    fn identical_frames_produce_empty_delta() {
        let base = snapshot(1);
        let delta = SnapshotDelta::between(&base, &base);
        assert!(delta.is_empty(), "без изменений дельта пуста");
    }

    #[test]
    fn delta_applies_back_to_full_frame_and_is_smaller() {
        let mut base = snapshot(1);
        base.loot = (0..40)
            .map(|i| LootItem {
                id: i,
                position: Default::default(),
                item: zn_sim::Item::from_catalog(1),
            })
            .collect();
        let mut current = base.clone();
        current.tick = 2;
        current.players[0].health = 90.0;
        current.loot.remove(0);

        let delta = SnapshotDelta::between(&base, &current);
        assert_eq!(delta.base_tick, 1);
        assert_eq!(delta.tick, 2);
        assert_eq!(delta.players.len(), 1, "изменился только игрок");
        assert_eq!(delta.removed_loot, vec![0]);

        let mut applied = base.clone();
        delta.apply(&mut applied);
        assert_eq!(applied, current, "дельта восстанавливает полный кадр");

        let delta_bytes = bincode::serialize(&delta).unwrap().len();
        let full_bytes = bincode::serialize(&current).unwrap().len();
        println!("delta={delta_bytes}B full={full_bytes}B");
        assert!(
            delta_bytes * 4 < full_bytes,
            "дельта {delta_bytes}B должна быть кратно меньше полного кадра {full_bytes}B"
        );
    }
}
