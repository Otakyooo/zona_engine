use eframe::egui;
use zn_content::ZoneMap;
use zn_scene::{SceneNodeFile, Transform};

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
            ui.label(format!(
                "bounds: {:?} .. {:?}",
                self.zone.bounds.min, self.zone.bounds.max
            ));
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

fn main() {
    env_logger::init();
    let options = eframe::NativeOptions::default();
    let _ = eframe::run_native(
        "ZONA Editor",
        options,
        Box::new(|_cc| Ok(Box::new(EditorApp::new()))),
    );
}
