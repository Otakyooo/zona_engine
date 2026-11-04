//! Экраны клиента: меню со схроном, смерть и инвентарь.
//!
//! HUD остаётся пассивным, а здесь есть кнопки — для них нужен `egui-winit`,
//! иначе клики уходили бы в стрельбу. Экран — это состояние, а не флаг: при
//! переходе в меню курсор освобождается, а игровой ввод не обрабатывается.

use zn_sim::{Inventory, InventoryItem, items};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Menu,
    Playing,
    Dead,
    /// Рейд закончился: показываем опыт, вынесенное или потерянное.
    Result,
}

/// Что выбрал игрок за текущий кадр.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    None,
    StartRaid,
    /// Вернуться в меню (схрон) со экрана смерти.
    ToMenu,
    Quit,
    /// Игрок выбрал оружие в меню; сервер подтвердит выбор `Loadout`.
    SetWeapon(u64),
    /// Забрать из схрона в рейдовый инвентарь.
    TakeItem(u64, u32),
}

fn panel_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(egui::Color32::from_black_alpha(205))
        .inner_margin(egui::Margin::same(18))
}

/// ПМ штатный, остальное оружие должно лежать в рейдовом инвентаре.
fn weapon_owned(id: u64, inventory: &Inventory) -> bool {
    id == items::PISTOL || inventory.has_item(id as i64, 1)
}

pub fn menu(
    ctx: &egui::Context,
    weapon_id: u64,
    protocol_ok: bool,
    inventory: &Inventory,
    stash: &[InventoryItem],
) -> Action {
    let mut action = Action::None;
    egui::Area::new(egui::Id::new("menu"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.set_width(420.0);
                ui.heading("ZONA");
                ui.label(
                    "WASD — движение, мышь — обзор, ЛКМ — выстрел, E — подобрать, \
                     1/2 — аптечка и бинт, Tab — инвентарь, Esc — курсор.",
                );
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label("Оружие:");
                    for (id, name) in [
                        (items::PISTOL, "PM"),
                        (items::SMG, "SMG"),
                        (items::RIFLE, "Rifle"),
                    ] {
                        let owned = weapon_owned(id, inventory);
                        let selected = weapon_id == id;
                        if ui
                            .add_enabled(owned, egui::SelectableLabel::new(selected, name))
                            .clicked()
                        {
                            action = Action::SetWeapon(id);
                        }
                    }
                });
                if !weapon_owned(weapon_id, inventory) {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 170, 90),
                        "Выбранного оружия нет в инвентаре — возьмите его из схрона.",
                    );
                }

                ui.add_space(8.0);
                ui.separator();
                ui.label("СХРОН");
                if stash.is_empty() {
                    ui.label("пусто — вынесите лут из рейда, чтобы он здесь появился");
                } else {
                    for it in stash {
                        // Отрицательный id — мусор из БД; в сеть его не шлём.
                        if it.item_id <= 0 {
                            continue;
                        }
                        ui.horizontal(|ui| {
                            ui.label(format!(
                                "{} x{}",
                                items::name(it.item_id as u64),
                                it.quantity
                            ));
                            if ui.small_button("Взять").clicked() {
                                action = Action::TakeItem(it.item_id as u64, 1);
                            }
                            if it.quantity > 1 && ui.small_button("Всё").clicked() {
                                action = Action::TakeItem(it.item_id as u64, it.quantity as u32);
                            }
                        });
                    }
                }

                if !protocol_ok {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 90, 90),
                        "Версия клиента не совпадает с сервером — обновите сборку.",
                    );
                }
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
                if ui.button("В меню (схрон)").clicked() {
                    action = Action::ToMenu;
                }
            });
        });
    action
}

/// Экран результата рейда.
pub fn result(ctx: &egui::Context, result: &zn_net::RaidResult) -> Action {
    let mut action = Action::None;
    let extracted = result.reason == zn_net::RaidEndReason::Extracted;
    egui::Area::new(egui::Id::new("raid_result"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.set_width(360.0);
                ui.heading(if extracted {
                    "Эвакуация успешна"
                } else {
                    "Вы погибли"
                });
                if !extracted {
                    ui.label("Снаряжение сохранено. Лут и расходники потеряны.");
                }
                ui.label(format!("Опыт за рейд: +{}", result.xp_gained));
                ui.label(format!(
                    "Время: {}:{:02}",
                    result.duration_secs / 60,
                    result.duration_secs % 60
                ));
                ui.add_space(6.0);
                ui.separator();
                ui.label(if extracted {
                    "ВЫНЕСЕНО В СХРОН"
                } else {
                    "ПОТЕРЯНО НА МЕСТЕ СМЕРТИ"
                });
                if result.items.is_empty() {
                    ui.label("пусто");
                } else {
                    let mut rows: Vec<String> = result
                        .items
                        .iter()
                        .map(|i| {
                            format!("{} x{}", items::name(i.item_id.max(0) as u64), i.quantity)
                        })
                        .collect();
                    rows.sort();
                    for row in rows {
                        ui.label(row);
                    }
                }
                ui.add_space(10.0);
                if ui.button("В рейд").clicked() {
                    action = Action::StartRaid;
                }
                if ui.button("В меню (схрон)").clicked() {
                    action = Action::ToMenu;
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
