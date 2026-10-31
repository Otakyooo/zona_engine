use clap::Parser;
use glam::Vec3;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::KeyCode;
use winit::window::{CursorGrabMode, Window, WindowId};
use zn_input::InputState;
use zn_net::{ClientPacket, ServerPacket, encode};
use zn_render::{Camera, Mesh, Renderer, Texture, Vertex};
use zn_scene::load_scene;
use zn_sim::{InputCommand, Snapshot};

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

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    input: InputState,
    camera: Camera,
    meshes: Vec<Mesh>,
    entity_meshes: std::collections::HashMap<u64, (Mesh, glam::Vec3)>,
    last_time: Option<Instant>,
    yaw: f32,
    pitch: f32,
    vertical_velocity: f32,
    cursor_captured: bool,
    scene_path: Option<String>,

    // networking
    net_out: Sender<ClientPacket>,
    net_in: Receiver<ServerPacket>,
    player_id: Option<u64>,
    latest_snapshot: Option<Snapshot>,
    seq: u32,
    local_pos: Vec3,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let attrs = Window::default_attributes().with_title("ZONA - client");
            let window = Arc::new(event_loop.create_window(attrs).expect("window"));
            let renderer = pollster::block_on(Renderer::new(window.clone())).expect("renderer");

            // meshes
            let floor_verts = vec![
                Vertex {
                    position: [-20.0, 0.0, -20.0],
                    normal: [0.0, 1.0, 0.0],
                    color: [0.3, 0.35, 0.3],
                    uv: [0.0, 0.0],
                },
                Vertex {
                    position: [20.0, 0.0, -20.0],
                    normal: [0.0, 1.0, 0.0],
                    color: [0.3, 0.35, 0.3],
                    uv: [1.0, 0.0],
                },
                Vertex {
                    position: [20.0, 0.0, 20.0],
                    normal: [0.0, 1.0, 0.0],
                    color: [0.3, 0.35, 0.3],
                    uv: [1.0, 1.0],
                },
                Vertex {
                    position: [-20.0, 0.0, 20.0],
                    normal: [0.0, 1.0, 0.0],
                    color: [0.3, 0.35, 0.3],
                    uv: [0.0, 1.0],
                },
            ];
            let floor_idx = vec![0, 2, 1, 0, 3, 2];
            let floor_mesh = renderer.create_mesh(&floor_verts, &floor_idx);

            let mut meshes = vec![floor_mesh];

            let scene_opt = self
                .scene_path
                .clone()
                .or_else(|| Some("maps/arena.ron".to_string()))
                .and_then(|p| load_scene(std::path::Path::new(&p)).ok());

            if let Some(scene) = scene_opt {
                for node in &scene.nodes {
                    let color = node.color;
                    let (vs, is) = if let Some(mesh_path) = &node.mesh {
                        if mesh_path.ends_with(".gltf") || mesh_path.ends_with(".glb") {
                            if let Ok(lm) = zn_assets::load_gltf(std::path::Path::new(mesh_path)) {
                                let mut vs = Vec::new();
                                for (i, p) in lm.positions.iter().enumerate() {
                                    let v = glam::Vec3::from(*p);
                                    let w = node.transform.matrix() * v.extend(1.0);
                                    let n = lm
                                        .normals
                                        .as_ref()
                                        .and_then(|ns| ns.get(i).copied())
                                        .unwrap_or([0.0, 1.0, 0.0]);
                                    let wn = node
                                        .transform
                                        .matrix()
                                        .transform_vector3(glam::Vec3::from(n))
                                        .normalize_or_zero();
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
                                make_cube(node.transform.position, 0.5, color)
                            }
                        } else {
                            make_cube(node.transform.position, 0.5, color)
                        }
                    } else {
                        make_cube(node.transform.position, 0.5, color)
                    };
                    meshes.push(renderer.create_mesh(&vs, &is));
                }
            } else {
                for i in -2..=2 {
                    for j in -2..=2 {
                        let x = i as f32 * 4.0;
                        let z = j as f32 * 4.0;
                        let cube = make_cube(Vec3::new(x, 0.5, z), 0.5, [0.8, 0.4, 0.2]);
                        meshes.push(renderer.create_mesh(&cube.0, &cube.1));
                    }
                }
            }

            let camera = Camera {
                eye: Vec3::new(0.0, 2.0, 5.0),
                target: Vec3::new(0.0, 1.0, 0.0),
                up: Vec3::Y,
                fov_y: 1.0,
                aspect: window.inner_size().width as f32 / window.inner_size().height.max(1) as f32,
                near: 0.1,
                far: 100.0,
            };

            let _ = window.set_cursor_grab(CursorGrabMode::Locked);
            window.set_cursor_visible(false);

            self.window = Some(window);
            self.renderer = Some(renderer);
            self.input = InputState::new();
            self.input.cursor_captured = true;
            self.camera = camera;
            self.meshes = meshes;
            self.last_time = Some(Instant::now());
            self.cursor_captured = true;
            self.local_pos = Vec3::new(0.0, 0.8, 0.0);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
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
                    self.cursor_captured = true;
                    self.input.cursor_captured = true;
                    if let Some(w) = self.window.as_ref() {
                        let _ = w.set_cursor_grab(CursorGrabMode::Locked);
                        w.set_cursor_visible(false);
                    }
                }
                if *state == winit::event::ElementState::Pressed
                    && *button == winit::event::MouseButton::Left
                    && self.cursor_captured
                {
                    let origin = self.local_pos + Vec3::new(0.0, 1.6, 0.0);
                    let dir = Vec3::new(
                        self.yaw.cos() * self.pitch.cos(),
                        self.pitch.sin(),
                        self.yaw.sin() * self.pitch.cos(),
                    );
                    let _ = self.net_out.send(ClientPacket::Shot {
                        seq: self.seq,
                        origin: [origin.x, origin.y, origin.z],
                        dir: [dir.x, dir.y, dir.z],
                        client_time: 0,
                    });
                    self.seq += 1;
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                self.input.handle_key_event(event);
                use winit::keyboard::PhysicalKey;
                if event.state == winit::event::ElementState::Pressed
                    && let PhysicalKey::Code(KeyCode::Escape) = event.physical_key
                {
                    if self.cursor_captured {
                        self.cursor_captured = false;
                        self.input.cursor_captured = false;
                        if let Some(w) = self.window.as_ref() {
                            let _ = w.set_cursor_grab(CursorGrabMode::None);
                            w.set_cursor_visible(true);
                        }
                    } else {
                        self.cursor_captured = true;
                        self.input.cursor_captured = true;
                        if let Some(w) = self.window.as_ref() {
                            let _ = w.set_cursor_grab(CursorGrabMode::Locked);
                            w.set_cursor_visible(false);
                        }
                    }
                }
                // E: pickup
                if event.state == winit::event::ElementState::Pressed
                    && let PhysicalKey::Code(KeyCode::KeyE) = event.physical_key
                    && let Some(snap) = &self.latest_snapshot
                {
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
                            best = Some((a.id, d));
                        }
                    }
                    if let Some((eid, _)) = best {
                        let _ = self.net_out.send(ClientPacket::Pickup { entity_id: eid });
                    }
                }
                if event.state == winit::event::ElementState::Pressed {
                    if let PhysicalKey::Code(KeyCode::Digit1) = event.physical_key {
                        let _ = self.net_out.send(ClientPacket::Consume { item_id: 1 });
                    }
                    if let PhysicalKey::Code(KeyCode::Digit2) = event.physical_key {
                        let _ = self.net_out.send(ClientPacket::Consume { item_id: 2 });
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                let now = Instant::now();
                let dt = self
                    .last_time
                    .map(|t| (now - t).as_secs_f32())
                    .unwrap_or(0.016)
                    .min(0.1);
                self.last_time = Some(now);

                // poll inbound
                while let Ok(pkt) = self.net_in.try_recv() {
                    match pkt {
                        ServerPacket::Welcome { id } => {
                            self.player_id = Some(id);
                        }
                        ServerPacket::LoginResult { ok, error, .. } => {
                            if !ok {
                                log::error!("login failed: {:?}", error);
                            }
                        }
                        ServerPacket::Profile { profile, .. } => {
                            log::info!("profile: {} level {}", profile.username, profile.level);
                        }
                        ServerPacket::Snapshot(snap) => {
                            // reconcile local
                            if let Some(pid) = self.player_id
                                && let Some(ps) = snap.players.iter().find(|p| p.id == pid)
                            {
                                let server_pos = ps.position;
                                let dist = self.local_pos.distance(server_pos);
                                if dist > 2.0 {
                                    self.local_pos = server_pos;
                                } else if dist > 0.5 {
                                    self.local_pos = self.local_pos.lerp(server_pos, 0.25);
                                }
                                self.yaw = ps.yaw;
                                self.pitch = ps.pitch;
                            }
                            self.latest_snapshot = Some(snap);
                        }
                        ServerPacket::ShotResult { .. } => {}
                    }
                }

                // update gamepad
                self.input.update_gamepad();

                // mouse look
                let (dx, dy) = self.input.mouse_delta;
                if self.cursor_captured {
                    self.yaw -= dx * 0.002;
                    self.pitch = (self.pitch - dy * 0.002).clamp(-1.4, 1.4);
                    // gamepad right stick
                    self.yaw -= self.input.gamepad_right.0 * dt * 2.0;
                    self.pitch =
                        (self.pitch - self.input.gamepad_right.1 * dt * 2.0).clamp(-1.4, 1.4);
                }

                // movement (simplified, matching server)
                let move_vec = self.input.movement_vector();
                let forward = Vec3::new(self.yaw.cos(), 0.0, self.yaw.sin());
                let right = Vec3::new(forward.z, 0.0, -forward.x);
                let mut vel = forward * (-move_vec.z) + right * move_vec.x;
                vel *= 5.0;

                // gravity
                if self.local_pos.y <= 0.8 {
                    self.vertical_velocity = 0.0;
                } else {
                    self.vertical_velocity -= 9.81 * dt;
                }
                vel.y = self.vertical_velocity;

                self.local_pos.x += vel.x * dt;
                self.local_pos.y += vel.y * dt;
                self.local_pos.z += vel.z * dt;
                if self.local_pos.y < 0.8 {
                    self.local_pos.y = 0.8;
                    self.vertical_velocity = 0.0;
                }
                self.local_pos.x = self.local_pos.x.clamp(-19.0, 19.0);
                self.local_pos.z = self.local_pos.z.clamp(-19.0, 19.0);

                // send input
                let cmd = InputCommand {
                    seq: self.seq,
                    yaw: self.yaw,
                    pitch: self.pitch,
                    move_x: move_vec.x,
                    move_z: move_vec.z,
                    dt,
                };
                self.seq += 1;
                let _ = self.net_out.send(ClientPacket::Input(cmd));

                // camera
                let eye = self.local_pos + Vec3::new(0.0, 1.6, 0.0);
                let dir = Vec3::new(
                    self.yaw.cos() * self.pitch.cos(),
                    self.pitch.sin(),
                    self.yaw.sin() * self.pitch.cos(),
                );
                self.camera.eye = eye;
                self.camera.target = eye + dir;

                // render remote players as cubes (rebuilt each frame? optimize: reuse meshes; simple: append to meshes list is not ideal)
                // We'll render base meshes only; remote players drawn by adding temporary meshes to a separate list.
                let mut render_list: Vec<&Mesh> = self.meshes.iter().collect();

                // build remote player meshes on the fly (cheap enough for M3)
                let mut entity_keys: Vec<u64> = Vec::new();
                let mut keep_keys: std::collections::HashSet<u64> =
                    std::collections::HashSet::new();
                if let Some(snap) = &self.latest_snapshot
                    && let Some(pid) = self.player_id
                {
                    let renderer = self.renderer.as_ref().unwrap();
                    for ps in &snap.players {
                        if ps.id == pid {
                            continue;
                        }
                        let key = ps.id;
                        let pos = ps.position + Vec3::new(0.0, 0.5, 0.0);
                        let needs = self
                            .entity_meshes
                            .get(&key)
                            .map(|(_, p)| p.distance(pos) > 0.001)
                            .unwrap_or(true);
                        if needs {
                            let cube = make_cube(pos, 0.5, [0.2, 0.8, 0.2]);
                            let m = renderer.create_mesh(&cube.0, &cube.1);
                            self.entity_meshes.insert(key, (m, pos));
                        }
                        entity_keys.push(key);
                    }
                    for c in &snap.creatures {
                        let key = c.id + 1_000_000;
                        let pos = c.position + Vec3::new(0.0, 0.5, 0.0);
                        let needs = self
                            .entity_meshes
                            .get(&key)
                            .map(|(_, p)| p.distance(pos) > 0.001)
                            .unwrap_or(true);
                        if needs {
                            let cube = make_cube(pos, 0.5, [0.8, 0.2, 0.2]);
                            let m = renderer.create_mesh(&cube.0, &cube.1);
                            self.entity_meshes.insert(key, (m, pos));
                        }
                        entity_keys.push(key);
                    }
                    for l in &snap.loot {
                        let key = l.id + 2_000_000;
                        let pos = l.position;
                        let needs = self
                            .entity_meshes
                            .get(&key)
                            .map(|(_, p)| p.distance(pos) > 0.001)
                            .unwrap_or(true);
                        if needs {
                            let cube = make_cube(pos, 0.3, [1.0, 1.0, 0.2]);
                            let m = renderer.create_mesh(&cube.0, &cube.1);
                            self.entity_meshes.insert(key, (m, pos));
                        }
                        entity_keys.push(key);
                    }
                    for a in &snap.anomalies {
                        let key = a.id + 3_000_000;
                        let pos = a.position;
                        let needs = self
                            .entity_meshes
                            .get(&key)
                            .map(|(_, p)| p.distance(pos) > 0.001)
                            .unwrap_or(true);
                        if needs {
                            let cube = make_cube(pos, a.radius, [0.2, 0.2, 0.8]);
                            let m = renderer.create_mesh(&cube.0, &cube.1);
                            self.entity_meshes.insert(key, (m, pos));
                        }
                        entity_keys.push(key);
                    }
                }
                for key in &entity_keys {
                    keep_keys.insert(*key);
                }
                self.entity_meshes.retain(|k, _| keep_keys.contains(k));
                for key in &entity_keys {
                    if let Some((m, _)) = self.entity_meshes.get(key) {
                        render_list.push(m);
                    }
                }

                let render_items: Vec<(&Mesh, Option<&Texture>)> =
                    render_list.iter().map(|m| (*m, None)).collect();
                if let Some(r) = self.renderer.as_mut() {
                    let _ = r.render(&render_items, &self.camera);
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

fn make_cube(center: Vec3, half: f32, color: [f32; 3]) -> (Vec<Vertex>, Vec<u32>) {
    let mut verts = Vec::new();
    for &sx in &[-1.0f32, 1.0] {
        for &sy in &[-1.0f32, 1.0] {
            for &sz in &[-1.0f32, 1.0] {
                let n = Vec3::new(sx, sy, sz).normalize();
                verts.push(Vertex {
                    position: [
                        center.x + sx * half,
                        center.y + sy * half,
                        center.z + sz * half,
                    ],
                    normal: [n.x, n.y, n.z],
                    color,
                    uv: [(sx + 1.0) * 0.5, (sz + 1.0) * 0.5],
                });
            }
        }
    }
    let faces: [[usize; 4]; 6] = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let mut idx = Vec::new();
    for f in faces {
        idx.push(f[0] as u32);
        idx.push(f[1] as u32);
        idx.push(f[2] as u32);
        idx.push(f[0] as u32);
        idx.push(f[2] as u32);
        idx.push(f[3] as u32);
    }
    (verts, idx)
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

    env_logger::init();
    let cli = Cli::parse();
    if cli.telemetry {
        log::info!("telemetry: session_start");
    }

    let (out_tx, out_rx) = mpsc::channel::<ClientPacket>();
    let (in_tx, in_rx) = mpsc::channel::<ServerPacket>();

    // network thread
    let token = cli.token.clone();
    let username = cli
        .username
        .clone()
        .or_else(|| std::env::var("ZONA_USERNAME").ok())
        .unwrap_or_else(|| "guest".into());
    let password = cli
        .password
        .clone()
        .or_else(|| std::env::var("ZONA_PASSWORD").ok())
        .unwrap_or_else(|| "guest".into());
    let register = cli.register;
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
        if let Ok(data) = encode(&reg) {
            let _ = udp.send_to(&data, server_addr);
        }

        loop {
            // outbound
            while let Ok(pkt) = out_rx.try_recv() {
                if let Ok(data) = encode(&pkt) {
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

    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App {
        window: None,
        renderer: None,
        input: InputState::new(),
        camera: Camera {
            eye: Vec3::ZERO,
            target: Vec3::Z,
            up: Vec3::Y,
            fov_y: 1.0,
            aspect: 1.0,
            near: 0.1,
            far: 100.0,
        },
        meshes: Vec::new(),
        entity_meshes: std::collections::HashMap::new(),
        last_time: None,
        yaw: 0.0,
        pitch: 0.0,
        vertical_velocity: 0.0,
        cursor_captured: false,
        scene_path: cli.scene,
        net_out: out_tx,
        net_in: in_rx,
        player_id: None,
        latest_snapshot: None,
        seq: 0,
        local_pos: Vec3::new(0.0, 0.8, 0.0),
    };
    event_loop.run_app(&mut app).expect("run");
}
