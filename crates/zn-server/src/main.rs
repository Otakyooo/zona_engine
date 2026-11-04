use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use clap::Parser;
use password_hash::{SaltString, rand_core::OsRng};
use sqlx::sqlite::SqlitePool;
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
struct Cli {
    #[arg(long, default_value = "0.0.0.0")]
    bind: String,
    #[arg(long, default_value_t = 7777)]
    port: u16,
    /// Карта зоны: геометрия плюс спавны, лут и аномалии.
    #[arg(long, default_value = "maps/arena.ron")]
    map: String,
}
use glam::Vec3;
use tokio::net::{TcpListener, tcp::OwnedWriteHalf};
use tokio::sync::Mutex;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::interval;
use zn_net::{
    ClientPacket, InputRateLimiter, PROTOCOL_VERSION, RaidEndReason, RaidResult, ServerPacket,
    UdpEndpoint, send_packet,
};
use zn_sim::{Inventory, InventoryItem, Player, PlayerProfile, SimWorld, items};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    zn_core::logging::init("zn-server");
    let cli = Cli::parse();
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:zona.db?mode=rwc".into());
    let pool = SqlitePool::connect(&db_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let listener = TcpListener::bind(format!("{}:{}", cli.bind, cli.port)).await?;
    let udp = UdpEndpoint::bind(&format!("{}:{}", cli.bind, cli.port + 1)).await?;
    let udp = Arc::new(udp);
    log::info!("zn-server listening TCP {}, UDP {}", cli.port, cli.port + 1);

    // Карта — источник истины: существа, лут и аномалии живут в файле, а не
    // в коде сервера. Невалидная карта падает на старте, а не у игрока в игре.
    let map_path = std::path::PathBuf::from(&cli.map);
    let zone = zn_content::ZoneMap::load(&map_path)?;
    log::info!(
        "карта {}: {} узлов, {} существ, {} лута, {} аномалий",
        map_path.display(),
        zone.nodes.len(),
        zone.creature_spawns.len(),
        zone.loot.len(),
        zone.anomalies.len(),
    );
    let mut world = SimWorld::new();
    zone.apply_to(&mut world);
    // Статические боксы карты: один раз для рейкаста и коллизий движения.
    // Клиент получает их в `Welcome`, иначе его предсказание проходит сквозь
    // стены, которые сервер считает твёрдыми.
    let obstacles = zone.obstacles();
    world.obstacles = obstacles.clone();
    let terrain_bounds = world.bounds;
    // Запись в БД живёт вне цикла тика: ожидание диска останавливало мир.
    let persist = spawn_persister(pool.clone());
    let mut writers: HashMap<u64, Arc<Mutex<OwnedWriteHalf>>> = HashMap::new();
    let mut token_to_player: HashMap<String, u64> = HashMap::new();
    let mut player_to_token: HashMap<u64, String> = HashMap::new();
    let mut player_to_user: HashMap<u64, i64> = HashMap::new();
    let mut conn_to_player: HashMap<u64, u64> = HashMap::new();
    let mut conn_addrs: HashMap<u64, SocketAddr> = HashMap::new();
    let mut client_addrs: HashMap<u64, SocketAddr> = HashMap::new();
    let mut addr_to_client: HashMap<SocketAddr, u64> = HashMap::new();
    // Когда от игрока в последний раз был живой пакет. Без этого игрок,
    // у которого умер UDP, остаётся в мире навсегда: TCP-разрыв придёт
    // только если клиент сам закроет соединение.
    let mut last_seen: HashMap<u64, Instant> = HashMap::new();
    // Начало рейда и опыт на входе: без базовой линии «опыт за рейд» — это
    // просто суммарный опыт аккаунта.
    let mut raid_started: HashMap<u64, Instant> = HashMap::new();
    let mut raid_start_xp: HashMap<u64, i64> = HashMap::new();
    let mut tick_stats = TickStats::default();
    let mut snapshot_bytes: usize = 0;
    let mut snapshot_rounds: usize = 0;
    let mut next_id: u64 = 1;
    let mut login_attempts: HashMap<IpAddr, Attempts> = HashMap::new();
    let mut input_limiter = InputRateLimiter::new(InputRateLimiter::DEFAULT_RATE);
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<(u64, ClientPacket)>(1024);

    let mut tick_timer = interval(Duration::from_millis(1000 / 60));
    let mut save_timer = interval(Duration::from_secs(60));
    let mut session_sweep = interval(Duration::from_secs(600));
    let mut idle_sweep = interval(Duration::from_secs(5));
    let mut stats_timer = interval(Duration::from_secs(10));
    let mut server_ping_timer = interval(Duration::from_secs(1));
    let mut snapshot_counter: u32 = 0;
    // Кадры на клиента: ack — что у него уже есть; history — из чего считать
    // дельту. Потеря дельты не рвёт состояние, пока базовый кадр в истории.
    let mut snap_sync: HashMap<u64, SnapSync> = HashMap::new();
    let mut snap_delta_frames: usize = 0;
    let mut snap_key_frames: usize = 0;
    // RTT измеряет сервер: клиентский HUD — не доверенный источник.
    let mut rtt_ms: HashMap<u64, f32> = HashMap::new();
    let mut server_pings: HashMap<u64, (u64, Instant)> = HashMap::new();

    let mut udp_buf = vec![0u8; 65535];
    loop {
        tokio::select! {
            Ok((stream, addr)) = listener.accept() => {
                log::info!("client connected (TCP): {}", addr);
                let (read_half, write_half) = stream.into_split();
                let id = next_id;
                next_id += 1;
                writers.insert(id, Arc::new(Mutex::new(write_half)));
                conn_addrs.insert(id, addr);

                let tx = cmd_tx.clone();
                tokio::spawn(async move {
                    let mut rh = read_half;
                    loop {
                        let buf = match zn_net::read_frame(&mut rh).await {
                            Ok(b) => b,
                            Err(e) => {
                                log::warn!("conn {}: {}", id, e);
                                break;
                            }
                        };
                        if let Ok(pkt) = bincode::deserialize::<ClientPacket>(&buf) {
                            let _ = tx.send((id, pkt)).await;
                        }
                    }
                    let _ = tx.send((id, ClientPacket::Hello { name: "__disconnect__".into() })).await;
                });
            }
            res = udp.socket.recv_from(&mut udp_buf) => {
                if let Ok((n, addr)) = res {
                    let data = &udp_buf[..n];
                    if let Ok(pkt) = bincode::deserialize::<ClientPacket>(data) {
                        // Любой разобранный пакет с известного адреса — признак
                        // жизни: молчащий UDP-канал больше не оставляет призрака.
                        if let Some(&pid) = addr_to_client.get(&addr) {
                            last_seen.insert(pid, Instant::now());
                        }
                        match pkt {
                            ClientPacket::Connect { token } => {
                                if let Some(uid) = validate_session(&pool, &token).await {
                                    // resolve or create player id
                                    let pid = *token_to_player.entry(token.clone()).or_insert_with(|| {
                                        let p = next_id;
                                        next_id += 1;
                                        p
                                    });
                                    log::info!("udp connect: user {uid} -> player {pid}");
                                    player_to_token.insert(pid, token.clone());
                                    let (player, profile, inventory, stash) =
                                        spawn_player(&pool, uid, pid, world.player_spawn).await;
                                    if let std::collections::hash_map::Entry::Vacant(e) = player_to_user.entry(pid) {
                                        e.insert(uid);
                                        world.add_player(player);
                                    }
                                    if let std::collections::hash_map::Entry::Vacant(e) = addr_to_client.entry(addr) {
                                        e.insert(pid);
                                        client_addrs.insert(pid, addr);
                                    }
                                    last_seen.insert(pid, Instant::now());
                                    snap_sync.insert(pid, SnapSync::default());
                                    raid_started.insert(pid, Instant::now());
                                    raid_start_xp
                                        .insert(pid, world.get_player(pid).map(|p| p.xp).unwrap_or(0));
                                    // send Welcome + Profile via UDP
                                    // Без префикса длины: датаграмма сама имеет границы,
                                    // клиент парсит её как сырой bincode.
                                    let welcome = ServerPacket::Welcome {
                                        id: pid,
                                        protocol: PROTOCOL_VERSION,
                                        bounds: terrain_bounds,
                                        obstacles: obstacles.clone(),
                                    };
                                    if let Ok(d) = bincode::serialize(&welcome) { let _ = udp.socket.send_to(&d, addr).await; }
                                    let prof = ServerPacket::Profile { profile, inventory };
                                    if let Ok(d) = bincode::serialize(&prof) { let _ = udp.socket.send_to(&d, addr).await; }
                                    let contents = ServerPacket::StashContents { items: stash };
                                    if let Ok(d) = bincode::serialize(&contents) { let _ = udp.socket.send_to(&d, addr).await; }
                                } else {
                                    log::warn!("udp connect: token invalid or expired");
                                }
                            }
                            ClientPacket::StartRaid => {
                                if let Some(&pid) = addr_to_client.get(&addr)
                                    && let Some(&uid) = player_to_user.get(&pid)
                                    && let Some((profile, inventory, stash)) =
                                        start_raid(&mut world, &pool, uid, pid).await
                                {
                                    snap_sync.insert(pid, SnapSync::default());
                                    raid_started.insert(pid, Instant::now());
                                    raid_start_xp.insert(
                                        pid,
                                        world.get_player(pid).map(|p| p.xp).unwrap_or(0),
                                    );
                                    if let Ok(d) = bincode::serialize(&ServerPacket::Profile {
                                        profile,
                                        inventory,
                                    }) {
                                        let _ = udp.socket.send_to(&d, addr).await;
                                    }
                                    if let Ok(d) = bincode::serialize(
                                        &ServerPacket::StashContents { items: stash },
                                    ) {
                                        let _ = udp.socket.send_to(&d, addr).await;
                                    }
                                    log::info!("player {pid} started a new raid");
                                }
                            }
                            ClientPacket::Input(cmd) => {
                                if let Some(&pid) = addr_to_client.get(&addr)
                                    && input_limiter.allow(pid, Instant::now())
                                {
                                    world.queue_input(pid, &cmd);
                                }
                            }
                            ClientPacket::Ping { nonce } => {
                                if addr_to_client.contains_key(&addr)
                                    && let Ok(d) =
                                        bincode::serialize(&ServerPacket::Pong { nonce })
                                {
                                    let _ = udp.socket.send_to(&d, addr).await;
                                }
                            }
                            ClientPacket::SnapshotAck { tick } => {
                                if let Some(&pid) = addr_to_client.get(&addr)
                                    && let Some(sync) = snap_sync.get_mut(&pid)
                                {
                                    // Чужой/будущий тик не должен становиться базой.
                                    sync.ack = tick.min(world.tick);
                                }
                            }
                            ClientPacket::Pong { nonce } => {
                                if let Some(&pid) = addr_to_client.get(&addr)
                                    && let Some((expected, sent)) = server_pings.get(&pid).copied()
                                    && expected == nonce
                                {
                                    // Поздний pong со старым nonce не должен
                                    // снимать уже отправленный новый замер.
                                    server_pings.remove(&pid);
                                    let sample = sent.elapsed().as_secs_f32() * 1000.0;
                                    let entry = rtt_ms.entry(pid).or_insert(sample);
                                    *entry = *entry * 0.8 + sample * 0.2;
                                }
                            }
                            ClientPacket::SetLoadout { weapon_id } => {
                                if let Some(&pid) = addr_to_client.get(&addr) {
                                    let reply = match handle_loadout(&mut world, pid, weapon_id) {
                                        Ok((weapon_id, ammo)) => {
                                            ServerPacket::Loadout { weapon_id, ammo }
                                        }
                                        Err(reason) => ServerPacket::LoadoutDenied {
                                            weapon_id,
                                            reason: reason.into(),
                                        },
                                    };
                                    if let Ok(d) = bincode::serialize(&reply) {
                                        let _ = udp.socket.send_to(&d, addr).await;
                                    }
                                }
                            }
                            ClientPacket::Shot {
                                origin,
                                dir,
                                client_time,
                                ..
                            } => {
                                if let Some(&pid) = addr_to_client.get(&addr) {
                                    let rtt = rtt_ms.get(&pid).copied().unwrap_or(0.0);
                                    let outcome =
                                        handle_shot(&mut world, pid, origin, dir, client_time, rtt);
                                    let reply = ServerPacket::ShotResult {
                                        hit: outcome.and_then(|o| o.hit),
                                        killed: outcome.map(|o| o.killed).unwrap_or(false),
                                        zone: outcome.and_then(|o| o.zone),
                                        damage: outcome.map(|o| o.damage).unwrap_or(0.0),
                                    };
                                    // Ответ нужен клиенту для звука и маркера попадания.
                                    if let Ok(d) = bincode::serialize(&reply) {
                                        let _ = udp.socket.send_to(&d, addr).await;
                                    }
                                }
                            }
                            ClientPacket::Pickup { entity_id } => {
                                if let Some(&pid) = addr_to_client.get(&addr) {
                                    handle_pickup(&mut world, pid, entity_id);
                                    send_inventory(&udp.socket, &world, pid, addr).await;
                                }
                            }
                            ClientPacket::Consume { item_id } => {
                                if let Some(&pid) = addr_to_client.get(&addr) {
                                    handle_consume(&mut world, pid, item_id);
                                    send_inventory(&udp.socket, &world, pid, addr).await;
                                }
                            }
                            ClientPacket::TakeFromStash { item_id, quantity } => {
                                if let Some(&pid) = addr_to_client.get(&addr)
                                    && let Some(&uid) = player_to_user.get(&pid)
                                {
                                    take_from_stash(&pool, &mut world, uid, pid, item_id, quantity).await;
                                    send_inventory(&udp.socket, &world, pid, addr).await;
                                    let items = load_stash(&pool, uid).await;
                                    if let Ok(d) =
                                        bincode::serialize(&ServerPacket::StashContents { items })
                                    {
                                        let _ = udp.socket.send_to(&d, addr).await;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some((conn_id, pkt)) = cmd_rx.recv() => {
                match pkt {
                    ClientPacket::Hello { name } => {
                        if name == "__disconnect__" {
                            if let Some(&pid) = conn_to_player.get(&conn_id) {
                                if let Some(&uid) = player_to_user.get(&pid)
                                    && let Some(p) = world.get_player(pid)
                                {
                                    persist_player(&persist, uid, p);
                                }
                                world.remove_player(pid);
                                input_limiter.forget(pid);
                                player_to_user.remove(&pid);
                                token_to_player.retain(|_, v| *v != pid);
                                if let Some(token) = player_to_token.remove(&pid) {
                                    let _ = persist.send(PersistJob::SessionDelete { token });
                                }
                                if let Some(a) = client_addrs.remove(&pid) {
                                    addr_to_client.remove(&a);
                                }
                                last_seen.remove(&pid);
                                snap_sync.remove(&pid);
                                rtt_ms.remove(&pid);
                                server_pings.remove(&pid);
                                raid_started.remove(&pid);
                                raid_start_xp.remove(&pid);
                                log::info!("player {} disconnected", pid);
                            }
                            writers.remove(&conn_id);
                            conn_to_player.remove(&conn_id);
                            conn_addrs.remove(&conn_id);
                        }
                    }
                    ClientPacket::Register { username, password } => {
                        let ip = conn_addrs.get(&conn_id).map(|a| a.ip());
                        if let Some(ip) = ip
                            && !allow_attempt(&mut login_attempts, ip, Instant::now())
                        {
                            if let Some(w) = writers.get(&conn_id) {
                                let mut g = w.lock().await;
                                let _ = send_packet(&mut *g, &ServerPacket::LoginResult { ok: false, token: None, error: Some("too many attempts, try again later".into()) }).await;
                            }
                            continue;
                        }
                        let result = handle_register(&pool, &username, &password).await;
                        match result {
                            Ok((token, _uid)) => {
                                if let Some(w) = writers.get(&conn_id) {
                                    let mut g = w.lock().await;
                                    let _ = send_packet(&mut *g, &ServerPacket::LoginResult { ok: true, token: Some(token), error: None }).await;
                                }
                            }
                            Err(e) => {
                                if let Some(w) = writers.get(&conn_id) {
                                    let mut g = w.lock().await;
                                    let _ = send_packet(&mut *g, &ServerPacket::LoginResult { ok: false, token: None, error: Some(e) }).await;
                                }
                            }
                        }
                    }
                    ClientPacket::Login { username, password } => {
                        let ip = conn_addrs.get(&conn_id).map(|a| a.ip());
                        if let Some(ip) = ip
                            && !allow_attempt(&mut login_attempts, ip, Instant::now())
                        {
                            if let Some(w) = writers.get(&conn_id) {
                                let mut g = w.lock().await;
                                let _ = send_packet(&mut *g, &ServerPacket::LoginResult { ok: false, token: None, error: Some("too many attempts, try again later".into()) }).await;
                            }
                            continue;
                        }
                        let result = handle_login(&pool, &username, &password).await;
                        match result {
                            Ok((token, _uid)) => {
                                if let Some(ip) = ip {
                                    login_attempts.remove(&ip);
                                }
                                if let Some(w) = writers.get(&conn_id) {
                                    let mut g = w.lock().await;
                                    let _ = send_packet(&mut *g, &ServerPacket::LoginResult { ok: true, token: Some(token), error: None }).await;
                                }
                            }
                            Err(e) => {
                                if let Some(w) = writers.get(&conn_id) {
                                    let mut g = w.lock().await;
                                    let _ = send_packet(&mut *g, &ServerPacket::LoginResult { ok: false, token: None, error: Some(e) }).await;
                                }
                            }
                        }
                    }
                    ClientPacket::Connect { token } => {
                        // TCP fallback
                        if let Some(uid) = validate_session(&pool, &token).await {
                            let pid = *token_to_player.entry(token.clone()).or_insert_with(|| {
                                let p = next_id;
                                next_id += 1;
                                p
                            });
                            player_to_token.insert(pid, token.clone());
                            let (player, profile, inventory, stash) =
                                spawn_player(&pool, uid, pid, world.player_spawn).await;
                            if let std::collections::hash_map::Entry::Vacant(e) = player_to_user.entry(pid) {
                                e.insert(uid);
                                world.add_player(player);
                            }
                            conn_to_player.insert(conn_id, pid);
                            snap_sync.insert(pid, SnapSync::default());
                            raid_started.insert(pid, Instant::now());
                            raid_start_xp
                                .insert(pid, world.get_player(pid).map(|p| p.xp).unwrap_or(0));
                            if let Some(w) = writers.get(&conn_id) {
                                let mut g = w.lock().await;
                                let _ = send_packet(&mut *g, &ServerPacket::Profile { profile, inventory }).await;
                                let _ = send_packet(&mut *g, &ServerPacket::StashContents { items: stash }).await;
                                let _ = send_packet(
                                    &mut *g,
                                    &ServerPacket::Welcome {
                                        id: pid,
                                        protocol: PROTOCOL_VERSION,
                                        bounds: terrain_bounds,
                                        obstacles: obstacles.clone(),
                                    },
                                )
                                .await;
                            }
                        } else {
                            if let Some(w) = writers.get(&conn_id) {
                                let mut g = w.lock().await;
                                let _ = send_packet(&mut *g, &ServerPacket::LoginResult { ok: false, token: None, error: Some("invalid token".into()) }).await;
                            }
                        }
                    }
                    ClientPacket::StartRaid => {
                        let pid = conn_to_player.get(&conn_id).copied().unwrap_or(conn_id);
                        if let Some(&uid) = player_to_user.get(&pid)
                            && let Some((profile, inventory, stash)) =
                                start_raid(&mut world, &pool, uid, pid).await
                        {
                            snap_sync.insert(pid, SnapSync::default());
                            raid_started.insert(pid, Instant::now());
                            raid_start_xp
                                .insert(pid, world.get_player(pid).map(|p| p.xp).unwrap_or(0));
                            if let Some(w) = writers.get(&conn_id) {
                                let mut g = w.lock().await;
                                let _ = send_packet(
                                    &mut *g,
                                    &ServerPacket::Profile {
                                        profile,
                                        inventory,
                                    },
                                )
                                .await;
                                let _ = send_packet(
                                    &mut *g,
                                    &ServerPacket::StashContents { items: stash },
                                )
                                .await;
                            }
                            log::info!("player {pid} started a new raid");
                        }
                    }
                    ClientPacket::Input(cmd) => {
                        let pid = conn_to_player.get(&conn_id).copied().unwrap_or(conn_id);
                        if input_limiter.allow(pid, Instant::now()) {
                            world.queue_input(pid, &cmd);
                        }
                    }
                    ClientPacket::Shot {
                        origin,
                        dir,
                        client_time,
                        ..
                    } => {
                        let pid = conn_to_player.get(&conn_id).copied().unwrap_or(conn_id);
                        let rtt = rtt_ms.get(&pid).copied().unwrap_or(0.0);
                        let outcome =
                            handle_shot(&mut world, pid, origin, dir, client_time, rtt);
                        let reply = ServerPacket::ShotResult {
                            hit: outcome.and_then(|o| o.hit),
                            killed: outcome.map(|o| o.killed).unwrap_or(false),
                            zone: outcome.and_then(|o| o.zone),
                            damage: outcome.map(|o| o.damage).unwrap_or(0.0),
                        };
                        if let Some(addr) = client_addrs.get(&pid).copied()
                            && let Ok(d) = bincode::serialize(&reply)
                        {
                            let _ = udp.socket.send_to(&d, addr).await;
                        }
                    }
                    ClientPacket::Pickup { entity_id } => {
                        let pid = conn_to_player.get(&conn_id).copied().unwrap_or(conn_id);
                        handle_pickup(&mut world, pid, entity_id);
                        // Клиент читает только UDP, поэтому даже для TCP-запроса
                        // ответ уходит на его UDP-адрес.
                        if let Some(addr) = client_addrs.get(&pid).copied() {
                            send_inventory(&udp.socket, &world, pid, addr).await;
                        }
                    }
                    ClientPacket::Consume { item_id } => {
                        let pid = conn_to_player.get(&conn_id).copied().unwrap_or(conn_id);
                        handle_consume(&mut world, pid, item_id);
                        if let Some(addr) = client_addrs.get(&pid).copied() {
                            send_inventory(&udp.socket, &world, pid, addr).await;
                        }
                    }
                    ClientPacket::TakeFromStash { item_id, quantity } => {
                        let pid = conn_to_player.get(&conn_id).copied().unwrap_or(conn_id);
                        if let Some(&uid) = player_to_user.get(&pid) {
                            take_from_stash(&pool, &mut world, uid, pid, item_id, quantity).await;
                            if let Some(addr) = client_addrs.get(&pid).copied() {
                                send_inventory(&udp.socket, &world, pid, addr).await;
                            }
                            if let Some(w) = writers.get(&conn_id) {
                                let items = load_stash(&pool, uid).await;
                                let mut g = w.lock().await;
                                let _ = send_packet(
                                    &mut *g,
                                    &ServerPacket::StashContents { items },
                                )
                                .await;
                            }
                        }
                    }
                    ClientPacket::SnapshotAck { tick } => {
                        let pid = conn_to_player.get(&conn_id).copied().unwrap_or(conn_id);
                        if let Some(sync) = snap_sync.get_mut(&pid) {
                            sync.ack = tick.min(world.tick);
                        }
                    }
                    ClientPacket::Pong { nonce } => {
                        let pid = conn_to_player.get(&conn_id).copied().unwrap_or(conn_id);
                        if let Some((expected, sent)) = server_pings.get(&pid).copied()
                            && expected == nonce
                        {
                            server_pings.remove(&pid);
                            let sample = sent.elapsed().as_secs_f32() * 1000.0;
                            let entry = rtt_ms.entry(pid).or_insert(sample);
                            *entry = *entry * 0.8 + sample * 0.2;
                        }
                    }
                    ClientPacket::Ping { nonce } => {
                        if let Some(w) = writers.get(&conn_id) {
                            let mut g = w.lock().await;
                            let _ = send_packet(&mut *g, &ServerPacket::Pong { nonce }).await;
                        }
                    }
                    ClientPacket::SetLoadout { weapon_id } => {
                        let pid = conn_to_player.get(&conn_id).copied().unwrap_or(conn_id);
                        let reply = match handle_loadout(&mut world, pid, weapon_id) {
                            Ok((weapon_id, ammo)) => ServerPacket::Loadout { weapon_id, ammo },
                            Err(reason) => ServerPacket::LoadoutDenied {
                                weapon_id,
                                reason: reason.into(),
                            },
                        };
                        if let Some(w) = writers.get(&conn_id) {
                            let mut g = w.lock().await;
                            let _ = send_packet(&mut *g, &reply).await;
                        }
                    }
                }
            }
            _ = tick_timer.tick() => {
                let tick_start = Instant::now();
                world.tick(1.0 / 60.0);
                tick_stats.record(tick_start.elapsed());
                // События только планируют запись в БД: канал не ждёт диск.
                for (pid, packet) in handle_events(
                    &mut world,
                    &player_to_user,
                    &persist,
                    &mut raid_started,
                    &mut raid_start_xp,
                ) {
                    if let Some(addr) = client_addrs.get(&pid).copied()
                        && let Ok(data) = bincode::serialize(&packet)
                    {
                        let _ = udp.socket.send_to(&data, addr).await;
                    }
                }
                snapshot_counter += 1;
                if snapshot_counter >= 3 {
                    snapshot_counter = 0;
                    for (pid, addr) in &client_addrs {
                        // Игрок на экране результата вне мира: слать ему нечего.
                        if world.get_player(*pid).is_none() {
                            continue;
                        }
                        let full = snapshot_for(&world, *pid);
                        let packet = {
                            let sync = snap_sync.entry(*pid).or_default();
                            let base = sync.history.iter().find(|s| s.tick == sync.ack);
                            let mut chosen = ServerPacket::Snapshot(full.clone());
                            if let Some(base) = base {
                                let delta = zn_net::SnapshotDelta::between(base, &full);
                                let full_bytes = bincode::serialize(&chosen)
                                    .map(|d| d.len())
                                    .unwrap_or(usize::MAX);
                                let delta_bytes =
                                    bincode::serialize(&ServerPacket::SnapshotDelta(delta.clone()))
                                        .map(|d| d.len())
                                        .unwrap_or(usize::MAX);
                                if delta_bytes < full_bytes {
                                    chosen = ServerPacket::SnapshotDelta(delta);
                                }
                            }
                            sync.history.push_back(full);
                            while sync.history.len() > SNAPSHOT_HISTORY {
                                sync.history.pop_front();
                            }
                            chosen
                        };
                        if let Ok(data) = bincode::serialize(&packet) {
                            snapshot_bytes += data.len();
                            snapshot_rounds += 1;
                            if matches!(&packet, ServerPacket::SnapshotDelta(_)) {
                                snap_delta_frames += 1;
                            } else {
                                snap_key_frames += 1;
                            }
                            let _ = udp.socket.send_to(&data, *addr).await;
                        }
                    }
                }
            }
            _ = server_ping_timer.tick() => {
                for (pid, addr) in &client_addrs {
                    let nonce = world.tick;
                    server_pings.insert(*pid, (nonce, Instant::now()));
                    if let Ok(d) = bincode::serialize(&ServerPacket::Ping { nonce }) {
                        let _ = udp.socket.send_to(&d, *addr).await;
                    }
                }
            }
            _ = stats_timer.tick() => {
                let m = world.metrics();
                let snap_avg = snapshot_bytes.checked_div(snapshot_rounds).unwrap_or(0);
                log::info!(
                    "[tick] tick={} players={} creatures={} loot={} events={} \
                     tick_avg={:.2}ms tick_max={:.2}ms snap_avg={}B \
                     snap_delta={} snap_key={}",
                    m.tick,
                    m.players,
                    m.creatures,
                    m.loot,
                    m.pending_events,
                    tick_stats.avg_ms(),
                    tick_stats.max_ms(),
                    snap_avg,
                    snap_delta_frames,
                    snap_key_frames
                );
                tick_stats.reset();
                snapshot_bytes = 0;
                snapshot_rounds = 0;
                snap_delta_frames = 0;
                snap_key_frames = 0;
            }
            _ = idle_sweep.tick() => {
                let idle = idle_players(&last_seen, Instant::now(), IDLE_TIMEOUT);
                for pid in idle {
                    if let Some(&uid) = player_to_user.get(&pid)
                        && let Some(p) = world.get_player(pid)
                    {
                        persist_player(&persist, uid, p);
                    }
                    log::info!("player {pid} timed out (no packets)");
                    world.remove_player(pid);
                    input_limiter.forget(pid);
                    last_seen.remove(&pid);
                    snap_sync.remove(&pid);
                    rtt_ms.remove(&pid);
                    server_pings.remove(&pid);
                    raid_started.remove(&pid);
                    raid_start_xp.remove(&pid);
                    player_to_user.remove(&pid);
                    token_to_player.retain(|_, v| *v != pid);
                    if let Some(token) = player_to_token.remove(&pid) {
                        let _ = persist.send(PersistJob::SessionDelete { token });
                    }
                    if let Some(a) = client_addrs.remove(&pid) {
                        addr_to_client.remove(&a);
                    }
                }
            }
            _ = session_sweep.tick() => {
                let _ = sqlx::query("DELETE FROM sessions WHERE expires_at < datetime('now')")
                    .execute(&pool)
                    .await;
            }
            _ = save_timer.tick() => {
                for (pid, uid) in &player_to_user {
                    if let Some(p) = world.get_player(*pid) {
                        persist_player(&persist, *uid, p);
                    }
                }
            }
        }
    }
}

/// Радиус подбора: клиент берёт с 2 м, запас на лаг.
const PICKUP_RANGE: f32 = 3.0;

/// Сколько кадров истории хранится на клиента. 64 кадра при 20 Гц — больше
/// трёх секунд: дельта от подтверждённого кадра переживает потерю пакетов.
const SNAPSHOT_HISTORY: usize = 64;
/// Радиусы интереса. Игрок видит ближайший мир, а не всю карту: дельта и
/// фильтрация вместе дают основной выигрыш по трафику.
const SNAPSHOT_PLAYER_RADIUS: f32 = 100.0;
const SNAPSHOT_CREATURE_RADIUS: f32 = 120.0;
const SNAPSHOT_LOOT_RADIUS: f32 = 60.0;
const SNAPSHOT_ANOMALY_RADIUS: f32 = 90.0;
const SNAPSHOT_ARTIFACT_RADIUS: f32 = 60.0;

#[derive(Default)]
struct SnapSync {
    /// Последний кадр, который клиент подтвердил.
    ack: u64,
    /// Полные кадры за последние секунды; из них выбирается база дельты.
    history: VecDeque<zn_sim::Snapshot>,
}

/// Полный кадр только с тем, что игрок реально может увидеть.
fn snapshot_for(world: &SimWorld, pid: u64) -> zn_sim::Snapshot {
    let center = world
        .get_player(pid)
        .map(|p| p.position)
        .unwrap_or_default();
    let near = |pos: Vec3, radius: f32| (pos - center).length() <= radius;
    zn_sim::Snapshot {
        tick: world.tick,
        players: world
            .players
            .iter()
            .filter(|p| p.id == pid || near(p.position, SNAPSHOT_PLAYER_RADIUS))
            .map(|p| zn_sim::PlayerState {
                id: p.id,
                name: p.name.clone(),
                position: p.position,
                health: p.health,
                radiation: p.radiation,
                hunger: p.hunger,
                thirst: p.thirst,
                yaw: p.yaw,
                pitch: p.pitch,
                level: p.level,
                xp: p.xp,
                velocity: p.velocity,
                weapon_id: p.equipped_weapon,
                ammo: p.ammo,
                last_input_seq: p.last_input_seq,
            })
            .collect(),
        anomalies: world
            .anomalies
            .iter()
            .filter(|a| near(a.position, SNAPSHOT_ANOMALY_RADIUS))
            .cloned()
            .collect(),
        artifacts: world
            .artifacts
            .iter()
            .filter(|a| near(a.position, SNAPSHOT_ARTIFACT_RADIUS))
            .cloned()
            .collect(),
        loot: world
            .loot
            .iter()
            .filter(|l| near(l.position, SNAPSHOT_LOOT_RADIUS))
            .cloned()
            .collect(),
        creatures: world
            .creatures
            .iter()
            .filter(|c| near(c.position, SNAPSHOT_CREATURE_RADIUS))
            .cloned()
            .collect(),
        extraction_zones: world.extraction_zones.clone(),
    }
}

/// Длительность серверного тика и предел компенсации лага.
const TICK_MS: f64 = 1000.0 / 60.0;
const MAX_LAG_COMP_MS: f64 = 200.0;

/// Сколько игрок может молчать, прежде чем сервер уберёт его из мира.
///
/// У UDP нет события «соединение закрыто», а TCP-канал живёт отдельно:
/// без таймаута замолчавший клиент остаётся в снапшотах навсегда.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// Статистика длительности тика за окно между логами.
///
/// Среднее без максимума бесполезно: один тик в 200 мс ломает движение,
/// но почти не меняет среднее.
#[derive(Default)]
struct TickStats {
    samples: u32,
    total: Duration,
    worst: Duration,
}

impl TickStats {
    fn record(&mut self, elapsed: Duration) {
        self.samples += 1;
        self.total += elapsed;
        if elapsed > self.worst {
            self.worst = elapsed;
        }
    }

    fn avg_ms(&self) -> f64 {
        if self.samples == 0 {
            0.0
        } else {
            self.total.as_secs_f64() * 1000.0 / self.samples as f64
        }
    }

    fn max_ms(&self) -> f64 {
        self.worst.as_secs_f64() * 1000.0
    }

    fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Игроки, от которых давно не было пакетов. Сортировка — чтобы лог и тест
/// были детерминированными.
fn idle_players(last_seen: &HashMap<u64, Instant>, now: Instant, timeout: Duration) -> Vec<u64> {
    let mut out: Vec<u64> = last_seen
        .iter()
        .filter(|(_, at)| now.saturating_duration_since(**at) >= timeout)
        .map(|(pid, _)| *pid)
        .collect();
    out.sort_unstable();
    out
}

/// Работа по записи в БД, которую выполняет отдельная задача.
///
/// Раньше `handle_events` вызывал `save_*` прямо в ветке тика: ожидание диска
/// (а на VPS это ещё и антивирус) останавливало мир на всё время записи.
/// Канал сохраняет порядок: инвентарь, отправленный при эвакуации, доедет
/// до БД раньше следующей периодической записи.
enum PersistJob {
    Player {
        user_id: i64,
        player: Player,
    },
    Inventory {
        user_id: i64,
        inventory: Inventory,
    },
    /// Вынесенный лут кладётся в отдельную таблицу: очистка рейдового
    /// инвентаря не должна стирать награду.
    StashAdd {
        user_id: i64,
        items: Vec<InventoryItem>,
    },
    SessionDelete {
        token: String,
    },
}

fn persist_player(tx: &UnboundedSender<PersistJob>, user_id: i64, player: &Player) {
    let _ = tx.send(PersistJob::Player {
        user_id,
        player: player.clone(),
    });
    let _ = tx.send(PersistJob::Inventory {
        user_id,
        inventory: player.inventory.clone(),
    });
}

fn spawn_persister(pool: SqlitePool) -> UnboundedSender<PersistJob> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PersistJob>();
    tokio::spawn(async move {
        while let Some(job) = rx.recv().await {
            match job {
                PersistJob::Player { user_id, player } => {
                    save_profile(&pool, user_id, Some(&player)).await;
                }
                PersistJob::Inventory { user_id, inventory } => {
                    save_inventory(&pool, user_id, &inventory).await;
                }
                PersistJob::StashAdd { user_id, items } => {
                    stash_add(&pool, user_id, &items).await;
                }
                PersistJob::SessionDelete { token } => delete_session(&pool, &token).await,
            }
        }
    });
    tx
}

/// Отправляет игроку его текущий инвентарь по UDP.
async fn send_inventory(
    socket: &tokio::net::UdpSocket,
    world: &SimWorld,
    pid: u64,
    addr: SocketAddr,
) {
    if let Some(p) = world.get_player(pid)
        && let Ok(data) = bincode::serialize(&ServerPacket::Inventory(p.inventory.clone()))
    {
        let _ = socket.send_to(&data, addr).await;
    }
}

/// Оценка отката: RTT/2, но не дальше окна и не дальше метки клиента.
///
/// Клиент присылает момент выстрела в серверной шкале. Если метка правдоподобна,
/// берётся минимум из RTT/2 и односторонней задержки: подделанное старое время
/// не откатит цель дальше честной оценки RTT. Без метки (0) работает только RTT.
fn rewind_ticks(server_tick: u64, client_time: u64, rtt_ms: f32) -> u32 {
    let rtt_half = (rtt_ms as f64 / 2.0).clamp(0.0, MAX_LAG_COMP_MS);
    let server_ms = server_tick as f64 * TICK_MS;
    let one_way = (server_ms - client_time as f64).clamp(0.0, MAX_LAG_COMP_MS);
    let lag = if client_time == 0 {
        rtt_half
    } else {
        one_way.min(rtt_half)
    };
    (lag / TICK_MS).round() as u32
}

/// Передаёт выстрел в симуляцию и пишет в лог факт убийства.
///
/// Никакой арифметики урона здесь нет: хитбоксы, дистанция, стены, XP и
/// смерть живут в `zn_sim::SimWorld::apply_shot_rewound`.
fn handle_shot(
    world: &mut SimWorld,
    pid: u64,
    origin: [f32; 3],
    dir: [f32; 3],
    client_time: u64,
    rtt_ms: f32,
) -> Option<zn_sim::ShotOutcome> {
    let rewind = rewind_ticks(world.tick, client_time, rtt_ms);
    let Some(outcome) = world.apply_shot_rewound(pid, Vec3::from(origin), Vec3::from(dir), rewind)
    else {
        // Отказ — чужая точка, мусорный вектор, кулдаун или пустой магазин.
        // Это же основа для разбора инцидентов и античита.
        log::debug!("shot rejected from player {pid}: origin={origin:?} dir={dir:?}");
        return None;
    };
    log::debug!(
        "player {pid} shot {:?} in {:?} for {:.0} damage (rewind={} tick={} clamped={})",
        outcome.hit,
        outcome.zone,
        outcome.damage,
        rewind,
        outcome.rewind_tick,
        outcome.rewind_clamped
    );
    if outcome.killed {
        log::info!("player {pid} killed {:?} ({:?})", outcome.hit, outcome.zone);
    }
    Some(outcome)
}

fn handle_pickup(world: &mut SimWorld, pid: u64, entity_id: u64) -> bool {
    let player_pos = match world.get_player(pid) {
        Some(p) => p.position,
        None => return false,
    };
    let Some(kind) = zn_sim::ids::classify(entity_id) else {
        return false;
    };

    match kind {
        zn_sim::ids::IdKind::Loot => {
            let Some(idx) = world.loot.iter().position(|l| l.id == entity_id) else {
                return false;
            };
            if (world.loot[idx].position - player_pos).length() > PICKUP_RANGE {
                return false;
            }
            let item = world.loot.remove(idx).item;
            if let Some(p) = world.get_player_mut(pid) {
                p.inventory.add_item(item.id as i64, 1);
            }
            log::info!("player {} picked up loot {}", pid, entity_id);
            true
        }
        zn_sim::ids::IdKind::Artifact => {
            let artifact_id = entity_id - zn_sim::ids::ARTIFACT_BASE;
            let Some(idx) = world.artifacts.iter().position(|a| a.id == artifact_id) else {
                return false;
            };
            if (world.artifacts[idx].position - player_pos).length() > PICKUP_RANGE {
                return false;
            }
            let art = world.artifacts.remove(idx);
            if let Some(p) = world.get_player_mut(pid) {
                p.health = (p.health + 20.0).min(p.max_health);
                // Артефакт живёт в своём диапазоне id, иначе его съедают как аптечку.
                p.inventory
                    .add_item((zn_sim::ids::ARTIFACT_BASE + art.id) as i64, 1);
            }
            log::info!("player {} picked up artifact {}", pid, entity_id);
            true
        }
        _ => false,
    }
}

/// Эффект расходника берётся из каталога: сервер больше не держит второй
/// `match` с числами лечения.
fn handle_consume(world: &mut SimWorld, pid: u64, item_id: u64) {
    if item_id == 0 || item_id > zn_sim::ids::ITEM_MAX {
        return;
    }
    let heal = items::heal(item_id);
    if heal <= 0.0 {
        return;
    }
    if let Some(p) = world.get_player_mut(pid) {
        let iid = item_id as i64;
        if p.inventory.has_item(iid, 1) {
            p.inventory.remove_item(iid, 1);
            p.health = (p.health + heal).min(p.max_health);
            log::info!("player {} consumed item {}", pid, item_id);
        }
    }
}

/// Меняет оружие, если id есть в каталоге и оружие принадлежит игроку.
///
/// ПМ — штатный пистолет: он есть всегда, иначе после смерти игрок остался бы
/// без оружия и не смог выйти из рейда. Остальное оружие должно лежать
/// в рейдовом инвентаре; туда его кладёт `TakeFromStash` из схрона.
fn handle_loadout(
    world: &mut SimWorld,
    pid: u64,
    weapon_id: u64,
) -> Result<(u64, u32), &'static str> {
    if !items::is_weapon(weapon_id) {
        return Err("unknown weapon");
    }
    let p = world.get_player_mut(pid).ok_or("no player")?;
    if weapon_id != items::PISTOL && !p.inventory.has_item(weapon_id as i64, 1) {
        return Err("weapon not in inventory");
    }
    p.equipped_weapon = weapon_id;
    p.shot_cooldown = 0.0;
    Ok((p.equipped_weapon, p.ammo))
}

/// Раскладывает игровые события в задания на запись. `async` здесь больше нет:
/// тик не ждёт БД, а канал сохраняет порядок записей.
fn handle_events(
    world: &mut SimWorld,
    player_to_user: &HashMap<u64, i64>,
    persist: &UnboundedSender<PersistJob>,
    raid_started: &mut HashMap<u64, Instant>,
    raid_start_xp: &mut HashMap<u64, i64>,
) -> Vec<(u64, ServerPacket)> {
    let mut changed: Vec<(u64, ServerPacket)> = Vec::new();
    let events: Vec<zn_sim::GameEvent> = std::mem::take(&mut world.events);
    for ev in events {
        match ev {
            zn_sim::GameEvent::ExtractionSuccess { player_id, .. } => {
                // XP начисляет симуляция в `zn_sim::tick` — второй раз здесь нельзя.
                let (uid, inv, xp_now) =
                    match (player_to_user.get(&player_id), world.get_player(player_id)) {
                        (Some(uid), Some(p)) => (*uid, p.inventory.clone(), p.xp),
                        _ => continue,
                    };
                // Награда уходит в схрон, рейдовый инвентарь очищается.
                // Задания владеют снимками: запись уже ничего не потеряет.
                let _ = persist.send(PersistJob::StashAdd {
                    user_id: uid,
                    items: inv.items.clone(),
                });
                let _ = persist.send(PersistJob::Inventory {
                    user_id: uid,
                    inventory: Inventory::default(),
                });
                if let Some(p) = world.get_player_mut(player_id) {
                    p.inventory = Inventory::default();
                }
                // Профиль с новым XP и пустым инвентарём: схрон живёт отдельно.
                if let Some(p) = world.get_player(player_id) {
                    persist_player(persist, uid, p);
                }
                let stacks = inv.items.len();
                changed.push((player_id, ServerPacket::Inventory(Inventory::default())));
                changed.push((
                    player_id,
                    ServerPacket::Stash {
                        items: inv.items.clone(),
                    },
                ));
                changed.push((
                    player_id,
                    ServerPacket::RaidResult {
                        result: RaidResult {
                            reason: RaidEndReason::Extracted,
                            xp_gained: xp_now - raid_start_xp.remove(&player_id).unwrap_or(xp_now),
                            items: inv.items,
                            duration_secs: raid_started
                                .remove(&player_id)
                                .map(|at| at.elapsed().as_secs() as u32)
                                .unwrap_or(0),
                        },
                    },
                ));
                world.remove_player(player_id);
                log::info!("player {player_id} extracted, {stacks} stacks to stash, raid ended");
            }
            zn_sim::GameEvent::PlayerDeath {
                player_id,
                dropped_loot,
                ..
            } => {
                // Лут выложен и респаун выполнен в симуляции: здесь только запись в БД.
                let Some(&uid) = player_to_user.get(&player_id) else {
                    continue;
                };
                let Some((xp_now, inventory)) = world
                    .get_player(player_id)
                    .map(|p| (p.xp, p.inventory.clone()))
                else {
                    continue;
                };
                let lost: Vec<InventoryItem> = dropped_loot
                    .iter()
                    .map(|item| InventoryItem {
                        item_id: item.id as i64,
                        quantity: 1,
                        slot: 0,
                    })
                    .collect();
                if let Some(p) = world.get_player(player_id) {
                    persist_player(persist, uid, p);
                }
                changed.push((player_id, ServerPacket::Inventory(inventory)));
                changed.push((
                    player_id,
                    ServerPacket::RaidResult {
                        result: RaidResult {
                            reason: RaidEndReason::Died,
                            xp_gained: xp_now - raid_start_xp.remove(&player_id).unwrap_or(xp_now),
                            items: lost,
                            duration_secs: raid_started
                                .remove(&player_id)
                                .map(|at| at.elapsed().as_secs() as u32)
                                .unwrap_or(0),
                        },
                    },
                ));
                world.remove_player(player_id);
                log::info!("player {player_id} died, raid ended");
            }
            _ => {}
        }
    }
    changed
}

/// Сколько неудачных попыток входа допускается с одного IP за окно.
const LOGIN_MAX_ATTEMPTS: u32 = 5;
const LOGIN_WINDOW: Duration = Duration::from_secs(300);

#[derive(Debug)]
struct Attempts {
    count: u32,
    window_start: Instant,
}

/// Счётчик попыток по IP с фиксированным окном. Ключ — адрес, а не имя
/// пользователя: иначе чужой человек мог заблокировать вход жертве.
fn allow_attempt(map: &mut HashMap<IpAddr, Attempts>, ip: IpAddr, now: Instant) -> bool {
    let entry = map.entry(ip).or_insert(Attempts {
        count: 0,
        window_start: now,
    });
    if now.duration_since(entry.window_start) >= LOGIN_WINDOW {
        entry.count = 0;
        entry.window_start = now;
    }
    if entry.count >= LOGIN_MAX_ATTEMPTS {
        return false;
    }
    entry.count += 1;
    true
}

/// Токен действителен только если он есть в БД и не истёк.
/// Раньше токены жили в HashMap и не проверялись никогда.
async fn validate_session(pool: &SqlitePool, token: &str) -> Option<i64> {
    sqlx::query_scalar(
        "SELECT user_id FROM sessions WHERE token = ? AND expires_at > datetime('now')",
    )
    .bind(token)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

async fn delete_session(pool: &SqlitePool, token: &str) {
    let _ = sqlx::query("DELETE FROM sessions WHERE token = ?")
        .bind(token)
        .execute(pool)
        .await;
}

async fn handle_register(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> Result<(String, i64), String> {
    if username.trim().is_empty() {
        return Err("empty username".into());
    }
    if password.len() < 8 {
        return Err("password too short (min 8)".into());
    }
    let exists: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE username = ?")
        .bind(username)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())?;
    if exists.is_some() {
        return Err("username already exists".into());
    }
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| e.to_string())?
        .to_string();
    let res = sqlx::query("INSERT INTO users (username, password_hash) VALUES (?, ?)")
        .bind(username)
        .bind(&hash)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    let id = res.last_insert_rowid();
    let _ = sqlx::query("INSERT INTO profiles (user_id) VALUES (?)")
        .bind(id)
        .execute(pool)
        .await;
    let token = uuid::Uuid::new_v4().to_string();
    let _ = sqlx::query(
        "INSERT INTO sessions (token, user_id, expires_at) VALUES (?, ?, datetime('now','+1 day'))",
    )
    .bind(&token)
    .bind(id)
    .execute(pool)
    .await;
    Ok((token, id))
}

// handle_login, load_profile, etc.
async fn handle_login(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> Result<(String, i64), String> {
    if username.trim().is_empty() {
        return Err("empty username".into());
    }
    if password.len() < 8 {
        return Err("password too short (min 8)".into());
    }
    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;

    let user_id = if let Some((id, hash)) = row {
        let parsed = PasswordHash::new(&hash).map_err(|e| e.to_string())?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| "invalid password".to_string())?;
        id
    } else {
        return Err("user not found".into());
    };

    // session token with expiry 24h
    let token = uuid::Uuid::new_v4().to_string();
    let _ = sqlx::query(
        "INSERT INTO sessions (token, user_id, expires_at) VALUES (?, ?, datetime('now','+1 day'))",
    )
    .bind(&token)
    .bind(user_id)
    .execute(pool)
    .await;
    Ok((token, user_id))
}

/// Строка профиля: level, xp, currency, позиция, оружие, патроны.
type ProfileRow = (i32, i64, i64, f64, f64, f64, i64, i64);

async fn load_profile(pool: &SqlitePool, user_id: i64) -> Option<PlayerProfile> {
    let row: Option<ProfileRow> = sqlx::query_as(
        "SELECT level, xp, currency, pos_x, pos_y, pos_z, weapon_id, ammo FROM profiles WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let username: String = sqlx::query_scalar("SELECT username FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .unwrap_or_else(|| "player".into());
    if let Some((level, xp, currency, px, py, pz, weapon_id, ammo)) = row {
        Some(PlayerProfile {
            user_id,
            username,
            level,
            xp,
            currency,
            position: Vec3::new(px as f32, py as f32, pz as f32),
            weapon_id: weapon_id.max(0) as u64,
            ammo: ammo.clamp(0, items::MAX_AMMO as i64) as u32,
        })
    } else {
        Some(PlayerProfile::new(user_id, username))
    }
}

/// Создаёт игрока с уже применённым сохранённым прогрессом.
///
/// Профиль и инвентарь читаются один раз: раньше оба пути подключения грузили
/// их дважды, а `level`/`xp`/`currency` вообще не переносились в `Player`,
/// из-за чего первое же сохранение обнуляло прогресс.
/// Поднимает игрока в новый рейд после эвакуации или смерти.
///
/// Возвращает `None`, если игрок уже в мире: повторный `StartRaid` не должен
/// телепортировать его посреди боя. Точка появления всегда берётся с карты,
/// а не из сохранённого профиля.
async fn start_raid(
    world: &mut SimWorld,
    pool: &SqlitePool,
    uid: i64,
    pid: u64,
) -> Option<(PlayerProfile, Inventory, Vec<InventoryItem>)> {
    if world.get_player(pid).is_some() {
        return None;
    }
    let (mut player, mut profile, inventory, stash) =
        spawn_player(pool, uid, pid, world.player_spawn).await;
    player.position = world.player_spawn;
    player.health = player.max_health;
    profile.position = player.position;
    world.add_player(player);
    Some((profile, inventory, stash))
}

async fn spawn_player(
    pool: &SqlitePool,
    uid: i64,
    pid: u64,
    spawn: Vec3,
) -> (Player, PlayerProfile, Inventory, Vec<InventoryItem>) {
    let mut profile = load_profile(pool, uid)
        .await
        .unwrap_or_else(|| PlayerProfile::new(uid, "player"));
    let inventory = load_inventory(pool, uid).await;
    let stash = load_stash(pool, uid).await;
    let mut player = Player::new(pid, profile.username.clone());
    player.apply_profile(&profile);
    // Новичок приходит с нулевой позицией из дефолтного профиля — ставим его
    // на точку спавна карты. Сохранённая позиция игрока не перетирается.
    if player.position == Vec3::ZERO {
        player.position = spawn;
    }
    player.inventory = inventory.clone();
    // Профиль уходит клиенту: он должен начать предсказание с той же точки,
    // а не с сохранённого нуля.
    profile.position = player.position;
    (player, profile, inventory, stash)
}

/// Содержимое схрона. Отдаётся клиенту целиком, чтобы экран не собирал его
/// из дельт: после переподключения дельты потеряны.
async fn load_stash(pool: &SqlitePool, user_id: i64) -> Vec<InventoryItem> {
    let rows: Vec<(i64, i32)> = sqlx::query_as(
        "SELECT item_id, quantity FROM stash_items WHERE user_id = ? ORDER BY item_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .enumerate()
        .map(|(slot, (item_id, quantity))| InventoryItem {
            item_id,
            quantity,
            slot: slot as i32,
        })
        .collect()
}

async fn load_inventory(pool: &SqlitePool, user_id: i64) -> Inventory {
    let rows: Vec<(i64, i32, i32)> =
        sqlx::query_as("SELECT item_id, quantity, slot FROM inventory_items WHERE user_id = ?")
            .bind(user_id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    Inventory {
        items: rows
            .into_iter()
            .map(|(item_id, quantity, slot)| InventoryItem {
                item_id,
                quantity,
                slot,
            })
            .collect(),
    }
}

async fn save_profile(pool: &SqlitePool, user_id: i64, player: Option<&Player>) {
    if let Some(p) = player {
        let _ = sqlx::query(
            "INSERT INTO profiles (user_id, level, xp, currency, pos_x, pos_y, pos_z, weapon_id, ammo) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(user_id) DO UPDATE SET level=excluded.level, xp=excluded.xp, \
             currency=excluded.currency, pos_x=excluded.pos_x, pos_y=excluded.pos_y, \
             pos_z=excluded.pos_z, weapon_id=excluded.weapon_id, ammo=excluded.ammo"
        )
        .bind(user_id)
        .bind(p.level)
        .bind(p.xp)
        .bind(p.currency)
        .bind(p.position.x)
        .bind(p.position.y)
        .bind(p.position.z)
        .bind(p.equipped_weapon as i64)
        .bind(p.ammo as i64)
        .execute(pool)
        .await;
    } else {
        let _ = sqlx::query(
            "INSERT INTO profiles (user_id) VALUES (?) ON CONFLICT(user_id) DO NOTHING",
        )
        .bind(user_id)
        .execute(pool)
        .await;
    }
}

/// Переписывает инвентарь одной транзакцией.
///
/// `DELETE` + `INSERT` по отдельности оставляли пустой инвентарь, если между
/// ними падал процесс или заканчивалось место: игрок терял всё вынесенное.
async fn save_inventory(pool: &SqlitePool, user_id: i64, inv: &Inventory) {
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("inventory tx for user {user_id}: {e}");
            return;
        }
    };
    if let Err(e) = sqlx::query("DELETE FROM inventory_items WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut *tx)
        .await
    {
        log::error!("inventory clear for user {user_id}: {e}");
        return;
    }
    for it in &inv.items {
        if let Err(e) = sqlx::query(
            "INSERT INTO inventory_items (user_id, item_id, quantity, slot) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(it.item_id)
        .bind(it.quantity)
        .bind(it.slot)
        .execute(&mut *tx)
        .await
        {
            log::error!("inventory insert for user {user_id}: {e}");
            return;
        }
    }
    if let Err(e) = tx.commit().await {
        log::error!("inventory commit for user {user_id}: {e}");
    }
}

/// Добавляет вынесенный лут в схрон, суммируя стеки.
///
/// Транзакция обязательна: половина стека в схроне хуже, чем ничего.
async fn take_from_stash(
    pool: &SqlitePool,
    world: &mut SimWorld,
    user_id: i64,
    pid: u64,
    item_id: u64,
    quantity: u32,
) -> bool {
    // Игрок обязан существовать до записи: иначе предмет уйдёт из БД в никуда.
    if items::get(item_id).is_none()
        || quantity == 0
        || quantity > 999
        || world.get_player(pid).is_none()
    {
        return false;
    }
    let item_id = item_id as i64;
    let quantity = quantity as i32;
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("stash take tx for user {user_id}: {e}");
            return false;
        }
    };
    let have: Option<i32> =
        sqlx::query_scalar("SELECT quantity FROM stash_items WHERE user_id = ? AND item_id = ?")
            .bind(user_id)
            .bind(item_id)
            .fetch_optional(&mut *tx)
            .await
            .ok()
            .flatten();
    let Some(have) = have else {
        return false;
    };
    if have < quantity {
        return false;
    }
    let res = if have == quantity {
        sqlx::query("DELETE FROM stash_items WHERE user_id = ? AND item_id = ?")
            .bind(user_id)
            .bind(item_id)
            .execute(&mut *tx)
            .await
    } else {
        sqlx::query(
            "UPDATE stash_items SET quantity = quantity - ? WHERE user_id = ? AND item_id = ?",
        )
        .bind(quantity)
        .bind(user_id)
        .bind(item_id)
        .execute(&mut *tx)
        .await
    };
    if let Err(e) = res {
        log::error!("stash take for user {user_id}: {e}");
        return false;
    }
    if let Err(e) = tx.commit().await {
        log::error!("stash take commit for user {user_id}: {e}");
        return false;
    }
    if let Some(p) = world.get_player_mut(pid) {
        p.inventory.add_item(item_id, quantity);
    }
    true
}

async fn stash_add(pool: &SqlitePool, user_id: i64, items: &[InventoryItem]) {
    if items.is_empty() {
        return;
    }
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            log::error!("stash tx for user {user_id}: {e}");
            return;
        }
    };
    for it in items {
        if let Err(e) = sqlx::query(
            "INSERT INTO stash_items (user_id, item_id, quantity) VALUES (?, ?, ?) \
             ON CONFLICT(user_id, item_id) DO UPDATE SET quantity = quantity + excluded.quantity",
        )
        .bind(user_id)
        .bind(it.item_id)
        .bind(it.quantity)
        .execute(&mut *tx)
        .await
        {
            log::error!("stash insert for user {user_id}: {e}");
            return;
        }
    }
    if let Err(e) = tx.commit().await {
        log::error!("stash commit for user {user_id}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world_with_loot() -> SimWorld {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        w.loot.push(zn_sim::LootItem {
            id: zn_sim::ids::LOOT_BASE,
            position: Vec3::new(50.0, 0.5, 50.0),
            item: zn_sim::Item {
                id: 1,
                name: "Medkit".into(),
                value: 1,
                consumable: true,
            },
        });
        w
    }

    #[test]
    fn rate_limit_blocks_after_five_attempts() {
        let mut map: HashMap<IpAddr, Attempts> = HashMap::new();
        let ip: IpAddr = "10.0.0.1".parse().unwrap();
        let t0 = Instant::now();

        for _ in 0..LOGIN_MAX_ATTEMPTS {
            assert!(allow_attempt(&mut map, ip, t0));
        }
        assert!(
            !allow_attempt(&mut map, ip, t0),
            "шестая попытка должна блокироваться"
        );

        // соседний адрес не страдает от чужого перебора
        let other: IpAddr = "10.0.0.2".parse().unwrap();
        assert!(allow_attempt(&mut map, other, t0));

        // окно истекло — снова можно
        let later = t0 + LOGIN_WINDOW + Duration::from_secs(1);
        assert!(allow_attempt(&mut map, ip, later));
    }

    #[test]
    fn pickup_out_of_range_rejected() {
        let mut w = world_with_loot();
        assert!(!handle_pickup(&mut w, 1, zn_sim::ids::LOOT_BASE));
        assert_eq!(w.loot.len(), 1, "лут не должен исчезнуть при отказе");
    }

    #[test]
    fn pickup_in_range_accepted() {
        let mut w = world_with_loot();
        w.get_player_mut(1).unwrap().position = Vec3::new(50.0, 0.8, 50.0);
        assert!(handle_pickup(&mut w, 1, zn_sim::ids::LOOT_BASE));
        assert!(w.loot.is_empty());
        assert!(w.get_player(1).unwrap().inventory.has_item(1, 1));
    }

    #[test]
    fn shot_from_far_origin_rejected() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        w.creatures.push(zn_sim::Creature {
            id: 1,
            position: Vec3::new(0.0, 0.8, 5.0),
            health: 50.0,
            max_health: 50.0,
            state: zn_sim::CreatureState::Patrol,
            target: None,
            damage: 5.0,
            kind: "test".into(),
        });
        // origin за сотни метров от игрока — выстрел не принимается
        assert!(handle_shot(&mut w, 1, [100.0, 100.0, 100.0], [0.0, 0.0, 1.0], 0, 100.0).is_none());
        assert_eq!(w.creatures[0].health, 50.0);
    }

    #[test]
    fn consume_rejects_non_item_ids() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.inventory.add_item(zn_sim::ids::ARTIFACT_BASE as i64, 1);
        w.add_player(p);
        let hp = w.get_player(1).unwrap().health;
        handle_consume(&mut w, 1, zn_sim::ids::ARTIFACT_BASE);
        assert_eq!(w.get_player(1).unwrap().health, hp);
        assert!(
            w.get_player(1)
                .unwrap()
                .inventory
                .has_item(zn_sim::ids::ARTIFACT_BASE as i64, 1)
        );
    }

    #[test]
    fn tick_stats_tracks_average_and_worst() {
        let mut stats = TickStats::default();
        stats.record(Duration::from_millis(2));
        stats.record(Duration::from_millis(10));
        assert!((stats.avg_ms() - 6.0).abs() < 1e-6);
        assert!((stats.max_ms() - 10.0).abs() < 1e-6);
        stats.reset();
        assert_eq!(stats.avg_ms(), 0.0, "после сброса окно пустое");
    }

    #[test]
    fn idle_players_respects_timeout() {
        let now = Instant::now();
        let mut seen: HashMap<u64, Instant> = HashMap::new();
        seen.insert(1, now);
        seen.insert(2, now - Duration::from_secs(31));
        seen.insert(3, now - Duration::from_secs(29));
        assert_eq!(
            idle_players(&seen, now, IDLE_TIMEOUT),
            vec![2],
            "только молчащий дольше таймаута игрок уходит из мира"
        );
    }

    #[test]
    fn loadout_requires_ownership_except_pistol() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        assert_eq!(
            handle_loadout(&mut w, 1, zn_sim::items::PISTOL),
            Ok((zn_sim::items::PISTOL, zn_sim::items::START_AMMO))
        );
        assert_eq!(
            handle_loadout(&mut w, 1, zn_sim::items::RIFLE),
            Err("weapon not in inventory"),
            "винтовка без владения не выдаётся"
        );
        w.get_player_mut(1)
            .unwrap()
            .inventory
            .add_item(zn_sim::items::RIFLE as i64, 1);
        assert_eq!(
            handle_loadout(&mut w, 1, zn_sim::items::RIFLE),
            Ok((zn_sim::items::RIFLE, zn_sim::items::START_AMMO))
        );
        assert!(
            handle_loadout(&mut w, 1, zn_sim::items::MEDKIT).is_err(),
            "расходник не может стать оружием"
        );
        assert!(handle_loadout(&mut w, 1, 999_999).is_err());
    }

    #[test]
    fn rewind_ticks_uses_rtt_half_and_clamps() {
        assert_eq!(rewind_ticks(100, 0, 100.0), 3, "RTT 100 → половина 50 мс");
        assert_eq!(
            rewind_ticks(100, 0, 10_000.0),
            12,
            "RTT/2 ограничен 200 мс, а не растёт без предела"
        );
        assert_eq!(rewind_ticks(100, 0, 0.0), 0, "без RTT отката нет");
        let now_ms = (100.0 * TICK_MS) as u64;
        assert_eq!(
            rewind_ticks(100, now_ms, 100.0),
            0,
            "метка времени в настоящем не даёт ложного отката"
        );
        assert_eq!(
            rewind_ticks(100, now_ms + 100, 100.0),
            0,
            "будущая метка зажимается к нулю"
        );
        assert_eq!(
            rewind_ticks(100, 0, 100.0),
            rewind_ticks(100, now_ms - 1000, 100.0),
            "старая метка не откатывает дальше RTT/2"
        );
    }

    #[test]
    fn snapshot_filters_distant_entities() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        w.loot.push(zn_sim::LootItem {
            id: 1_000_000,
            position: Vec3::new(1.0, 0.5, 1.0),
            item: zn_sim::Item::from_catalog(items::MEDKIT),
        });
        w.loot.push(zn_sim::LootItem {
            id: 1_000_001,
            position: Vec3::new(500.0, 0.5, 500.0),
            item: zn_sim::Item::from_catalog(items::MEDKIT),
        });

        let snap = snapshot_for(&w, 1);
        assert_eq!(snap.loot.len(), 1, "дальний лут не попадает в кадр");
        assert_eq!(snap.loot[0].id, 1_000_000);
    }

    #[test]
    fn extraction_ends_raid_and_banks_loot() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.inventory.add_item(items::MEDKIT as i64, 2);
        p.xp = 100;
        w.add_player(p);
        w.events.push(zn_sim::GameEvent::ExtractionSuccess {
            player_id: 1,
            loot: Vec::new(),
        });
        let mut users: HashMap<u64, i64> = HashMap::new();
        users.insert(1, 42);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut started: HashMap<u64, Instant> = HashMap::new();
        started.insert(1, Instant::now());
        let mut baseline: HashMap<u64, i64> = HashMap::new();
        baseline.insert(1, 0);

        let out = handle_events(&mut w, &users, &tx, &mut started, &mut baseline);

        assert!(
            w.get_player(1).is_none(),
            "эвакуация убирает игрока из мира"
        );
        assert!(out.iter().any(|(_, pkt)| matches!(
            pkt,
            ServerPacket::RaidResult { result }
                if result.reason == RaidEndReason::Extracted && result.xp_gained == 100
        )));
        assert!(out.iter().any(|(_, pkt)| matches!(
            pkt,
            ServerPacket::Stash { items } if items.len() == 1
        )));
        assert!(
            started.is_empty() && baseline.is_empty(),
            "база рейда чистится"
        );
    }

    #[test]
    fn death_ends_raid_and_reports_lost_items() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        w.events.push(zn_sim::GameEvent::PlayerDeath {
            player_id: 1,
            dropped_loot: vec![zn_sim::Item {
                id: items::MEDKIT,
                name: "Medkit".into(),
                value: 50,
                consumable: true,
            }],
            dropped_consumables: Vec::new(),
        });
        let mut users: HashMap<u64, i64> = HashMap::new();
        users.insert(1, 42);
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let mut started: HashMap<u64, Instant> = HashMap::new();
        started.insert(1, Instant::now());
        let mut baseline: HashMap<u64, i64> = HashMap::new();
        baseline.insert(1, 0);

        let out = handle_events(&mut w, &users, &tx, &mut started, &mut baseline);

        assert!(w.get_player(1).is_none(), "смерть тоже заканчивает рейд");
        assert!(out.iter().any(|(_, pkt)| matches!(
            pkt,
            ServerPacket::RaidResult { result }
                if result.reason == RaidEndReason::Died && result.items.len() == 1
        )));
    }

    #[tokio::test]
    async fn stash_moves_items_into_raid_inventory() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));

        stash_add(
            &pool,
            42,
            &[InventoryItem {
                item_id: zn_sim::items::SMG as i64,
                quantity: 2,
                slot: 0,
            }],
        )
        .await;
        assert_eq!(load_stash(&pool, 42).await[0].quantity, 2);

        assert!(take_from_stash(&pool, &mut w, 42, 1, zn_sim::items::SMG, 1).await);
        assert!(
            w.get_player(1)
                .unwrap()
                .inventory
                .has_item(zn_sim::items::SMG as i64, 1),
            "изъятое оружие обязано попасть в рейдовый инвентарь"
        );
        assert_eq!(load_stash(&pool, 42).await[0].quantity, 1);

        assert!(
            !take_from_stash(&pool, &mut w, 42, 1, zn_sim::items::SMG, 5).await,
            "нельзя взять больше, чем лежит в схроне"
        );
        assert!(
            !take_from_stash(&pool, &mut w, 42, 999, zn_sim::items::SMG, 1).await,
            "без игрока предмет не покидает схрон"
        );
        assert_eq!(load_stash(&pool, 42).await[0].quantity, 1);
    }
}
