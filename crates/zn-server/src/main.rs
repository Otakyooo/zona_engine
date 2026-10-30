use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use password_hash::{SaltString, rand_core::OsRng};
use sqlx::sqlite::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::{TcpListener, tcp::OwnedWriteHalf};
use tokio::sync::Mutex;
use tokio::time::interval;
use zn_net::{ClientPacket, ServerPacket, send_packet};
use zn_sim::{Inventory, InventoryItem, Player, PlayerProfile, SimWorld};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:zona.db?mode=rwc".into());
    let pool = SqlitePool::connect(&db_url).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;

    let listener = TcpListener::bind("0.0.0.0:7777").await?;
    log::info!("zn-server listening on 7777");

    let mut world = SimWorld::new();
    let mut writers: HashMap<u64, Arc<Mutex<OwnedWriteHalf>>> = HashMap::new();
    let mut tokens: HashMap<String, i64> = HashMap::new(); // token -> user_id
    let mut user_of_conn: HashMap<u64, i64> = HashMap::new();
    let mut next_id: u64 = 1;
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<(u64, ClientPacket)>(1024);

    let mut tick_timer = interval(Duration::from_millis(1000 / 60));
    let mut save_timer = interval(Duration::from_secs(60));
    let mut snapshot_counter: u32 = 0;

    loop {
        tokio::select! {
            Ok((stream, addr)) = listener.accept() => {
                log::info!("client connected: {}", addr);
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
            Some((id, pkt)) = cmd_rx.recv() => {
                match pkt {
                    ClientPacket::Hello { name } => {
                        if name == "__disconnect__" {
                            // save profile
                            if let Some(uid) = user_of_conn.remove(&id) {
                                save_profile(&pool, uid, world.get_player(uid as u64)).await;
                            }
                            world.remove_player(id);
                            writers.remove(&id);
                            log::info!("player {} disconnected", id);
                        }
                    }
                    ClientPacket::Login { username, password } => {
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
                        if let Some(&uid) = tokens.get(&token) {
                            user_of_conn.insert(id, uid);
                            // load profile
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
                    ClientPacket::Shot { .. } => {}
                }
            }
            _ = tick_timer.tick() => {
                world.tick(1.0 / 60.0);
                snapshot_counter += 1;
                if snapshot_counter >= 3 {
                    snapshot_counter = 0;
                    let snap = world.snapshot();
                    let pkt = ServerPacket::Snapshot(snap);
                    let mut dead = Vec::new();
                    for (pid, w) in &writers {
                        let mut g = w.lock().await;
                        if send_packet(&mut *g, &pkt).await.is_err() {
                            dead.push(*pid);
                        }
                    }
                    for pid in dead {
                        if let Some(uid) = user_of_conn.remove(&pid) {
                            save_profile(&pool, uid, world.get_player(pid)).await;
                        }
                        world.remove_player(pid);
                        writers.remove(&pid);
                    }
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

async fn handle_login(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> Result<(String, i64), String> {
    // find user
    let row: Option<(i64, String)> =
        sqlx::query_as("SELECT id, password_hash FROM users WHERE username = ?")
            .bind(username)
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;

    let user_id = if let Some((id, hash)) = row {
        // verify
        let parsed = PasswordHash::new(&hash).map_err(|e| e.to_string())?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| "invalid password".to_string())?;
        id
    } else {
        // create
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
        // create profile
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
    // upsert profile (xp/level not tracked in sim yet; keep existing)
    let _ =
        sqlx::query("INSERT INTO profiles (user_id) VALUES (?) ON CONFLICT(user_id) DO NOTHING")
            .bind(user_id)
            .execute(pool)
            .await;
}
