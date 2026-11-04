use eframe::egui;
use glam::Vec3;
use zn_content::{PlayerSpawn, ZoneMap};
use zn_scene::{SceneNodeFile, Transform};
use zn_sim::{CreatureSpawn, ExtractionZone, Item, LootItem, ids};

struct EditorApp {
    /// Редактор держит карту целиком, а не только `nodes`: сохранение
    /// `SceneFile` молча стирало бы спавны, лут и аномалии из файла.
    zone: ZoneMap,
    selected: Option<usize>,
    scene_path: String,
    status: String,
}

impl EditorApp {
    fn new() -> Self {
        Self {
            zone: ZoneMap::default(),
            selected: None,
            scene_path: "maps/arena.ron".to_string(),
            status: String::new(),
        }
    }

    fn save(&mut self) {
        let problems = self.zone.validate();
        if !problems.is_empty() {
            let first = problems
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join("; ");
            self.status = format!("не сохранено, карта невалидна: {first}");
            return;
        }
        match self.zone.save(std::path::Path::new(&self.scene_path)) {
            Ok(_) => self.status = format!("saved {}", self.scene_path),
            Err(e) => self.status = format!("save err: {e}"),
        }
    }

    fn load(&mut self) {
        match ZoneMap::load(std::path::Path::new(&self.scene_path)) {
            Ok(zone) => {
                self.zone = zone;
                self.selected = None;
                self.status = format!("loaded {}", self.scene_path);
            }
            Err(e) => self.status = format!("load err: {e}"),
        }
    }

    /// Точка в центре карты — заведомо внутри границ.
    fn centre(&self) -> Vec3 {
        let b = &self.zone.bounds;
        Vec3::new((b.min.x + b.max.x) * 0.5, 0.8, (b.min.z + b.max.z) * 0.5)
    }

    fn next_creature_id(&self) -> u64 {
        self.zone
            .creature_spawns
            .iter()
            .map(|c| c.id)
            .max()
            .unwrap_or(ids::CREATURE_BASE)
            + 1
    }

    fn next_loot_id(&self) -> u64 {
        self.zone
            .loot
            .iter()
            .map(|l| l.id)
            .max()
            .unwrap_or(ids::LOOT_BASE)
            + 1
    }

    fn next_extraction_id(&self) -> u64 {
        self.zone
            .extractions
            .iter()
            .map(|z| z.id)
            .max()
            .unwrap_or(0)
            + 1
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Add Cube").clicked() {
                    self.zone.nodes.push(SceneNodeFile {
                        name: format!("cube_{}", self.zone.nodes.len()),
                        transform: Transform::default(),
                        mesh: None,
                        color: [0.8, 0.4, 0.2],
                        texture: Some("concrete".into()),
                        uv_scale: 1.0,
                    });
                }
                // Наполнение карты — часть контент-пайплайна: без этого
                // спавны и лут приходилось бы писать в RON руками.
                if ui.button("Add Creature").clicked() {
                    let id = self.next_creature_id();
                    self.zone.creature_spawns.push(CreatureSpawn {
                        id,
                        position: self.centre(),
                        kind: "snork".into(),
                        health: 40.0,
                        damage: 5.0,
                        respawn_secs: 30.0,
                    });
                    self.status = format!("added creature {id}");
                }
                if ui.button("Add Loot").clicked() {
                    let id = self.next_loot_id();
                    let mut position = self.centre();
                    position.y = 0.5;
                    self.zone.loot.push(LootItem {
                        id,
                        position,
                        item: Item::from_catalog(zn_sim::items::MEDKIT),
                    });
                    self.status = format!("added loot {id}");
                }
                if ui.button("Add Extraction").clicked() {
                    let id = self.next_extraction_id();
                    self.zone.extractions.push(ExtractionZone {
                        id,
                        position: self.centre(),
                        radius: 2.0,
                    });
                    self.status = format!("added extraction {id}");
                }
                if ui.button("Add Player Spawn").clicked() {
                    let position = self.centre();
                    self.zone.players.push(PlayerSpawn {
                        position: [position.x, position.y, position.z],
                        yaw: 0.0,
                    });
                    self.status = "added player spawn".into();
                }
                if ui.button("Save").clicked() {
                    self.save();
                }
                if ui.button("Load").clicked() {
                    self.load();
                }
                if ui.button("Play").clicked() {
                    let _ = std::process::Command::new("cargo")
                        .args(["run", "-p", "zn-client", "--", "--scene", &self.scene_path])
                        .spawn();
                    self.status = "launched client".into();
                }
                ui.label(&self.status);
            });
        });
        egui::SidePanel::left("hierarchy").show(ctx, |ui| {
            ui.label("Hierarchy");
            for (i, n) in self.zone.nodes.iter().enumerate() {
                let sel = self.selected == Some(i);
                if ui.selectable_label(sel, &n.name).clicked() {
                    self.selected = Some(i);
                }
            }
            ui.separator();
            ui.label(format!(
                "спавны: {} игрок / {} существ",
                self.zone.players.len(),
                self.zone.creature_spawns.len()
            ));
            ui.label(format!(
                "лут: {}, аномалии: {}, эвакуация: {}",
                self.zone.loot.len(),
                self.zone.anomalies.len(),
                self.zone.extractions.len()
            ));
            ui.separator();
            ui.label("bounds");
            vec3_row(ui, "min", &mut self.zone.bounds.min);
            vec3_row(ui, "max", &mut self.zone.bounds.max);
        });
        egui::SidePanel::right("inspector").show(ctx, |ui| {
            ui.label("Inspector");
            if let Some(i) = self.selected {
                if let Some(node) = self.zone.nodes.get_mut(i) {
                    ui.label(&node.name);
                    ui.horizontal(|ui| {
                        ui.label("x");
                        ui.add(egui::DragValue::new(&mut node.transform.position.x));
                    });
                    ui.horizontal(|ui| {
                        ui.label("y");
                        ui.add(egui::DragValue::new(&mut node.transform.position.y));
                    });
                    ui.horizontal(|ui| {
                        ui.label("z");
                        ui.add(egui::DragValue::new(&mut node.transform.position.z));
                    });
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label("material");
                        let mut name = node.texture.clone().unwrap_or_default();
                        if ui.text_edit_singleline(&mut name).changed() {
                            let trimmed = name.trim();
                            node.texture = if trimmed.is_empty() {
                                None
                            } else {
                                Some(trimmed.to_string())
                            };
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("tile m");
                        ui.add(
                            egui::DragValue::new(&mut node.uv_scale)
                                .speed(0.1)
                                .range(0.1..=64.0),
                        );
                    });
                }
            } else {
                ui.label("no selection");
            }
        });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Viewport (preview not wired in M2)");
            ui.text_edit_singleline(&mut self.scene_path);
        });
    }
}

fn vec3_row(ui: &mut egui::Ui, label: &str, v: &mut Vec3) {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.add(egui::DragValue::new(&mut v.x).speed(0.5).prefix("x "));
        ui.add(egui::DragValue::new(&mut v.y).speed(0.5).prefix("y "));
        ui.add(egui::DragValue::new(&mut v.z).speed(0.5).prefix("z "));
    });
}

fn main() {
    env_logger::init();
    let options = eframe::NativeOptions::default();
    let _ = eframe::run_native(
        "ZONA Editor",
        options,
        Box::new(|_cc| Ok(Box::new(EditorApp::new()))),
    );
}
