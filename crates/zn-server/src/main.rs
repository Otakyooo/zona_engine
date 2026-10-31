use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use clap::Parser;
use password_hash::{SaltString, rand_core::OsRng};
use sqlx::sqlite::SqlitePool;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
struct Cli {
    #[arg(long, default_value = "0.0.0.0")]
    bind: String,
    #[arg(long, default_value_t = 7777)]
    port: u16,
}
use glam::Vec3;
use tokio::net::{TcpListener, tcp::OwnedWriteHalf};
use tokio::sync::Mutex;
use tokio::time::interval;
use zn_net::{ClientPacket, ServerPacket, UdpEndpoint, send_packet};
use zn_sim::{Inventory, InventoryItem, Player, PlayerProfile, SimWorld};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let cli = Cli::parse();
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:zona.db?mode=rwc".into());
    let pool = SqlitePool::connect(&db_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let listener = TcpListener::bind(format!("{}:{}", cli.bind, cli.port)).await?;
    let udp = UdpEndpoint::bind(&format!("{}:{}", cli.bind, cli.port + 1)).await?;
    let udp = Arc::new(udp);
    log::info!("zn-server listening TCP {}, UDP {}", cli.port, cli.port + 1);

    let mut world = SimWorld::new();
    // M4: spawn sample entities
    world.anomalies.push(zn_sim::Anomaly {
        id: 1,
        position: Vec3::new(5.0, 0.0, 5.0),
        radius: 3.0,
        damage: 5.0,
        radiation: 10.0,
    });
    world.artifacts.push(zn_sim::Artifact {
        id: 1,
        position: Vec3::new(5.0, 0.5, 5.0),
        effect: "regen".into(),
        value: 10,
    });
    world.loot.push(zn_sim::LootItem {
        id: zn_sim::ids::LOOT_BASE,
        position: Vec3::new(2.0, 0.5, 2.0),
        item: zn_sim::Item {
            id: 1,
            name: "Medkit".into(),
            value: 50,
            consumable: true,
        },
    });
    world.creatures.push(zn_sim::Creature {
        id: 1,
        position: Vec3::new(10.0, 0.8, 10.0),
        health: 50.0,
        max_health: 50.0,
        state: zn_sim::CreatureState::Patrol,
        target: None,
        damage: 5.0,
    });
    world.extraction_zones.push(zn_sim::ExtractionZone {
        id: 1,
        position: Vec3::new(18.0, 0.0, 18.0),
        radius: 3.0,
    });
    let mut writers: HashMap<u64, Arc<Mutex<OwnedWriteHalf>>> = HashMap::new();
    let mut token_to_player: HashMap<String, u64> = HashMap::new();
    let mut player_to_token: HashMap<u64, String> = HashMap::new();
    let mut player_to_user: HashMap<u64, i64> = HashMap::new();
    let mut conn_to_player: HashMap<u64, u64> = HashMap::new();
    let mut conn_addrs: HashMap<u64, SocketAddr> = HashMap::new();
    let mut client_addrs: HashMap<u64, SocketAddr> = HashMap::new();
    let mut addr_to_client: HashMap<SocketAddr, u64> = HashMap::new();
    let mut next_id: u64 = 1;
    let mut login_attempts: HashMap<IpAddr, Attempts> = HashMap::new();
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<(u64, ClientPacket)>(1024);

    let mut tick_timer = interval(Duration::from_millis(1000 / 60));
    let mut save_timer = interval(Duration::from_secs(60));
    let mut session_sweep = interval(Duration::from_secs(600));
    let mut snapshot_counter: u32 = 0;

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
                        match pkt {
                            ClientPacket::Connect { token } => {
                                if let Some(uid) = validate_session(&pool, &token).await {
                                    // resolve or create player id
                                    let pid = *token_to_player.entry(token.clone()).or_insert_with(|| {
                                        let p = next_id;
                                        next_id += 1;
                                        p
                                    });
                                    player_to_token.insert(pid, token.clone());
                                    let (player, profile, inventory) = spawn_player(&pool, uid, pid).await;
                                    if let std::collections::hash_map::Entry::Vacant(e) = player_to_user.entry(pid) {
                                        e.insert(uid);
                                        world.add_player(player);
                                    }
                                    if let std::collections::hash_map::Entry::Vacant(e) = addr_to_client.entry(addr) {
                                        e.insert(pid);
                                        client_addrs.insert(pid, addr);
                                    }
                                    // send Welcome + Profile via UDP
                                    let welcome = ServerPacket::Welcome { id: pid };
                                    if let Ok(d) = zn_net::encode(&welcome) { let _ = udp.socket.send_to(&d, addr).await; }
                                    let prof = ServerPacket::Profile { profile, inventory };
                                    if let Ok(d) = zn_net::encode(&prof) { let _ = udp.socket.send_to(&d, addr).await; }
                                }
                            }
                            ClientPacket::Input(cmd) => {
                                if let Some(&pid) = addr_to_client.get(&addr) {
                                    world.apply_input(pid, &cmd);
                                }
                            }
                            ClientPacket::Shot { origin, dir, .. } => {
                                if let Some(&pid) = addr_to_client.get(&addr) {
                                    handle_shot(&mut world, pid, origin, dir);
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
                                if let Some(&uid) = player_to_user.get(&pid) {
                                    save_profile(&pool, uid, world.get_player(pid)).await;
                                    save_inventory(&pool, uid, &world.get_player(pid).map(|p| p.inventory.clone()).unwrap_or_default()).await;
                                }
                                world.remove_player(pid);
                                player_to_user.remove(&pid);
                                token_to_player.retain(|_, v| *v != pid);
                                if let Some(token) = player_to_token.remove(&pid) {
                                    delete_session(&pool, &token).await;
                                }
                                if let Some(a) = client_addrs.remove(&pid) {
                                    addr_to_client.remove(&a);
                                }
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
                            let (player, profile, inventory) = spawn_player(&pool, uid, pid).await;
                            if let std::collections::hash_map::Entry::Vacant(e) = player_to_user.entry(pid) {
                                e.insert(uid);
                                world.add_player(player);
                            }
                            conn_to_player.insert(conn_id, pid);
                            if let Some(w) = writers.get(&conn_id) {
                                let mut g = w.lock().await;
                                let _ = send_packet(&mut *g, &ServerPacket::Profile { profile, inventory }).await;
                                let _ = send_packet(&mut *g, &ServerPacket::Welcome { id: pid }).await;
                            }
                        } else {
                            if let Some(w) = writers.get(&conn_id) {
                                let mut g = w.lock().await;
                                let _ = send_packet(&mut *g, &ServerPacket::LoginResult { ok: false, token: None, error: Some("invalid token".into()) }).await;
                            }
                        }
                    }
                    ClientPacket::Input(cmd) => {
                        let pid = conn_to_player.get(&conn_id).copied().unwrap_or(conn_id);
                        world.apply_input(pid, &cmd);
                    }
                    ClientPacket::Shot { origin, dir, .. } => {
                        let pid = conn_to_player.get(&conn_id).copied().unwrap_or(conn_id);
                        handle_shot(&mut world, pid, origin, dir);
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
                }
            }
            _ = tick_timer.tick() => {
                world.tick(1.0 / 60.0);
                // handle events
                for (pid, inv) in handle_events(&mut world, &pool, &player_to_user).await {
                    if let Some(addr) = client_addrs.get(&pid).copied()
                        && let Ok(data) = zn_net::encode(&ServerPacket::Inventory(inv))
                    {
                        let _ = udp.socket.send_to(&data, addr).await;
                    }
                }
                snapshot_counter += 1;
                if snapshot_counter >= 3 {
                    snapshot_counter = 0;
                    for (pid, addr) in &client_addrs {
                        let center = world.get_player(*pid).map(|p| p.position).unwrap_or_default();
                        let nearby: Vec<_> = world.players.iter().filter(|p| (p.position - center).length() <= 100.0).map(|p| zn_sim::PlayerState {
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
                        }).collect();
                        let snap = zn_sim::Snapshot {
                            tick: world.tick,
                            players: nearby,
                            anomalies: world.anomalies.clone(),
                            artifacts: world.artifacts.clone(),
                            loot: world.loot.clone(),
                            creatures: world.creatures.clone(),
                            extraction_zones: world.extraction_zones.clone(),
                        };
                        let pkt = ServerPacket::Snapshot(snap);
                        if let Ok(data) = bincode::serialize(&pkt) {
                            let _ = udp.socket.send_to(&data, *addr).await;
                        }
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
                        save_profile(&pool, *uid, Some(p)).await;
                        save_inventory(&pool, *uid, &p.inventory).await;
                    }
                }
            }
        }
    }
}

/// Радиус подбора: клиент берёт с 2 м, запас на лаг.
const PICKUP_RANGE: f32 = 3.0;
/// Допустимое расхождение между присланным origin и глазом игрока.
const SHOT_ORIGIN_TOLERANCE: f32 = 2.0;
const EYE_HEIGHT: f32 = 1.6;

/// Отправляет игроку его текущий инвентарь по UDP.
async fn send_inventory(
    socket: &tokio::net::UdpSocket,
    world: &SimWorld,
    pid: u64,
    addr: SocketAddr,
) {
    if let Some(p) = world.get_player(pid)
        && let Ok(data) = zn_net::encode(&ServerPacket::Inventory(p.inventory.clone()))
    {
        let _ = socket.send_to(&data, addr).await;
    }
}

fn handle_shot(world: &mut SimWorld, pid: u64, origin: [f32; 3], dir: [f32; 3]) -> bool {
    let eye = match world.get_player(pid) {
        Some(p) => p.position + Vec3::Y * EYE_HEIGHT,
        None => return false,
    };
    let o = Vec3::from(origin);
    let raw_dir = Vec3::from(dir);
    // Клиент не должен стрелять из чужой точки и не должен присылать нулевой вектор.
    if (o - eye).length() > SHOT_ORIGIN_TOLERANCE || raw_dir.length() < 0.1 {
        return false;
    }
    let d = raw_dir.normalize();

    let mut best: Option<(usize, f32)> = None;
    for (i, c) in world.creatures.iter().enumerate() {
        let to_c = c.position - o;
        let t = to_c.dot(d);
        if t < 0.0 {
            continue;
        }
        let closest = o + d * t;
        let dist = (closest - c.position).length();
        if dist <= 1.0 && (best.is_none() || t < best.unwrap().1) {
            best = Some((i, t));
        }
    }
    if let Some((i, _)) = best {
        let cid = world.creatures[i].id;
        world.creatures[i].health -= 20.0;
        if world.creatures[i].health <= 0.0 {
            world.creatures.remove(i);
            if let Some(p) = world.get_player_mut(pid) {
                p.add_xp(50);
            }
            log::info!("creature {} killed by player {}", cid, pid);
        }
    }
    true
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

fn handle_consume(world: &mut SimWorld, pid: u64, item_id: u64) {
    if item_id == 0 || item_id > zn_sim::ids::ITEM_MAX {
        return;
    }
    if let Some(p) = world.get_player_mut(pid) {
        let iid = item_id as i64;
        if p.inventory.has_item(iid, 1) {
            p.inventory.remove_item(iid, 1);
            match item_id {
                1 => p.health = (p.health + 30.0).min(p.max_health),
                2 => p.health = (p.health + 15.0).min(p.max_health),
                _ => {}
            }
            log::info!("player {} consumed item {}", pid, item_id);
        }
    }
}

async fn handle_events(
    world: &mut SimWorld,
    pool: &SqlitePool,
    player_to_user: &HashMap<u64, i64>,
) -> Vec<(u64, Inventory)> {
    let mut changed: Vec<(u64, Inventory)> = Vec::new();
    let events: Vec<zn_sim::GameEvent> = std::mem::take(&mut world.events);
    for ev in events {
        match ev {
            zn_sim::GameEvent::ExtractionSuccess { player_id, .. } => {
                // XP начисляет симуляция в `zn_sim::tick` — второй раз здесь нельзя.
                let (uid, inv) = match (player_to_user.get(&player_id), world.get_player(player_id))
                {
                    (Some(uid), Some(p)) => (*uid, p.inventory.clone()),
                    _ => continue,
                };
                // Сначала в БД, потом очистка: иначе вынесенный лут просто исчезает.
                save_inventory(pool, uid, &inv).await;
                if let Some(p) = world.get_player_mut(player_id) {
                    p.inventory = Inventory::default();
                }
                changed.push((player_id, Inventory::default()));
                log::info!(
                    "player {} extracted, {} stacks saved",
                    player_id,
                    inv.items.len()
                );
            }
            zn_sim::GameEvent::PlayerDeath { player_id, .. } => {
                // Лут выложен и респаун выполнен в симуляции: здесь только запись в БД.
                if let Some(uid) = player_to_user.get(&player_id)
                    && let Some(p) = world.get_player(player_id)
                {
                    save_profile(pool, *uid, Some(p)).await;
                    save_inventory(pool, *uid, &p.inventory).await;
                    changed.push((player_id, p.inventory.clone()));
                }
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

async fn load_profile(pool: &SqlitePool, user_id: i64) -> Option<PlayerProfile> {
    let row: Option<(i32, i64, i64, f64, f64, f64)> = sqlx::query_as(
        "SELECT level, xp, currency, pos_x, pos_y, pos_z FROM profiles WHERE user_id = ?",
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
    if let Some((level, xp, currency, px, py, pz)) = row {
        Some(PlayerProfile {
            user_id,
            username,
            level,
            xp,
            currency,
            position: Vec3::new(px as f32, py as f32, pz as f32),
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
async fn spawn_player(pool: &SqlitePool, uid: i64, pid: u64) -> (Player, PlayerProfile, Inventory) {
    let profile = load_profile(pool, uid)
        .await
        .unwrap_or_else(|| PlayerProfile::new(uid, "player"));
    let inventory = load_inventory(pool, uid).await;
    let mut player = Player::new(pid, profile.username.clone());
    player.apply_profile(&profile);
    player.inventory = inventory.clone();
    (player, profile, inventory)
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
            "INSERT INTO profiles (user_id, level, xp, currency, pos_x, pos_y, pos_z) VALUES (?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT(user_id) DO UPDATE SET level=excluded.level, xp=excluded.xp, currency=excluded.currency, pos_x=excluded.pos_x, pos_y=excluded.pos_y, pos_z=excluded.pos_z"
        )
        .bind(user_id)
        .bind(p.level)
        .bind(p.xp)
        .bind(p.currency)
        .bind(p.position.x)
        .bind(p.position.y)
        .bind(p.position.z)
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

async fn save_inventory(pool: &SqlitePool, user_id: i64, inv: &Inventory) {
    let _ = sqlx::query("DELETE FROM inventory_items WHERE user_id = ?")
        .bind(user_id)
        .execute(pool)
        .await;
    for it in &inv.items {
        let _ = sqlx::query(
            "INSERT INTO inventory_items (user_id, item_id, quantity, slot) VALUES (?, ?, ?, ?)",
        )
        .bind(user_id)
        .bind(it.item_id)
        .bind(it.quantity)
        .bind(it.slot)
        .execute(pool)
        .await;
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
        });
        // origin за сотни метров от игрока — выстрел не принимается
        assert!(!handle_shot(
            &mut w,
            1,
            [100.0, 100.0, 100.0],
            [0.0, 0.0, 1.0]
        ));
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
}
