//! Пассивный HUD: прицел, витальные, прогресс, инвентарь и сообщения.
//!
//! Виджеты не интерактивны — ввод в egui не транслируется, поэтому
//! `egui-winit` не нужен. Если появятся кнопки, его придётся подключить.

use std::time::Instant;

/// Сообщение и момент его появления.
pub type Toast = (String, Instant);

/// Сколько живёт сообщение, секунды.
pub const TOAST_TTL: f32 = 5.0;

/// Длительность экстракции на сервере, секунды.
const EXTRACT_SECONDS: f32 = 10.0;

#[derive(Default)]
pub struct HudState {
    pub health: f32,
    pub radiation: f32,
    pub hunger: f32,
    pub thirst: f32,
    pub level: i32,
    pub xp: i64,
    pub inventory: String,
    pub extract: f32,
    pub fps: f32,
    /// Оружие, патроны и задержка канала: без них игрок не понимает, почему
    /// выстрел не произошёл или цель «не пробилась».
    pub weapon: String,
    pub ammo: u32,
    pub rtt_ms: f32,
    pub toasts: Vec<Toast>,
}

pub fn draw(ctx: &egui::Context, s: &HudState, now: Instant) {
    crosshair(ctx);
    vitals(ctx, s);
    stats(ctx, s);
    inventory(ctx, s);
    extraction(ctx, s);
    toasts(ctx, s, now);
}

/// Прицел строится от `screen_rect().center()` в точках, поэтому остаётся
/// по центру при любом DPI и размере окна.
fn crosshair(ctx: &egui::Context) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("crosshair"),
    ));
    let c = ctx.screen_rect().center();
    let outline = egui::Stroke::new(2.5_f32, egui::Color32::from_black_alpha(120));
    let white = egui::Stroke::new(1.5_f32, egui::Color32::from_white_alpha(210));

    for (dx, dy) in [(1.0f32, 0.0f32), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
        let a = c + egui::vec2(dx * 4.0, dy * 4.0);
        let b = c + egui::vec2(dx * 10.0, dy * 10.0);
        painter.line_segment([a, b], outline);
        painter.line_segment([a, b], white);
    }
    painter.circle_filled(c, 1.5, egui::Color32::from_white_alpha(210));
}

fn panel_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(egui::Color32::from_black_alpha(90))
        .inner_margin(egui::Margin::same(6))
}

fn vitals(ctx: &egui::Context, s: &HudState) {
    egui::Area::new(egui::Id::new("vitals"))
        .fixed_pos(egui::pos2(16.0, 16.0))
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.set_width(190.0);
                bar(ui, "HP", s.health, egui::Color32::from_rgb(200, 60, 60));
                bar(
                    ui,
                    "RAD",
                    s.radiation,
                    egui::Color32::from_rgb(120, 200, 90),
                );
                bar(ui, "FOOD", s.hunger, egui::Color32::from_rgb(200, 170, 80));
                bar(ui, "WATER", s.thirst, egui::Color32::from_rgb(90, 160, 220));
            });
        });
}

fn bar(ui: &mut egui::Ui, label: &str, value: f32, color: egui::Color32) {
    let frac = (value / 100.0).clamp(0.0, 1.0);
    ui.add(
        egui::ProgressBar::new(frac)
            .desired_width(190.0)
            .fill(color)
            .text(format!("{label} {value:.0}")),
    );
}

fn stats(ctx: &egui::Context, s: &HudState) {
    egui::Area::new(egui::Id::new("stats"))
        .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-16.0, 16.0))
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.label(format!(
                    "LV {}   xp {}/1000",
                    s.level,
                    s.xp.rem_euclid(1000)
                ));
                ui.label(format!("{}   {:.0} FPS   {} ms", s.weapon, s.fps, s.rtt_ms));
                ui.label(format!("патроны: {}", s.ammo));
            });
        });
}

fn inventory(ctx: &egui::Context, s: &HudState) {
    if s.inventory.is_empty() {
        return;
    }
    egui::Area::new(egui::Id::new("inventory"))
        .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(16.0, -16.0))
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.label(&s.inventory);
            });
        });
}

fn extraction(ctx: &egui::Context, s: &HudState) {
    if s.extract <= 0.0 {
        return;
    }
    egui::Area::new(egui::Id::new("extraction"))
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -48.0))
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.add(
                    egui::ProgressBar::new(s.extract)
                        .desired_width(240.0)
                        .text("EXTRACT"),
                );
            });
        });
}

fn toasts(ctx: &egui::Context, s: &HudState, now: Instant) {
    let mut offset = -96.0;
    for (text, at) in &s.toasts {
        let age = now.duration_since(*at).as_secs_f32();
        if age > TOAST_TTL {
            continue;
        }
        let alpha = ((TOAST_TTL - age) / 1.0).clamp(0.0, 1.0);
        egui::Area::new(egui::Id::new(("toast", at)))
            .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, offset))
            .show(ctx, |ui| {
                panel_frame().show(ui, |ui| {
                    ui.colored_label(egui::Color32::from_white_alpha((alpha * 255.0) as u8), text);
                });
            });
        offset -= 24.0;
    }
}

/// Прогресс экстракции по локальному времени в зоне.
///
/// Приближение: серверный таймер клиенту не передаётся.
pub fn extraction_progress(elapsed: f32) -> f32 {
    (elapsed / EXTRACT_SECONDS).clamp(0.0, 1.0)
}

/// Человекочитаемая строка инвентаря; пустой инвентарь даёт пустую строку.
pub fn format_inventory(inv: &zn_sim::Inventory) -> String {
    if inv.items.is_empty() {
        return String::new();
    }
    let mut parts: Vec<String> = inv
        .items
        .iter()
        .map(|i| format!("{} x{}", item_name(i.item_id), i.quantity))
        .collect();
    parts.sort();
    format!("INV: {}", parts.join("   "))
}

/// Имя предмета по id. Общее для HUD и панели инвентаря.
///
/// Обычные предметы берутся из каталога симуляции: HUD больше не держит
/// второй список имён, который расходился с сервером.
pub(crate) fn item_name(id: i64) -> String {
    if id >= zn_sim::ids::ARTIFACT_BASE as i64 {
        return format!("Artifact {}", id - zn_sim::ids::ARTIFACT_BASE as i64);
    }
    zn_sim::items::name(id.max(0) as u64).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zn_sim::{Inventory, InventoryItem};

    #[test]
    fn inventory_text_lists_stacks() {
        let mut inv = Inventory::default();
        assert_eq!(format_inventory(&inv), "");

        inv.add_item(1, 2);
        inv.add_item(2, 1);
        let text = format_inventory(&inv);
        assert!(text.starts_with("INV: "));
        assert!(text.contains("Medkit x2"), "{text}");
        assert!(text.contains("Bandage x1"), "{text}");
    }

    #[test]
    fn artifact_gets_its_own_name() {
        let inv = Inventory {
            items: vec![InventoryItem {
                item_id: zn_sim::ids::ARTIFACT_BASE as i64 + 3,
                quantity: 1,
                slot: 0,
            }],
        };
        assert!(format_inventory(&inv).contains("Artifact 3"));
    }

    #[test]
    fn extraction_progress_clamps() {
        assert_eq!(extraction_progress(0.0), 0.0);
        assert!((extraction_progress(5.0) - 0.5).abs() < 1e-6);
        assert_eq!(extraction_progress(10.0), 1.0);
        assert_eq!(extraction_progress(99.0), 1.0);
    }
}
