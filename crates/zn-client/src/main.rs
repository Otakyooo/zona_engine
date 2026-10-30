use clap::Parser;
use glam::Vec3;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
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
}

struct App {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    input: InputState,
    camera: Camera,
    meshes: Vec<Mesh>,
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
                .as_ref()
                .and_then(|p| load_scene(std::path::Path::new(p)).ok());

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
                                        normal: n,
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
            WindowEvent::KeyboardInput { .. } => {
                self.input.handle_window_event(&event);
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
                                if self.local_pos.distance(server_pos) > 0.5 {
                                    self.local_pos = server_pos;
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
                let forward = Vec3::new(self.yaw.sin(), 0.0, self.yaw.cos());
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
                let mut remote_meshes: Vec<Mesh> = Vec::new();
                if let Some(snap) = &self.latest_snapshot
                    && let Some(pid) = self.player_id
                {
                    for ps in &snap.players {
                        if ps.id == pid {
                            continue;
                        }
                        let cube =
                            make_cube(ps.position + Vec3::new(0.0, 0.5, 0.0), 0.5, [0.2, 0.8, 0.2]);
                        remote_meshes.push(
                            self.renderer
                                .as_ref()
                                .unwrap()
                                .create_mesh(&cube.0, &cube.1),
                        );
                    }
                    for c in &snap.creatures {
                        let cube = make_cube(c.position + Vec3::new(0.0, 0.5, 0.0), 0.5, [0.8, 0.2, 0.2]);
                        remote_meshes.push(
                            self.renderer.as_ref().unwrap().create_mesh(&cube.0, &cube.1),
                        );
                    }
                    for l in &snap.loot {
                        let cube = make_cube(l.position, 0.3, [1.0, 1.0, 0.2]);
                        remote_meshes.push(
                            self.renderer.as_ref().unwrap().create_mesh(&cube.0, &cube.1),
                        );
                    }
                    for a in &snap.anomalies {
                        let cube = make_cube(a.position, a.radius, [0.2, 0.2, 0.8]);
                        remote_meshes.push(
                            self.renderer.as_ref().unwrap().create_mesh(&cube.0, &cube.1),
                        );
                    }
                }
                for m in &remote_meshes {
                    render_list.push(m);
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
    let username = cli.username.clone().unwrap_or_else(|| "guest".into());
    let password = cli.password.clone().unwrap_or_else(|| "guest".into());
    thread::spawn(move || {
        use std::io::{Read, Write};
        use std::net::UdpSocket;

        // TCP auth
        let mut stream = match std::net::TcpStream::connect("127.0.0.1:7777") {
            Ok(s) => s,
            Err(e) => {
                log::error!("connect failed: {}", e);
                return;
            }
        };
        stream.set_read_timeout(Some(Duration::from_millis(5))).ok();

        let mut got_token = token;
        if got_token.is_none() {
            let pkt = ClientPacket::Login { username, password };
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
        let server_addr: std::net::SocketAddr = "127.0.0.1:7778".parse().unwrap();

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
