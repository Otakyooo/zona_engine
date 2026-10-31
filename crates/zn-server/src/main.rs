use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use clap::Parser;
use password_hash::{SaltString, rand_core::OsRng};
use sqlx::sqlite::SqlitePool;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

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
    log::info!("zn-server listening TCP 7777, UDP 7778");

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
        id: 1,
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
    let mut tokens: HashMap<String, i64> = HashMap::new();
    let mut user_of_conn: HashMap<u64, i64> = HashMap::new();
    let mut client_addrs: HashMap<u64, SocketAddr> = HashMap::new();
    let mut addr_to_client: HashMap<SocketAddr, u64> = HashMap::new();
    let mut next_id: u64 = 1;
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<(u64, ClientPacket)>(1024);

    let mut tick_timer = interval(Duration::from_millis(1000 / 60));
    let mut save_timer = interval(Duration::from_secs(60));
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

                let tx = cmd_tx.clone();
                tokio::spawn(async move {
                    use tokio::io::AsyncReadExt;
                    let mut rh = read_half;
                    loop {
                        let mut len_buf = [0u8; 4];
                        if rh.read_exact(&mut len_buf).await.is_err() { break; }
                        let len = u32::from_le_bytes(len_buf) as usize;
                        let mut buf = vec![0u8; len];
                        if rh.read_exact(&mut buf).await.is_err() { break; }
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
                            if let Some(&uid) = tokens.get(&token)
                                && let std::collections::hash_map::Entry::Vacant(e) = addr_to_client.entry(addr)
                            {
                                let id = next_id;
                                next_id += 1;
                                e.insert(id);
                                client_addrs.insert(id, addr);
                                user_of_conn.insert(id, uid);
                                world.add_player(Player::new(id, format!("player_{}", id)));
                            }
                            }
                            ClientPacket::Input(cmd) => {
                                if let Some(&id) = addr_to_client.get(&addr) {
                                    world.apply_input(id, &cmd);
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }
            Some((id, pkt)) = cmd_rx.recv() => {
                match pkt {
                    ClientPacket::Hello { name } => {
                        if name == "__disconnect__" {
                            if let Some(uid) = user_of_conn.remove(&id) {
                                save_profile(&pool, uid, world.get_player(id)).await;
                            }
                            world.remove_player(id);
                            writers.remove(&id);
                            // remove udp mapping
                            if let Some(addr) = client_addrs.remove(&id) {
                                addr_to_client.remove(&addr);
                            }
                            log::info!("player {} disconnected", id);
                        }
                    }
                    ClientPacket::Login { username, password } => {
                        // find writer by id (tcp)
                        let result = handle_login(&pool, &username, &password).await;
                        match result {
                            Ok((token, uid)) => {
                                tokens.insert(token.clone(), uid);
                                if let Some(w) = writers.get(&id) {
                                    let mut g = w.lock().await;
                                    let _ = send_packet(&mut *g, &ServerPacket::LoginResult { ok: true, token: Some(token), error: None }).await;
                                }
                            }
                            Err(e) => {
                                if let Some(w) = writers.get(&id) {
                                    let mut g = w.lock().await;
                                    let _ = send_packet(&mut *g, &ServerPacket::LoginResult { ok: false, token: None, error: Some(e) }).await;
                                }
                            }
                        }
                    }
                    ClientPacket::Connect { token } => {
                        // TCP connect (fallback): validate token, create player, welcome via TCP
                        if let Some(&uid) = tokens.get(&token) {
                            user_of_conn.insert(id, uid);
                            let profile = load_profile(&pool, uid).await.unwrap_or_else(|| PlayerProfile::new(uid, "player"));
                            let inventory = load_inventory(&pool, uid).await;
                            world.add_player(Player::new(id, profile.username.clone()));
                            if let Some(w) = writers.get(&id) {
                                let mut g = w.lock().await;
                                let _ = send_packet(&mut *g, &ServerPacket::Profile { profile, inventory }).await;
                                let _ = send_packet(&mut *g, &ServerPacket::Welcome { id }).await;
                            }
                        } else {
                            if let Some(w) = writers.get(&id) {
                                let mut g = w.lock().await;
                                let _ = send_packet(&mut *g, &ServerPacket::LoginResult { ok: false, token: None, error: Some("invalid token".into()) }).await;
                            }
                        }
                    }
                    ClientPacket::Input(cmd) => {
                        world.apply_input(id, &cmd);
                    }
                    ClientPacket::Shot { origin, dir, .. } => {
                        // simple hit detection against creatures
                        let o = glam::Vec3::from(origin);
                        let d = glam::Vec3::from(dir).normalize_or_zero();
                        // find closest creature hit
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
                                if let Some(p) = world.get_player_mut(id) {
                                    p.add_xp(50);
                                }
                                log::info!("creature {} killed by player {}", cid, id);
                            }
                        }
                    }
                    ClientPacket::Pickup { entity_id } => {
                        // remove loot/artifact if exists, add to player inventory (sim not tracked yet)
                        if let Some(idx) = world.loot.iter().position(|l| l.id == entity_id) {
                            world.loot.remove(idx);
                            log::info!("player {} picked up loot {}", id, entity_id);
                        }
                        if let Some(idx) = world.artifacts.iter().position(|a| a.id == entity_id) {
                            world.artifacts.remove(idx);
                            if let Some(p) = world.get_player_mut(id) {
                                p.health = (p.health + 20.0).min(p.max_health);
                            }
                            log::info!("player {} picked up artifact {}", id, entity_id);
                        }
                    }
                    ClientPacket::Consume { item_id } => {
                        if let Some(p) = world.get_player_mut(id) {
                            // medkit: heal 30
                            p.health = (p.health + 30.0).min(p.max_health);
                            log::info!("player {} consumed item {}", id, item_id);
                        }
                    }
                }
            }
            _ = tick_timer.tick() => {
                world.tick(1.0 / 60.0);
                snapshot_counter += 1;
                if snapshot_counter >= 3 {
                    snapshot_counter = 0;
                    // send per-client filtered snapshot via UDP
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
                    // also TCP clients (fallback) not handled here
                }
            }
            _ = save_timer.tick() => {
                for (pid, uid) in &user_of_conn {
                    save_profile(&pool, *uid, world.get_player(*pid)).await;
                }
            }
        }
    }
}

// handle_login, load_profile, etc. (same as before)
async fn handle_login(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> Result<(String, i64), String> {
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
        id
    };

    let token = uuid::Uuid::new_v4().to_string();
    Ok((token, user_id))
}

async fn load_profile(pool: &SqlitePool, user_id: i64) -> Option<PlayerProfile> {
    let row: Option<(i32, i64, i64)> =
        sqlx::query_as("SELECT level, xp, currency FROM profiles WHERE user_id = ?")
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
    if let Some((level, xp, currency)) = row {
        Some(PlayerProfile {
            user_id,
            username,
            level,
            xp,
            currency,
        })
    } else {
        Some(PlayerProfile::new(user_id, username))
    }
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

async fn save_profile(pool: &SqlitePool, user_id: i64, _player: Option<&Player>) {
    let _ =
        sqlx::query("INSERT INTO profiles (user_id) VALUES (?) ON CONFLICT(user_id) DO NOTHING")
            .bind(user_id)
            .execute(pool)
            .await;
}
