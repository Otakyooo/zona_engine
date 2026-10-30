use eframe::egui;
use zn_scene::{SceneFile, SceneNodeFile, Transform, load_scene, save_scene};

struct EditorApp {
    nodes: Vec<SceneNodeFile>,
    selected: Option<usize>,
    scene_path: String,
    status: String,
}

impl EditorApp {
    fn new() -> Self {
        Self {
            nodes: Vec::new(),
            selected: None,
            scene_path: "scene.ron".to_string(),
            status: String::new(),
        }
    }
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Add Cube").clicked() {
                    self.nodes.push(SceneNodeFile {
                        name: format!("cube_{}", self.nodes.len()),
                        transform: Transform::default(),
                        mesh: None,
                        color: [0.8, 0.4, 0.2],
                    });
                }
                if ui.button("Save").clicked() {
                    let scene = SceneFile {
                        nodes: self.nodes.clone(),
                    };
                    match save_scene(std::path::Path::new(&self.scene_path), &scene) {
                        Ok(_) => self.status = format!("saved {}", self.scene_path),
                        Err(e) => self.status = format!("save err: {}", e),
                    }
                }
                if ui.button("Load").clicked() {
                    match load_scene(std::path::Path::new(&self.scene_path)) {
                        Ok(s) => {
                            self.nodes = s.nodes;
                            self.status = format!("loaded {}", self.scene_path);
                        }
                        Err(e) => self.status = format!("load err: {}", e),
                    }
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
            for (i, n) in self.nodes.iter().enumerate() {
                let sel = self.selected == Some(i);
                if ui.selectable_label(sel, &n.name).clicked() {
                    self.selected = Some(i);
                }
            }
        });
        egui::SidePanel::right("inspector").show(ctx, |ui| {
            ui.label("Inspector");
            if let Some(i) = self.selected {
                if let Some(node) = self.nodes.get_mut(i) {
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
