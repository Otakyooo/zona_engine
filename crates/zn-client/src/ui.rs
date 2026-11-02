//! Экраны клиента: меню, смерть и инвентарь.
//!
//! HUD остаётся пассивным, а здесь есть кнопки — для них нужен `egui-winit`,
//! иначе клики уходили бы в стрельбу. Экран — это состояние, а не флаг: при
//! переходе в меню курсор освобождается, а игровой ввод не обрабатывается.

use zn_sim::Inventory;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Menu,
    Playing,
    Dead,
}

/// Что выбрал игрок за текущий кадр.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    StartRaid,
    Quit,
}

fn panel_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(egui::Color32::from_black_alpha(205))
        .inner_margin(egui::Margin::same(18))
}

pub fn menu(ctx: &egui::Context) -> Action {
    let mut action = Action::None;
    egui::Area::new(egui::Id::new("menu"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.set_width(340.0);
                ui.heading("ZONA");
                ui.label(
                    "WASD — движение, мышь — обзор, ЛКМ — выстрел, E — подобрать, \
                     1/2 — аптечка и бинт, Tab — инвентарь, Esc — курсор.",
                );
                ui.add_space(10.0);
                if ui.button("В рейд").clicked() {
                    action = Action::StartRaid;
                }
                if ui.button("Выход").clicked() {
                    action = Action::Quit;
                }
            });
        });
    action
}

pub fn dead(ctx: &egui::Context, xp: i64, level: i32) -> Action {
    let mut action = Action::None;
    egui::Area::new(egui::Id::new("dead"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.set_width(320.0);
                ui.heading("Вы погибли");
                ui.label(format!("Уровень {level}, опыт {xp}"));
                ui.label("Инвентарь остался на месте смерти.");
                ui.add_space(10.0);
                if ui.button("В рейд").clicked() {
                    action = Action::StartRaid;
                }
            });
        });
    action
}

/// Панель инвентаря. Экипировку не показываем: сервер её не передаёт, а
/// подпись без данных хуже её отсутствия.
pub fn inventory(ctx: &egui::Context, inv: &Inventory) {
    egui::Area::new(egui::Id::new("inventory_panel"))
        .anchor(egui::Align2::RIGHT_CENTER, egui::vec2(-16.0, 0.0))
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.set_width(200.0);
                ui.label("ИНВЕНТАРЬ");
                if inv.items.is_empty() {
                    ui.label("пусто");
                }
                let mut rows: Vec<String> = inv
                    .items
                    .iter()
                    .map(|i| format!("{} x{}", crate::hud::item_name(i.item_id), i.quantity))
                    .collect();
                rows.sort();
                for row in rows {
                    ui.label(row);
                }
            });
        });
}
