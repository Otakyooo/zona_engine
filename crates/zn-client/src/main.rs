use clap::Parser;
use glam::{Mat4, Vec3};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::KeyCode;
use winit::window::{CursorGrabMode, Window, WindowId};
use zn_audio::Sfx;
use zn_input::InputState;
use zn_net::{ClientPacket, PROTOCOL_VERSION, ServerPacket, encode};
use zn_render::{
    Camera, Mesh, RenderItem, Renderer, Texture, Vertex,
    geom::{self, make_box},
};
use zn_scene::Transform;
use zn_sim::{
    Aabb, Bounds, EYE_HEIGHT, GROUND_Y, InputCommand, Snapshot, interpolate, items, lerp_angle,
};

mod hud;
mod perf;
mod prediction;
mod settings;
mod ui;

use settings::Settings;

/// Складывает дельту схрона в локальный список.
fn merge_stack(stash: &mut Vec<zn_sim::InventoryItem>, incoming: &zn_sim::InventoryItem) {
    if let Some(it) = stash.iter_mut().find(|i| i.item_id == incoming.item_id) {
        it.quantity += incoming.quantity;
    } else {
        stash.push(incoming.clone());
    }
}

use prediction::{Prediction, TICK_DT};

#[derive(Parser, Debug)]
struct Cli {
    #[arg(long)]
    scene: Option<String>,
    #[arg(long)]
    token: Option<String>,
    #[arg(long)]
    username: Option<String>,
    #[arg(long)]
    password: Option<String>,
    #[arg(long)]
    telemetry: bool,
    #[arg(long)]
    register: bool,
    #[arg(long, default_value = "127.0.0.1")]
    server: String,
    #[arg(long, default_value_t = 7777)]
    port: u16,
}

/// Сглаживание чужих сущностей между снапшотами.
///
/// Сервер присылает позиции 20 раз в секунду, а кадров больше: без
/// интерполяции движение выглядит рывками раз в три тика. Рисуем с задержкой
/// на один интервал — `alpha` идёт от предыдущего снапшота (0) к текущему (1).
#[derive(Default)]
struct RemoteInterp {
    prev_players: std::collections::HashMap<u64, (Vec3, f32)>,
    players: std::collections::HashMap<u64, (Vec3, f32)>,
    prev_creatures: std::collections::HashMap<u64, Vec3>,
    creatures: std::collections::HashMap<u64, Vec3>,
    /// Когда пришёл последний снапшот.
    at: Option<Instant>,
    /// Измеренный интервал между снапшотами, секунды.
    interval: f32,
}

impl RemoteInterp {
    /// Начальное значение до первого измерения: 20 Гц — текущая частота сервера.
    const DEFAULT_INTERVAL: f32 = 1.0 / 20.0;

    fn push(
        &mut self,
        players: &[zn_sim::PlayerState],
        creatures: &[zn_sim::Creature],
        now: Instant,
    ) {
        self.prev_players = std::mem::take(&mut self.players);
        self.prev_creatures = std::mem::take(&mut self.creatures);
        for p in players {
            self.players.insert(p.id, (p.position, p.yaw));
        }
        for c in creatures {
            self.creatures.insert(c.id, c.position);
        }
        match self.at {
            Some(at) => {
                let dt = now.saturating_duration_since(at).as_secs_f32();
                if dt > 0.0 {
                    // Сглаживание интервала: одиночный лаг сети не должен
                    // растягивать сглаживание на полсекунды.
                    self.interval = (self.interval * 0.8 + dt * 0.2).clamp(0.02, 0.2);
                }
            }
            None => self.interval = Self::DEFAULT_INTERVAL,
        }
        self.at = Some(now);
    }

    fn alpha(&self, now: Instant) -> f32 {
        match self.at {
            Some(at) if self.interval > 0.0 => {
                (now.saturating_duration_since(at).as_secs_f32() / self.interval).clamp(0.0, 1.0)
            }
            _ => 1.0,
        }
    }

    fn player(&self, id: u64, alpha: f32) -> Option<(Vec3, f32)> {
        let (pos, yaw) = *self.players.get(&id)?;
        let (prev_pos, prev_yaw) = self.prev_players.get(&id).copied().unwrap_or((pos, yaw));
        Some((
            interpolate(prev_pos, pos, alpha, zn_sim::INTERP_MAX_JUMP),
            lerp_angle(prev_yaw, yaw, alpha),
        ))
    }

    fn creature(&self, id: u64, alpha: f32) -> Option<Vec3> {
        let cur = *self.creatures.get(&id)?;
        let prev = self.prev_creatures.get(&id).copied().unwrap_or(cur);
        Some(interpolate(prev, cur, alpha, zn_sim::INTERP_MAX_JUMP))
    }
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    input: InputState,
    camera: Camera,
    scene_items: Vec<(Mesh, Option<String>)>,
    /// Меш на сущность строится один раз; в кадре двигается только модель.
    entity_meshes: std::collections::HashMap<u64, (Mesh, &'static str)>,
    /// Грубый уровень детализации: один бокс вместо силуэта.
    player_lod: Option<Mesh>,
    creature_lod: Option<Mesh>,
    textures: std::collections::HashMap<String, Texture>,
    egui_ctx: egui::Context,
    hud: hud::HudState,
    start: Instant,
    inventory: zn_sim::Inventory,
    /// Схрон: полное содержимое из `StashContents`, дельты из `Stash`.
    stash: Vec<zn_sim::InventoryItem>,
    /// Итог последнего рейда; показывается на экране `Result`.
    raid_result: Option<zn_net::RaidResult>,
    extract_elapsed: f32,
    fps: f32,
    perf: perf::PerfStats,
    net_dead: bool,
    audio: zn_audio::AudioManager,
    step_timer: f32,
    last_health: f32,
    last_time: Option<Instant>,
    yaw: f32,
    pitch: f32,
    cursor_captured: bool,
    scene_path: Option<String>,
    /// Предсказание движения и неподтверждённые команды.
    prediction: Prediction,
    /// Границы и стены карты из `Welcome` — те же, что у сервера.
    server_bounds: Bounds,
    obstacles: Vec<Aabb>,
    input_accum: f32,
    /// Оружие и патроны: сервер присылает их в профиле и снапшоте.
    weapon_id: u64,
    ammo: u32,
    weapon_cooldown: f32,
    rtt_ms: f32,
    ping_at: Option<(u64, Instant)>,
    protocol_ok: bool,
    /// Экран — состояние, а не флаг: от него зависит и ввод, и отрисовка.
    screen: ui::Screen,
    /// Куда вернуться из настроек: меню или пауза.
    settings_return: ui::Screen,
    settings: Settings,
    inventory_ui: ui::InventoryUiState,
    /// Переводит события winit в события egui. Без него кнопки не нажимаются.
    egui_state: Option<egui_winit::State>,
    interp: RemoteInterp,

    // networking
    net_out: Sender<ClientPacket>,
    net_in: Receiver<ServerPacket>,
    player_id: Option<u64>,
    latest_snapshot: Option<Snapshot>,
    seq: u32,
    /// Тик последнего применённого кадра: база для дельт и ack.
    snapshot_tick: u64,
    /// Когда применён последний кадр: по нему клиентские часы переводятся
    /// в серверную шкалу для `Shot.client_time`.
    snapshot_at: Option<Instant>,
    local_pos: Vec3,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let attrs = Window::default_attributes().with_title("ZONA - client");
            let window = Arc::new(event_loop.create_window(attrs).expect("window"));
            let mut renderer = pollster::block_on(Renderer::new(window.clone())).expect("renderer");
            if !renderer.set_vsync(self.settings.vsync) {
                log::warn!("V-Sync недоступен на этой поверхности");
            }

            // meshes
            let mut meshes: Vec<(Mesh, Option<String>)> = Vec::new();

            // Ресурсы лежат рядом с exe (так их ставит лаунчер). Раньше пути были
            // относительными, и всё зависело от рабочего каталога лаунчера.
            let root = asset_root();
            let scene_opt = {
                let requested = self
                    .scene_path
                    .as_ref()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| root.join("maps/arena.ron"));
                let path = if requested.is_absolute() || requested.exists() {
                    requested
                } else {
                    root.join(&requested)
                };
                zn_content::ZoneMap::load(&path).ok()
            };

            if let Some(scene) = scene_opt {
                log::info!(
                    "карта загружена: {} узлов, {} существ, {} лута",
                    scene.nodes.len(),
                    scene.creature_spawns.len(),
                    scene.loot.len()
                );
                for node in &scene.nodes {
                    let color = node.color;
                    let (vs, is) = if let Some(mesh_path) = &node.mesh {
                        if mesh_path.ends_with(".gltf") || mesh_path.ends_with(".glb") {
                            if let Ok(lm) = zn_assets::load_gltf(std::path::Path::new(mesh_path)) {
                                let model = node.transform.matrix();
                                // Нормали трансформируются inverse-transpose:
                                // при неравномерном масштабе обычная матрица
                                // уводит их от граней и ломает освещение.
                                let normal_matrix =
                                    glam::Mat3::from_mat4(model).inverse().transpose();
                                let mut vs = Vec::new();
                                for (i, p) in lm.positions.iter().enumerate() {
                                    let v = glam::Vec3::from(*p);
                                    let w = model * v.extend(1.0);
                                    let n = lm
                                        .normals
                                        .as_ref()
                                        .and_then(|ns| ns.get(i).copied())
                                        .unwrap_or([0.0, 1.0, 0.0]);
                                    let wn =
                                        (normal_matrix * glam::Vec3::from(n)).normalize_or_zero();
                                    let c = lm
                                        .colors
                                        .as_ref()
                                        .and_then(|cs| cs.get(i).copied())
                                        .unwrap_or(color);
                                    let uv = lm
                                        .uvs
                                        .as_ref()
                                        .and_then(|us| us.get(i).copied())
                                        .unwrap_or([0.0, 0.0]);
                                    vs.push(Vertex {
                                        position: [w.x, w.y, w.z],
                                        normal: [wn.x, wn.y, wn.z],
                                        color: c,
                                        uv,
                                    });
                                }
                                let is = if lm.indices.is_empty() {
                                    (0..vs.len() as u32).collect()
                                } else {
                                    lm.indices.clone()
                                };
                                (vs, is)
                            } else {
                                make_box(&node.transform, color, node.tile_meters())
                            }
                        } else {
                            make_box(&node.transform, color, node.tile_meters())
                        }
                    } else {
                        make_box(&node.transform, color, node.tile_meters())
                    };
                    meshes.push((renderer.create_mesh(&vs, &is), node.texture.clone()));
                }
            } else {
                // карта не загрузилась — простейшая заглушка, чтобы было на чём стоять
                log::warn!("scene not loaded, using fallback geometry");
                let floor = make_box(
                    &Transform {
                        position: Vec3::new(0.0, 0.0, 0.0),
                        scale: Vec3::new(40.0, 0.2, 40.0),
                        ..Default::default()
                    },
                    [0.3, 0.35, 0.3],
                    2.0,
                );
                meshes.push((
                    renderer.create_mesh(&floor.0, &floor.1),
                    Some("concrete".to_string()),
                ));
                for i in -2..=2 {
                    for j in -2..=2 {
                        let x = i as f32 * 4.0;
                        let z = j as f32 * 4.0;
                        let cube = make_box(
                            &Transform {
                                position: Vec3::new(x, 0.5, z),
                                ..Default::default()
                            },
                            [0.8, 0.4, 0.2],
                            1.0,
                        );
                        meshes.push((
                            renderer.create_mesh(&cube.0, &cube.1),
                            Some("metal".to_string()),
                        ));
                    }
                }
            }

            let camera = Camera {
                eye: Vec3::new(0.0, 2.0, 5.0),
                target: Vec3::new(0.0, 1.0, 0.0),
                up: Vec3::Y,
                fov_y: self.settings.fov_radians(),
                aspect: window.inner_size().width as f32 / window.inner_size().height.max(1) as f32,
                near: 0.1,
                far: self.settings.render_distance,
            };

            let mut textures = std::collections::HashMap::new();
            for name in ["concrete", "metal", "rust", "dirt", "grass", "wood"] {
                let path = root.join("assets/textures").join(format!("{name}.png"));
                match zn_assets::load_texture(&path) {
                    Ok(lt) => {
                        textures.insert(
                            name.to_string(),
                            renderer.create_texture_mipped(&lt.mip_levels, lt.width, lt.height),
                        );
                    }
                    Err(e) => log::warn!("текстура {} не загружена: {e}", path.display()),
                }
            }

            // Грубый LOD: меш строится один раз, размер приходит матрицей модели.
            let (pv, pi) = geom::unit_box([0.2, 0.8, 0.2], 1.0);
            self.player_lod = Some(renderer.create_mesh(&pv, &pi));
            let (cv, ci) = geom::unit_box([0.8, 0.2, 0.2], 1.0);
            self.creature_lod = Some(renderer.create_mesh(&cv, &ci));

            // Стартуем в меню: курсор свободен, игровой ввод не захвачен.
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
            self.egui_state = Some(egui_winit::State::new(
                self.egui_ctx.clone(),
                egui::ViewportId::ROOT,
                window.as_ref(),
                Some(window.scale_factor() as f32),
                None,
                None,
            ));

            self.window = Some(window);
            self.renderer = Some(renderer);
            self.input = InputState::new();
            self.input.cursor_captured = false;
            self.camera = camera;
            self.scene_items = meshes;
            self.textures = textures;
            self.last_time = Some(Instant::now());
            self.cursor_captured = false;
            self.local_pos = Vec3::new(0.0, 0.8, 0.0);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // egui получает событие первым: кнопки меню должны нажиматься, а не
        // превращаться в выстрел.
        if let Some(state) = self.egui_state.as_mut()
            && let Some(window) = self.window.as_ref()
        {
            let _ = state.on_window_event(window.as_ref(), &event);
        }
        // В меню и на экране смерти мышь не стреляет.
        if self.screen != ui::Screen::Playing && matches!(&event, WindowEvent::MouseInput { .. }) {
            return;
        }
        match &event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let Some(r) = self.renderer.as_mut() {
                    r.resize(size.width, size.height);
                }
                if size.height > 0 {
                    self.camera.aspect = size.width as f32 / size.height as f32;
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if !self.cursor_captured
                    && *button == winit::event::MouseButton::Left
                    && *state == winit::event::ElementState::Pressed
                {
                    self.capture_cursor();
                }
                if *state == winit::event::ElementState::Pressed
                    && *button == winit::event::MouseButton::Left
                    && self.cursor_captured
                    && let Some(weapon) = items::weapon(self.weapon_id)
                    // Локальный кулдаун только для отзывчивости: сервер
                    // проверяет темп сам и отклонит лишний выстрел.
                    && self.weapon_cooldown <= 0.0
                    && self.ammo >= weapon.ammo_per_shot
                {
                    let origin = self.local_pos + Vec3::Y * EYE_HEIGHT;
                    let dir = Vec3::new(
                        self.yaw.cos() * self.pitch.cos(),
                        self.pitch.sin(),
                        self.yaw.sin() * self.pitch.cos(),
                    );
                    let _ = self.net_out.send(ClientPacket::Shot {
                        seq: self.seq,
                        origin: [origin.x, origin.y, origin.z],
                        dir: [dir.x, dir.y, dir.z],
                        // Серверная шкала: тик последнего кадра плюс время,
                        // прошедшее с его получения. Ноль здесь означал бы,
                        // что сервер не знает, когда стреляли.
                        client_time: self.server_time_ms(),
                    });
                    self.seq += 1;
                    self.ammo = self.ammo.saturating_sub(weapon.ammo_per_shot);
                    self.weapon_cooldown = weapon.cooldown_secs;
                    self.audio.play(Sfx::Shot, 1.0);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.input.handle_key_event(event);
                use winit::keyboard::PhysicalKey;
                if event.state == winit::event::ElementState::Pressed
                    && let PhysicalKey::Code(code) = event.physical_key
                {
                    match self.screen {
                        ui::Screen::Playing => match code {
                            KeyCode::Escape => self.pause(),
                            KeyCode::Tab => self.open_inventory(),
                            KeyCode::KeyE => self.try_pickup(),
                            KeyCode::Digit1 => {
                                let _ = self.net_out.send(ClientPacket::Consume { item_id: 1 });
                            }
                            KeyCode::Digit2 => {
                                let _ = self.net_out.send(ClientPacket::Consume { item_id: 2 });
                            }
                            _ => {}
                        },
                        ui::Screen::Inventory if matches!(code, KeyCode::Escape | KeyCode::Tab) => {
                            self.close_inventory();
                        }
                        ui::Screen::Paused if code == KeyCode::Escape => self.resume(),
                        ui::Screen::Settings if code == KeyCode::Escape => self.close_settings(),
                        _ => {}
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                self.perf.begin_frame(now);
                let dt = self
                    .last_time
                    .map(|t| (now - t).as_secs_f32())
                    .unwrap_or(0.016)
                    .min(0.1);
                self.last_time = Some(now);
                self.fps = self.fps * 0.9 + 0.1 / dt.max(1e-4);

                // poll inbound
                loop {
                    let pkt = match self.net_in.try_recv() {
                        Ok(p) => p,
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            if !self.net_dead {
                                self.net_dead = true;
                                self.hud
                                    .toasts
                                    .push(("connection lost".into(), Instant::now()));
                            }
                            break;
                        }
                    };
                    match pkt {
                        ServerPacket::Welcome {
                            id,
                            protocol,
                            bounds,
                            obstacles,
                        } => {
                            if protocol != PROTOCOL_VERSION {
                                log::error!(
                                    "protocol mismatch: сервер {protocol}, клиент {PROTOCOL_VERSION}"
                                );
                                self.protocol_ok = false;
                                self.hud.toasts.push((
                                    "версия клиента не совпадает с сервером".into(),
                                    Instant::now(),
                                ));
                            }
                            log::info!(
                                "welcome: player id {id}, {} стен, границы {:?}..{:?}",
                                obstacles.len(),
                                bounds.min,
                                bounds.max
                            );
                            self.player_id = Some(id);
                            self.server_bounds = bounds;
                            self.obstacles = obstacles;
                        }
                        ServerPacket::LoginResult { ok, error, .. } => {
                            if !ok {
                                let text = error.unwrap_or_else(|| "login failed".into());
                                log::error!("login failed: {text}");
                                self.hud.toasts.push((text, Instant::now()));
                            }
                        }
                        ServerPacket::Profile { profile, inventory } => {
                            log::info!("profile: {} level {}", profile.username, profile.level);
                            self.inventory = inventory;
                            self.hud.level = profile.level;
                            self.weapon_id = profile.weapon_id;
                            self.ammo = profile.ammo;
                            // Сервер уже поставил новичка на точку спавна карты:
                            // профиль несёт итоговую позицию, а не сохранённый ноль.
                            self.prediction.teleport(profile.position);
                            self.local_pos = profile.position;
                            // Новый рейд: старые счётчики и интерполяция чужого
                            // мира к нему не относятся.
                            self.extract_elapsed = 0.0;
                            self.last_health = 100.0;
                            self.interp = RemoteInterp::default();
                            self.latest_snapshot = None;
                            self.snapshot_at = None;
                        }
                        ServerPacket::Inventory(inv) => {
                            // Звук подбора — когда предметов стало больше.
                            let before: i64 =
                                self.inventory.items.iter().map(|i| i.quantity as i64).sum();
                            let after: i64 = inv.items.iter().map(|i| i.quantity as i64).sum();
                            if after > before {
                                self.audio.play(Sfx::Pickup, 1.0);
                            }
                            self.inventory = inv;
                        }
                        ServerPacket::Ping { nonce } => {
                            let _ = self.net_out.send(ClientPacket::Pong { nonce });
                        }
                        ServerPacket::Snapshot(snap) => self.apply_snapshot(snap),
                        ServerPacket::SnapshotDelta(delta) => {
                            if delta.base_tick == self.snapshot_tick {
                                if let Some(snap) = self.latest_snapshot.as_mut() {
                                    delta.apply(snap);
                                    let applied = snap.clone();
                                    self.apply_snapshot(applied);
                                }
                            } else {
                                // Клиент и сервер разошлись по базе: просим
                                // следующий кадр от того, что у нас есть.
                                log::debug!(
                                    "snapshot delta base {} != current {}",
                                    delta.base_tick,
                                    self.snapshot_tick
                                );
                                let _ = self.net_out.send(ClientPacket::SnapshotAck {
                                    tick: self.snapshot_tick,
                                });
                            }
                        }
                        ServerPacket::Pong { nonce } => {
                            if let Some((expected, sent)) = self.ping_at
                                && expected == nonce
                            {
                                let ms = sent.elapsed().as_secs_f32() * 1000.0;
                                self.rtt_ms = if self.rtt_ms <= 0.0 {
                                    ms
                                } else {
                                    self.rtt_ms * 0.8 + ms * 0.2
                                };
                                self.ping_at = None;
                            }
                        }
                        ServerPacket::Loadout { weapon_id, ammo } => {
                            self.weapon_id = weapon_id;
                            self.ammo = ammo;
                            self.weapon_cooldown = 0.0;
                            self.hud.toasts.push((
                                format!("оружие: {}", items::name(weapon_id)),
                                Instant::now(),
                            ));
                        }
                        ServerPacket::Stash { items } => {
                            if !items.is_empty() {
                                // Дельта: вынесенный лут добавляется к схрону,
                                // а полный список приходит только на коннекте.
                                for it in &items {
                                    merge_stack(&mut self.stash, it);
                                }
                                zn_sim::normalize_items(
                                    &mut self.stash,
                                    zn_sim::STASH_GRID_W,
                                    zn_sim::STASH_GRID_H,
                                );
                                let text = items
                                    .iter()
                                    .map(|i| {
                                        format!(
                                            "{} x{}",
                                            items::name(i.item_id.max(0) as u64),
                                            i.quantity
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", ");
                                self.hud
                                    .toasts
                                    .push((format!("в схрон: {text}"), Instant::now()));
                            }
                        }
                        ServerPacket::RaidResult { result } => {
                            log::info!(
                                "raid finished: {:?}, xp +{}, {} items, {}s",
                                result.reason,
                                result.xp_gained,
                                result.items.len(),
                                result.duration_secs
                            );
                            self.raid_result = Some(result);
                            self.screen = ui::Screen::Result;
                            self.release_cursor();
                        }
                        ServerPacket::StashContents { items } => {
                            self.stash = items;
                            log::info!("схрон: {} позиций", self.stash.len());
                        }
                        ServerPacket::LoadoutDenied { weapon_id, reason } => {
                            self.hud.toasts.push((
                                format!(
                                    "{}: {reason}. Возьмите оружие из схрона.",
                                    items::name(weapon_id)
                                ),
                                Instant::now(),
                            ));
                        }
                        ServerPacket::MoveRejected { reason } => {
                            self.hud
                                .toasts
                                .push((format!("перенос отклонён: {reason}"), Instant::now()));
                        }
                        ServerPacket::ShotResult {
                            hit,
                            killed,
                            zone,
                            damage,
                        } => {
                            if hit.is_some() {
                                self.audio.play(Sfx::Hit, 1.0);
                            }
                            let head = zone == Some(zn_sim::HitZone::Head);
                            if head {
                                self.hud
                                    .toasts
                                    .push((format!("хедшот — {damage:.0} урона"), Instant::now()));
                            }
                            if killed {
                                let text = if head {
                                    "цель уничтожена в голову"
                                } else {
                                    "цель уничтожена"
                                };
                                self.hud.toasts.push((text.into(), Instant::now()));
                            }
                        }
                    }
                }

                // update gamepad
                self.input.update_gamepad();

                // mouse look
                let (dx, dy) = self.input.mouse_delta;
                if self.cursor_captured {
                    let sensitivity = 0.002 * self.settings.mouse_sensitivity;
                    let invert = if self.settings.invert_y { -1.0 } else { 1.0 };
                    self.yaw += dx * sensitivity;
                    self.pitch = (self.pitch - dy * sensitivity * invert).clamp(-1.4, 1.4);
                    // gamepad right stick
                    self.yaw += self.input.gamepad_right.0 * dt * 2.0;
                    self.pitch =
                        (self.pitch - self.input.gamepad_right.1 * dt * 2.0).clamp(-1.4, 1.4);
                }

                // Предсказание: фиксированный шаг 1/60, ровно один вход на
                // тик сервера. Отправка раз в кадр (144 Гц) давала несколько
                // команд на тик и делала реплей недетерминированным.
                // В меню и на экране смерти игрок стоит: нулевой вектор уходит
                // на сервер, поэтому предсказание и сервер не расходятся.
                let move_vec = if self.screen == ui::Screen::Playing {
                    self.input.movement_vector()
                } else {
                    Default::default()
                };
                self.input_accum = (self.input_accum + dt).min(0.25);
                while self.input_accum >= TICK_DT {
                    self.input_accum -= TICK_DT;
                    let cmd = InputCommand {
                        seq: self.seq,
                        yaw: self.yaw,
                        pitch: self.pitch,
                        move_x: move_vec.x,
                        move_z: move_vec.z,
                        dt: TICK_DT,
                    };
                    self.seq += 1;
                    self.prediction
                        .predict(cmd.clone(), self.server_bounds, &self.obstacles);
                    let _ = self.net_out.send(ClientPacket::Input(cmd));
                }
                self.local_pos = self.prediction.position;
                self.weapon_cooldown = (self.weapon_cooldown - dt).max(0.0);

                // Пинг раз в секунду: без него UDP-канал не отличить от
                // живого, а игрок не видит лаг. Потерянный Pong не блокирует
                // следующий замер.
                let ping_due = match self.ping_at {
                    None => true,
                    Some((_, at)) => now.duration_since(at) >= Duration::from_secs(1),
                };
                if self.player_id.is_some() && ping_due {
                    let nonce = self.start.elapsed().as_millis() as u64;
                    let _ = self.net_out.send(ClientPacket::Ping { nonce });
                    self.ping_at = Some((nonce, now));
                }

                // Шаги: только на земле и при движении.
                let moving = move_vec.x.abs() + move_vec.z.abs() > 0.1;
                let on_ground = self.local_pos.y <= GROUND_Y + 0.01;
                if moving && on_ground {
                    self.step_timer -= dt;
                    if self.step_timer <= 0.0 {
                        self.audio.play(Sfx::Step, 0.8);
                        self.step_timer = 0.45;
                    }
                } else {
                    self.step_timer = 0.0;
                }

                // данные HUD: всё, что уже приходит в снапшоте и профиле
                if let Some(snap) = &self.latest_snapshot {
                    if let Some(pid) = self.player_id
                        && let Some(ps) = snap.players.iter().find(|p| p.id == pid)
                    {
                        // Урон и смерть — по падению здоровья между снапшотами.
                        if ps.health + 0.5 < self.last_health {
                            self.audio.play(Sfx::Hurt, 1.0);
                        }
                        if ps.health <= 0.0 && self.last_health > 0.0 {
                            self.audio.play(Sfx::Death, 1.0);
                        }
                        self.last_health = ps.health;

                        self.hud.health = ps.health;
                        self.hud.radiation = ps.radiation;
                        self.hud.hunger = ps.hunger;
                        self.hud.thirst = ps.thirst;
                        self.hud.level = ps.level;
                        self.hud.xp = ps.xp;
                    }
                    // Приближение: серверный таймер экстракции клиенту не передаётся.
                    let inside = snap
                        .extraction_zones
                        .iter()
                        .any(|z| (self.local_pos - z.position).length() <= z.radius);
                    if inside {
                        self.extract_elapsed += dt;
                    } else {
                        self.extract_elapsed = 0.0;
                    }
                    self.hud.extract = hud::extraction_progress(self.extract_elapsed);
                }
                self.hud.inventory = hud::format_inventory(&self.inventory);
                self.hud.fps = self.fps;
                self.hud.rtt_ms = self.rtt_ms;
                self.hud.ammo = self.ammo;
                self.hud.weapon = items::name(self.weapon_id).to_string();
                // Просроченные сообщения убираем, иначе очередь растёт бесконечно.
                self.hud
                    .toasts
                    .retain(|(_, at)| now.duration_since(*at).as_secs_f32() <= hud::TOAST_TTL);

                // camera
                let eye = self.local_pos + Vec3::Y * EYE_HEIGHT;
                let dir = Vec3::new(
                    self.yaw.cos() * self.pitch.cos(),
                    self.pitch.sin(),
                    self.yaw.sin() * self.pitch.cos(),
                );
                self.camera.eye = eye;
                self.camera.target = eye + dir;

                // Статика уже в мировых координатах, поэтому identity-модель.
                let mut render_list: Vec<(&Mesh, Option<&str>, Mat4)> = self
                    .scene_items
                    .iter()
                    .map(|(m, key)| (m, key.as_deref(), Mat4::IDENTITY))
                    .collect();

                // Два прохода. Первый создаёт меши (по одному на сущность) и
                // чистит исчезнувшие: вставлять в карту нельзя, пока список
                // отрисовки держит ссылки на её меши. Второй только читает и
                // собирает матрицы моделей, поэтому в кадре меши не пересоздаются.
                if let Some(snap) = &self.latest_snapshot
                    && let Some(pid) = self.player_id
                {
                    let renderer = self.renderer.as_ref().unwrap();
                    let mut keep: std::collections::HashSet<u64> = std::collections::HashSet::new();

                    for ps in &snap.players {
                        if ps.id == pid {
                            continue;
                        }
                        keep.insert(ps.id);
                        if !self.entity_meshes.contains_key(&ps.id) {
                            let started = Instant::now();
                            let (vs, is) = geom::humanoid_local([0.25, 0.65, 0.25], 1.0);
                            let m = renderer.create_mesh(&vs, &is);
                            self.perf.mesh_rebuild(started.elapsed());
                            self.entity_meshes.insert(ps.id, (m, "metal"));
                        }
                    }
                    for c in &snap.creatures {
                        let key = c.id + 1_000_000;
                        keep.insert(key);
                        if !self.entity_meshes.contains_key(&key) {
                            let started = Instant::now();
                            let (vs, is) = geom::quadruped_local([0.75, 0.25, 0.2], 1.0);
                            let m = renderer.create_mesh(&vs, &is);
                            self.perf.mesh_rebuild(started.elapsed());
                            self.entity_meshes.insert(key, (m, "rust"));
                        }
                    }
                    for l in &snap.loot {
                        let key = l.id + 2_000_000;
                        keep.insert(key);
                        if !self.entity_meshes.contains_key(&key) {
                            let started = Instant::now();
                            let (vs, is) = geom::unit_box([1.0, 1.0, 0.2], 1.0);
                            let m = renderer.create_mesh(&vs, &is);
                            self.perf.mesh_rebuild(started.elapsed());
                            self.entity_meshes.insert(key, (m, "wood"));
                        }
                    }
                    for a in &snap.anomalies {
                        let key = a.id + 3_000_000;
                        keep.insert(key);
                        if !self.entity_meshes.contains_key(&key) {
                            let started = Instant::now();
                            let (vs, is) = geom::unit_box([0.2, 0.2, 0.8], 1.0);
                            let m = renderer.create_mesh(&vs, &is);
                            self.perf.mesh_rebuild(started.elapsed());
                            self.entity_meshes.insert(key, (m, "concrete"));
                        }
                    }
                    self.entity_meshes.retain(|k, _| keep.contains(k));

                    // alpha: 0 — предыдущий снапшот, 1 — текущий.
                    let alpha = self.interp.alpha(now);

                    for ps in &snap.players {
                        if ps.id == pid {
                            continue;
                        }
                        let (pos, yaw_rad) = self
                            .interp
                            .player(ps.id, alpha)
                            .unwrap_or((ps.position, ps.yaw));
                        let yaw = glam::Quat::from_rotation_y(yaw_rad);
                        let far =
                            zn_scene::select_lod((pos - self.local_pos).length(), &[25.0]) > 0;
                        let chosen = if far {
                            self.player_lod.as_ref().map(|m| (m, "metal"))
                        } else {
                            self.entity_meshes.get(&ps.id).map(|(m, mat)| (m, *mat))
                        };
                        if let Some((mesh, material)) = chosen {
                            let model = if far {
                                Mat4::from_scale_rotation_translation(
                                    Vec3::new(0.6, 1.8, 0.6),
                                    yaw,
                                    pos + Vec3::Y * 0.9,
                                )
                            } else {
                                Mat4::from_rotation_translation(yaw, pos)
                            };
                            render_list.push((mesh, Some(material), model));
                        }
                    }
                    for c in &snap.creatures {
                        let key = c.id + 1_000_000;
                        let pos = self.interp.creature(c.id, alpha).unwrap_or(c.position);
                        let far =
                            zn_scene::select_lod((pos - self.local_pos).length(), &[30.0]) > 0;
                        let chosen = if far {
                            self.creature_lod.as_ref().map(|m| (m, "rust"))
                        } else {
                            self.entity_meshes.get(&key).map(|(m, mat)| (m, *mat))
                        };
                        if let Some((mesh, material)) = chosen {
                            let model = if far {
                                Mat4::from_scale_rotation_translation(
                                    Vec3::new(0.8, 1.2, 0.8),
                                    glam::Quat::IDENTITY,
                                    pos + Vec3::Y * 0.6,
                                )
                            } else {
                                Mat4::from_translation(pos)
                            };
                            render_list.push((mesh, Some(material), model));
                        }
                    }
                    for l in &snap.loot {
                        let key = l.id + 2_000_000;
                        if let Some((m, material)) = self.entity_meshes.get(&key) {
                            render_list.push((
                                m,
                                Some(*material),
                                Mat4::from_scale_rotation_translation(
                                    Vec3::splat(0.6),
                                    glam::Quat::IDENTITY,
                                    l.position,
                                ),
                            ));
                        }
                    }
                    for a in &snap.anomalies {
                        let key = a.id + 3_000_000;
                        if let Some((m, material)) = self.entity_meshes.get(&key) {
                            render_list.push((
                                m,
                                Some(*material),
                                Mat4::from_scale_rotation_translation(
                                    Vec3::splat(a.radius * 2.0),
                                    glam::Quat::IDENTITY,
                                    a.position,
                                ),
                            ));
                        }
                    }
                }
                let render_items: Vec<RenderItem> = render_list
                    .iter()
                    .map(|(m, key, model)| RenderItem {
                        mesh: m,
                        texture: key.and_then(|k| self.textures.get(k)),
                        model: *model,
                    })
                    .collect();

                // Ввод интерфейса идёт через egui-winit: без него кнопки не
                // получали бы клики, а клик по меню уходил бы в выстрел.
                let mut raw = match (self.egui_state.as_mut(), self.window.as_ref()) {
                    (Some(state), Some(window)) => state.take_egui_input(window.as_ref()),
                    _ => egui::RawInput::default(),
                };
                raw.time = Some(self.start.elapsed().as_secs_f64());

                let screen = self.screen;
                let mut action = ui::Action::None;
                let hud_state = &self.hud;
                let inventory = &self.inventory;
                let stash = &self.stash;
                let raid_result = self.raid_result.as_ref();
                let weapon_id = self.weapon_id;
                let protocol_ok = self.protocol_ok;
                let settings = self.settings;
                let settings_return = self.settings_return;
                let mut inventory_ui = std::mem::take(&mut self.inventory_ui);
                let output = self.egui_ctx.run(raw, |ctx| {
                    action = match screen {
                        ui::Screen::Menu => ui::menu(
                            ctx,
                            weapon_id,
                            protocol_ok,
                            inventory,
                            stash,
                            &mut inventory_ui,
                        ),
                        ui::Screen::Settings => ui::settings(ctx, &settings, settings_return),
                        ui::Screen::Dead => ui::dead(ctx, hud_state.xp, hud_state.level),
                        ui::Screen::Result => match raid_result {
                            Some(result) => ui::result(ctx, result),
                            None => ui::Action::ToMenu,
                        },
                        ui::Screen::Paused => {
                            hud::draw(ctx, hud_state, Instant::now());
                            ui::paused(ctx)
                        }
                        ui::Screen::Inventory => {
                            hud::draw(ctx, hud_state, Instant::now());
                            ui::inventory(ctx, inventory, stash, weapon_id, &mut inventory_ui)
                        }
                        ui::Screen::Playing => {
                            hud::draw(ctx, hud_state, Instant::now());
                            ui::Action::None
                        }
                    };
                });
                self.inventory_ui = inventory_ui;

                let mut render_stats = zn_render::RenderStats::default();
                if let Some(r) = self.renderer.as_mut() {
                    let ui = zn_render::UiFrame {
                        ctx: &self.egui_ctx,
                        output,
                    };
                    let _ = r.render(&render_items, &self.camera, Some(ui));
                    render_stats = r.stats();
                }

                self.perf.end_frame(render_items.len());
                if let Some(line) = self.perf.take_report(Instant::now()) {
                    // Сеть в том же окне: queue — неподтверждённые команды,
                    // corrections — сколько раз пришлось принять серверную
                    // позицию. Рост любого из чисел виден до жалобы игрока.
                    log::info!(
                        "[perf] {line} drawn={} culled={} calls={} pred_queue={} ack={} corrections={}",
                        render_stats.drawn,
                        render_stats.culled,
                        render_stats.draw_calls,
                        self.prediction.pending_len(),
                        self.prediction.last_acked(),
                        self.prediction.corrections()
                    );
                }

                // Действие меню применяется после кадра: до этого момента
                // render_items держит ссылки на меши и текстуры, и захват
                // курсора через &mut self с ними бы конфликтовал.
                match action {
                    ui::Action::StartRaid => {
                        // Сервер решает, можно ли начать рейд: клиент не должен
                        // телепортировать себя сам.
                        let _ = self.net_out.send(ClientPacket::StartRaid);
                        self.raid_result = None;
                        self.screen = ui::Screen::Playing;
                        self.capture_cursor();
                    }
                    ui::Action::ToMenu => {
                        self.screen = ui::Screen::Menu;
                        self.release_cursor();
                    }
                    ui::Action::Resume => self.resume(),
                    ui::Action::OpenInventory => self.open_inventory(),
                    ui::Action::CloseInventory => self.close_inventory(),
                    ui::Action::OpenSettings => self.open_settings(),
                    ui::Action::CloseSettings => self.close_settings(),
                    ui::Action::ApplySettings(settings) => {
                        self.settings = settings.clamped();
                        self.apply_settings();
                    }
                    ui::Action::Quit => event_loop.exit(),
                    ui::Action::SetWeapon(id) => {
                        let _ = self
                            .net_out
                            .send(ClientPacket::SetLoadout { weapon_id: id });
                    }
                    ui::Action::ConsumeItem(item_id) => {
                        let _ = self.net_out.send(ClientPacket::Consume { item_id });
                    }
                    ui::Action::MoveItem {
                        from,
                        item_id,
                        to,
                        x,
                        y,
                        rotated,
                    } => {
                        let _ = self.net_out.send(ClientPacket::MoveItem {
                            from,
                            item_id,
                            to,
                            x,
                            y,
                            rotated,
                        });
                    }
                    ui::Action::None => {}
                }

                self.input.reset_frame();

                if let Some(w) = self.window.as_ref() {
                    w.request_redraw();
                }
            }
            _ => {
                self.input.handle_window_event(&event);
            }
        }
    }

    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        if let DeviceEvent::MouseMotion { delta } = event
            && self.cursor_captured
        {
            self.input.add_mouse_delta(delta.0 as f32, delta.1 as f32);
        }
    }
}

impl App {
    /// Оценка серверного времени в миллисекундах по последнему кадру.
    fn server_time_ms(&self) -> u64 {
        let base = self.snapshot_tick as f64 * (1000.0 / 60.0);
        let since = self
            .snapshot_at
            .map(|at| at.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        (base + since) as u64
    }

    /// Применяет полный кадр: keyframe с сервера или собранный из дельты.
    fn apply_snapshot(&mut self, snap: Snapshot) {
        if snap.tick <= self.snapshot_tick {
            return;
        }
        self.snapshot_tick = snap.tick;
        self.snapshot_at = Some(Instant::now());
        // Реконсиляция: принимаем серверную позицию только
        // при заметной ошибке и переигрываем неподтверждённые
        // команды — иначе мышь и клавиши отстают на снапшот.
        if let Some(pid) = self.player_id
            && let Some(ps) = snap.players.iter().find(|p| p.id == pid)
        {
            self.prediction.reconcile(
                ps.position,
                ps.velocity,
                ps.last_input_seq,
                self.server_bounds,
                &self.obstacles,
            );
            self.local_pos = self.prediction.position;
            self.weapon_id = ps.weapon_id;
            // Патроны — серверная правда. Локальный расход
            // нужен только для мгновенной реакции; если
            // сервер выстрел отклонил, он вернёт число.
            self.ammo = ps.ammo;
            // Углы не перезаписываем: снапшот приходит 20 раз
            // в секунду и перебивал бы локальную мышь.
            if let Some((yaw, pitch)) =
                zn_sim::correct_angles((self.yaw, self.pitch), (ps.yaw, ps.pitch))
            {
                self.yaw = yaw;
                self.pitch = pitch;
            }
            // Смерть ловим по снапшоту: сервер уже
            // респавнит игрока, поэтому без экрана игрок
            // не заметил бы, что погиб.
            if ps.health <= 0.0 && self.screen == ui::Screen::Playing {
                self.screen = ui::Screen::Dead;
                self.release_cursor();
            }
        }
        // Копии позиций для сглаживания: `snap` уезжает в
        // `latest_snapshot`, а предыдущие значения нужны в кадре.
        self.interp
            .push(&snap.players, &snap.creatures, Instant::now());
        self.latest_snapshot = Some(snap);
        let _ = self.net_out.send(ClientPacket::SnapshotAck {
            tick: self.snapshot_tick,
        });
    }

    fn pause(&mut self) {
        if self.screen == ui::Screen::Playing {
            self.screen = ui::Screen::Paused;
            self.inventory_ui = ui::InventoryUiState::default();
            self.release_cursor();
        }
    }

    fn resume(&mut self) {
        self.screen = ui::Screen::Playing;
        self.capture_cursor();
    }

    fn open_inventory(&mut self) {
        if matches!(self.screen, ui::Screen::Playing | ui::Screen::Paused) {
            self.screen = ui::Screen::Inventory;
            self.inventory_ui = ui::InventoryUiState::default();
            self.release_cursor();
        }
    }

    fn close_inventory(&mut self) {
        self.screen = ui::Screen::Playing;
        self.inventory_ui = ui::InventoryUiState::default();
        self.capture_cursor();
    }

    fn open_settings(&mut self) {
        self.settings_return = self.screen;
        self.screen = ui::Screen::Settings;
        self.inventory_ui = ui::InventoryUiState::default();
        self.release_cursor();
    }

    fn close_settings(&mut self) {
        self.save_settings();
        self.screen = self.settings_return;
        if self.screen == ui::Screen::Playing {
            self.capture_cursor();
        } else {
            self.release_cursor();
        }
    }

    fn save_settings(&self) {
        if let Err(e) = self.settings.save(&settings_path()) {
            log::warn!("настройки не сохранены: {e}");
        }
    }

    fn apply_settings(&mut self) {
        self.camera.fov_y = self.settings.fov_radians();
        self.camera.far = self.settings.render_distance;
        self.audio.master_volume = self.settings.master_volume;
        if let Some(renderer) = self.renderer.as_mut() {
            let actual = renderer.set_vsync(self.settings.vsync);
            if !actual && !self.settings.vsync {
                // Поверхность умеет только Fifo — показываем это честно,
                // иначе чекбокс обещал бы режим, которого нет.
                self.settings.vsync = true;
                self.hud.toasts.push((
                    "V-Sync: режим без синхронизации недоступен".into(),
                    Instant::now(),
                ));
            }
        }
    }

    fn try_pickup(&mut self) {
        let Some(snap) = &self.latest_snapshot else {
            return;
        };
        let pos = self.local_pos;
        let mut best: Option<(u64, f32)> = None;
        for l in &snap.loot {
            let d = (l.position - pos).length();
            if d < 2.0 && (best.is_none() || d < best.unwrap().1) {
                best = Some((l.id, d));
            }
        }
        for a in &snap.artifacts {
            let d = (a.position - pos).length();
            if d < 2.0 && (best.is_none() || d < best.unwrap().1) {
                // Артефакты живут в своём диапазоне id, иначе сервер
                // примет их за предмет инвентаря.
                best = Some((zn_sim::ids::ARTIFACT_BASE + a.id, d));
            }
        }
        if let Some((eid, _)) = best {
            let _ = self.net_out.send(ClientPacket::Pickup { entity_id: eid });
        }
    }

    /// Захват курсора — только по явному действию игрока (меню или клик).
    fn capture_cursor(&mut self) {
        self.cursor_captured = true;
        self.input.cursor_captured = true;
        if let Some(w) = self.window.as_ref() {
            let _ = w.set_cursor_grab(CursorGrabMode::Locked);
            w.set_cursor_visible(false);
        }
    }

    fn release_cursor(&mut self) {
        self.cursor_captured = false;
        self.input.cursor_captured = false;
        if let Some(w) = self.window.as_ref() {
            let _ = w.set_cursor_grab(CursorGrabMode::None);
            w.set_cursor_visible(true);
        }
    }
}

/// Настройки лежат рядом с crash.log: `%APPDATA%\ZONA` на Windows,
/// иначе — в рабочем каталоге (Linux/CI).
fn settings_path() -> std::path::PathBuf {
    std::env::var("APPDATA")
        .map(|p| {
            std::path::PathBuf::from(p)
                .join("ZONA")
                .join("settings.ron")
        })
        .unwrap_or_else(|_| std::path::PathBuf::from("settings.ron"))
}

/// Каталог с ресурсами (`assets/`, `maps/`).
///
/// Сначала смотрим рядом с исполняемым файлом — так игру ставит лаунчер и так
/// ресурсы находятся независимо от рабочего каталога процесса. Если рядом
/// ничего нет (запуск через `cargo run` из корня), отдаём текущий каталог.
fn asset_root() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
        && dir.join("assets").is_dir()
    {
        return dir.to_path_buf();
    }
    std::path::PathBuf::from(".")
}

/// Локальная учётная запись для запуска без явных кредов.
///
/// Лаунчер не спрашивает логин, поэтому при первом старте создаётся запись
/// `player_<hex>` со случайным паролем и сохраняется рядом с игрой в
/// `player.txt` (две строки). Возвращает `(логин, пароль, создана_ли_сейчас)` —
/// флаг нужен, чтобы при первом запуске отправить Register, а дальше Login.
fn local_account(root: &std::path::Path) -> (String, String, bool) {
    let path = root.join("player.txt");
    if let Ok(text) = std::fs::read_to_string(&path) {
        let mut lines = text.lines();
        if let (Some(u), Some(p)) = (lines.next(), lines.next()) {
            let (u, p) = (u.trim(), p.trim());
            if !u.is_empty() && p.len() >= 8 {
                return (u.to_string(), p.to_string(), false);
            }
        }
    }

    let username = format!("player_{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let password = uuid::Uuid::new_v4().to_string();
    if let Err(e) = std::fs::write(&path, format!("{username}\n{password}\n")) {
        log::warn!("не удалось сохранить {}: {e}", path.display());
    }
    (username, password, true)
}

fn main() {
    // crash reporting: write panic to crash.log
    std::panic::set_hook(Box::new(|info| {
        let log_path = std::env::var("APPDATA")
            .map(|p| std::path::PathBuf::from(p).join("ZONA").join("crash.log"))
            .unwrap_or_else(|_| std::path::PathBuf::from("crash.log"));
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let msg = format!("ZONA crash: {}\n", info);
        let _ = std::fs::write(&log_path, &msg);
        eprintln!("{}", msg);
    }));

    zn_core::logging::init("zn-client");
    let cli = Cli::parse();
    if cli.telemetry {
        log::info!("telemetry: session_start");
    }

    let (out_tx, out_rx) = mpsc::channel::<ClientPacket>();
    let (in_tx, in_rx) = mpsc::channel::<ServerPacket>();

    // network thread
    let token = cli.token.clone();
    let mut username = cli
        .username
        .clone()
        .or_else(|| std::env::var("ZONA_USERNAME").ok());
    let mut password = cli
        .password
        .clone()
        .or_else(|| std::env::var("ZONA_PASSWORD").ok());
    let mut register = cli.register;

    // Лаунчер не передаёт учётные данные, поэтому без явных кредов заводим
    // локальную запись рядом с игрой: иначе кнопка Play просто закрывала бы игру.
    if token.is_none() && (username.is_none() || password.is_none()) {
        let (u, p, is_new) = local_account(&asset_root());
        log::info!(
            "local account: {u} ({})",
            if is_new { "created" } else { "existing" }
        );
        username = Some(u);
        password = Some(p);
        register = register || is_new;
    }
    let username = username.unwrap_or_default();
    let password = password.unwrap_or_default();
    let server = cli.server.clone();
    let port = cli.port;
    thread::spawn(move || {
        use std::io::{Read, Write};
        use std::net::UdpSocket;

        // TCP auth
        let mut stream = match std::net::TcpStream::connect(format!("{}:{}", server, port)) {
            Ok(s) => s,
            Err(e) => {
                log::error!("connect failed: {}", e);
                return;
            }
        };
        stream.set_read_timeout(Some(Duration::from_millis(5))).ok();

        let mut got_token = token;
        if got_token.is_none() {
            let pkt = if register {
                ClientPacket::Register { username, password }
            } else {
                ClientPacket::Login { username, password }
            };
            if let Ok(data) = encode(&pkt) {
                let _ = stream.write_all(&data);
            }
            for _ in 0..200 {
                let mut len_buf = [0u8; 4];
                if stream.read_exact(&mut len_buf).is_ok() {
                    let len = u32::from_le_bytes(len_buf) as usize;
                    if len > zn_net::MAX_FRAME_BYTES {
                        log::error!("server sent oversized frame: {len} bytes");
                        break;
                    }
                    let mut buf = vec![0u8; len];
                    if stream.read_exact(&mut buf).is_ok()
                        && let Ok(ServerPacket::LoginResult { ok, token, .. }) =
                            bincode::deserialize::<ServerPacket>(&buf)
                    {
                        if ok {
                            got_token = token;
                        }
                        break;
                    }
                }
                thread::sleep(Duration::from_millis(10));
            }
        }

        let token = match got_token {
            Some(t) => t,
            None => {
                log::error!("auth failed");
                return;
            }
        };

        // UDP setup
        let udp = match UdpSocket::bind("0.0.0.0:0") {
            Ok(u) => u,
            Err(e) => {
                log::error!("udp bind failed: {}", e);
                return;
            }
        };
        udp.set_read_timeout(Some(Duration::from_millis(5))).ok();
        let server_addr: std::net::SocketAddr = format!("{}:{}", server, port + 1).parse().unwrap();

        // register endpoint
        let reg = ClientPacket::Connect {
            token: token.clone(),
        };
        // UDP-датаграмма сама имеет границы: префикс длины здесь не нужен,
        // сервер парсит датаграмму как сырой bincode.
        if let Ok(data) = bincode::serialize(&reg) {
            let _ = udp.send_to(&data, server_addr);
        }

        loop {
            // outbound
            while let Ok(pkt) = out_rx.try_recv() {
                if let Ok(data) = bincode::serialize(&pkt) {
                    let _ = udp.send_to(&data, server_addr);
                }
            }
            // inbound
            let mut buf = vec![0u8; 65535];
            if let Ok((n, _addr)) = udp.recv_from(&mut buf) {
                buf.truncate(n);
                if let Ok(pkt) = bincode::deserialize::<ServerPacket>(&buf) {
                    let _ = in_tx.send(pkt);
                }
            }
            thread::sleep(Duration::from_millis(1));
        }
    });

    let settings = Settings::load(&settings_path()).unwrap_or_default();
    let mut audio = zn_audio::AudioManager::new();
    audio.master_volume = settings.master_volume;
    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App {
        window: None,
        renderer: None,
        input: InputState::new(),
        camera: Camera {
            eye: Vec3::ZERO,
            target: Vec3::Z,
            up: Vec3::Y,
            fov_y: settings.fov_radians(),
            aspect: 1.0,
            near: 0.1,
            far: settings.render_distance,
        },
        scene_items: Vec::new(),
        entity_meshes: std::collections::HashMap::new(),
        player_lod: None,
        creature_lod: None,
        textures: std::collections::HashMap::new(),
        egui_ctx: egui::Context::default(),
        hud: hud::HudState::default(),
        start: Instant::now(),
        inventory: zn_sim::Inventory::default(),
        stash: Vec::new(),
        raid_result: None,
        extract_elapsed: 0.0,
        fps: 0.0,
        perf: perf::PerfStats::new(),
        net_dead: false,
        audio,
        step_timer: 0.0,
        last_health: 100.0,
        last_time: None,
        yaw: 0.0,
        pitch: 0.0,
        cursor_captured: false,
        scene_path: cli.scene,
        prediction: Prediction::new(Vec3::new(0.0, GROUND_Y, 0.0)),
        server_bounds: Bounds::default(),
        obstacles: Vec::new(),
        input_accum: 0.0,
        weapon_id: items::DEFAULT_WEAPON,
        ammo: items::START_AMMO,
        weapon_cooldown: 0.0,
        rtt_ms: 0.0,
        ping_at: None,
        protocol_ok: true,
        screen: ui::Screen::Menu,
        settings_return: ui::Screen::Menu,
        settings,
        inventory_ui: ui::InventoryUiState::default(),
        egui_state: None,
        interp: RemoteInterp::default(),
        net_out: out_tx,
        net_in: in_rx,
        player_id: None,
        latest_snapshot: None,
        seq: 0,
        snapshot_tick: 0,
        snapshot_at: None,
        local_pos: Vec3::new(0.0, 0.8, 0.0),
    };
    event_loop.run_app(&mut app).expect("run");
}
