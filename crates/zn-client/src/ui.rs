//! Экраны клиента: меню со схроном, пауза, настройки и сеточный инвентарь.
//!
//! HUD остаётся пассивным, а здесь есть кнопки — для них нужен `egui-winit`,
//! иначе клики уходили бы в стрельбу. Сетка рисуется вручную: клетки,
//! поворот и перенос between контейнерами не выразить стандартным `Grid`.

use crate::settings::Settings;
use zn_sim::{
    Inventory, InventoryItem, ItemContainer, RAID_GRID_H, RAID_GRID_W, STASH_GRID_H, STASH_GRID_W,
    can_place, find_spot, items,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Menu,
    Settings,
    Playing,
    /// Сеточный инвентарь поверх мира; курсор свободен, стрельба заблокирована.
    Inventory,
    Paused,
    Dead,
    /// Рейд закончился: показываем опыт, вынесенное или потерянное.
    Result,
}

/// Что выбрал игрок за текущий кадр.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    None,
    StartRaid,
    /// Вернуться в меню (схрон) со экрана смерти.
    ToMenu,
    Quit,
    Resume,
    OpenInventory,
    CloseInventory,
    OpenSettings,
    CloseSettings,
    ApplySettings(Settings),
    /// Игрок выбрал оружие; сервер подтвердит выбор `Loadout`.
    SetWeapon(u64),
    /// Использовать расходник в рейде.
    ConsumeItem(u64),
    /// Перенос предмета. Координаты проверяет сервер.
    MoveItem {
        from: ItemContainer,
        item_id: u64,
        to: ItemContainer,
        x: i32,
        y: i32,
        rotated: bool,
    },
}

#[derive(Clone, Copy, Debug)]
struct Selection {
    from: ItemContainer,
    item_id: u64,
}

#[derive(Clone, Copy, Debug)]
struct DragState {
    from: ItemContainer,
    item_id: u64,
    /// Смещение курсора от левого верхнего угла предмета в пикселях.
    grab: egui::Vec2,
    rotated: bool,
}

/// Состояние сетки между кадрами. Хранится в `App`, а не в egui temp data:
/// при смене экрана перетаскивание не должно теряться.
#[derive(Default)]
pub struct InventoryUiState {
    selected: Option<Selection>,
    drag: Option<DragState>,
}

const CELL: f32 = 34.0;

fn panel_frame() -> egui::Frame {
    egui::Frame::NONE
        .fill(egui::Color32::from_black_alpha(215))
        .inner_margin(egui::Margin::same(18))
}

/// ПМ штатный, остальное оружие должно лежать в рейдовом инвентаре.
fn weapon_owned(id: u64, inventory: &Inventory) -> bool {
    id == items::PISTOL || inventory.has_item(id as i64, 1)
}

fn item_name(id: i64) -> String {
    if id >= zn_sim::ids::ARTIFACT_BASE as i64 {
        return format!("Artifact {}", id - zn_sim::ids::ARTIFACT_BASE as i64);
    }
    items::name(id.max(0) as u64).to_string()
}

fn item_color(id: i64) -> egui::Color32 {
    if id >= zn_sim::ids::ARTIFACT_BASE as i64 {
        return egui::Color32::from_rgb(150, 95, 190);
    }
    if items::is_weapon(id.max(0) as u64) {
        egui::Color32::from_rgb(185, 115, 55)
    } else if id as u64 == items::LIGHT_ARMOR {
        egui::Color32::from_rgb(65, 105, 155)
    } else if items::heal(id.max(0) as u64) > 0.0 {
        egui::Color32::from_rgb(70, 135, 80)
    } else if id as u64 == items::AMMO {
        egui::Color32::from_rgb(120, 120, 105)
    } else {
        egui::Color32::from_rgb(115, 92, 65)
    }
}

fn short_name(name: &str) -> String {
    let mut out: String = name.chars().take(4).collect();
    if name.chars().count() > 4 {
        out.push('…');
    }
    out
}

fn totals(items: &[InventoryItem]) -> (f32, u32) {
    items.iter().fold((0.0, 0), |(w, v), it| {
        let id = it.item_id.max(0) as u64;
        (
            w + items::weight(id) * it.quantity.max(0) as f32,
            v + items::value(id) * it.quantity.max(1) as u32,
        )
    })
}

fn used_cells(items: &[InventoryItem]) -> i32 {
    items
        .iter()
        .filter(|i| i.item_id > 0)
        .map(|i| {
            let (w, h) = i.cells();
            w * h
        })
        .sum()
}

fn item_at(items: &[InventoryItem], x: i32, y: i32) -> Option<&InventoryItem> {
    items.iter().find(|it| {
        let (w, h) = it.cells();
        x >= it.x && x < it.x + w && y >= it.y && y < it.y + h
    })
}

fn find_item(items: &[InventoryItem], id: i64) -> Option<InventoryItem> {
    items.iter().find(|i| i.item_id == id).cloned()
}

pub fn menu(
    ctx: &egui::Context,
    weapon_id: u64,
    protocol_ok: bool,
    inventory: &Inventory,
    stash: &[InventoryItem],
    state: &mut InventoryUiState,
) -> Action {
    let mut action = Action::None;
    egui::Area::new(egui::Id::new("menu"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.set_width(900.0);
                ui.horizontal(|ui| {
                    ui.heading("ZONA");
                    ui.label(" / схрон");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Выход").clicked() {
                            action = Action::Quit;
                        }
                        if ui.button("Настройки").clicked() {
                            action = Action::OpenSettings;
                        }
                    });
                });
                ui.label(
                    "WASD — движение, мышь — обзор, ЛКМ — выстрел, E — подобрать, \
                     Tab — инвентарь, Esc — пауза.",
                );
                ui.add_space(6.0);

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
                    ui.separator();
                    ui.label(format!("В руках: {}", item_name(weapon_id as i64)));
                });
                if !weapon_owned(weapon_id, inventory) {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 170, 90),
                        "Выбранного оружия нет в инвентаре — возьмите его из схрона.",
                    );
                }
                ui.separator();

                let grid_action = containers(ui, inventory, stash, weapon_id, state, false);
                if grid_action != Action::None {
                    action = grid_action;
                }

                if !protocol_ok {
                    ui.colored_label(
                        egui::Color32::from_rgb(230, 90, 90),
                        "Версия клиента не совпадает с сервером — обновите сборку.",
                    );
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("В рейд").clicked() {
                        action = Action::StartRaid;
                    }
                    ui.label("Tab — открыть инвентарь в рейде, Esc — пауза.");
                });
            });
        });
    action
}

/// Инвентарь в рейде: те же контейнеры, но поверх мира и с кнопкой закрытия.
pub fn inventory(
    ctx: &egui::Context,
    inventory: &Inventory,
    stash: &[InventoryItem],
    weapon_id: u64,
    state: &mut InventoryUiState,
) -> Action {
    let mut action = Action::None;
    egui::Area::new(egui::Id::new("inventory_panel"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.set_width(900.0);
                ui.horizontal(|ui| {
                    ui.heading("ИНВЕНТАРЬ");
                    ui.label("Tab / Esc — закрыть · R — повернуть предмет при переносе");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Закрыть").clicked() {
                            action = Action::CloseInventory;
                        }
                    });
                });
                ui.separator();
                let grid_action = containers(ui, inventory, stash, weapon_id, state, true);
                if grid_action != Action::None {
                    action = grid_action;
                }
            });
        });
    action
}

pub fn paused(ctx: &egui::Context) -> Action {
    let mut action = Action::None;
    egui::Area::new(egui::Id::new("pause"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.set_width(300.0);
                ui.heading("ПАУЗА");
                ui.label("Рейд продолжается на сервере.");
                ui.add_space(10.0);
                if ui.button("Продолжить").clicked() {
                    action = Action::Resume;
                }
                if ui.button("Инвентарь").clicked() {
                    action = Action::OpenInventory;
                }
                if ui.button("Настройки").clicked() {
                    action = Action::OpenSettings;
                }
                ui.separator();
                if ui.button("Выход из игры").clicked() {
                    action = Action::Quit;
                }
            });
        });
    action
}

pub fn settings(ctx: &egui::Context, settings: &Settings, back_to: Screen) -> Action {
    let mut action = Action::None;
    let mut draft = *settings;
    let mut changed = false;
    egui::Area::new(egui::Id::new("settings"))
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            panel_frame().show(ui, |ui| {
                ui.set_width(430.0);
                ui.heading("НАСТРОЙКИ");
                ui.add_space(6.0);
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut draft.mouse_sensitivity,
                            crate::settings::SENSITIVITY_MIN..=crate::settings::SENSITIVITY_MAX,
                        )
                        .text("Чувствительность мыши"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut draft.fov_deg,
                            crate::settings::FOV_MIN..=crate::settings::FOV_MAX,
                        )
                        .text("Угол обзора, °"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(
                            &mut draft.render_distance,
                            crate::settings::RENDER_DISTANCE_MIN
                                ..=crate::settings::RENDER_DISTANCE_MAX,
                        )
                        .text("Дальность отрисовки, м"),
                    )
                    .changed();
                changed |= ui
                    .add(
                        egui::Slider::new(&mut draft.master_volume, 0.0..=1.0)
                            .text("Громкость")
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
                    )
                    .changed();
                changed |= ui
                    .checkbox(&mut draft.vsync, "Вертикальная синхронизация")
                    .changed();
                changed |= ui
                    .checkbox(&mut draft.invert_y, "Инвертировать мышь по Y")
                    .changed();
                ui.add_space(6.0);
                ui.label(
                    "FOV и дальность применяются к камере сразу; V-Sync переключает \
                     режим презентации поверхности.",
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Сбросить").clicked() {
                        action = Action::ApplySettings(Settings::default());
                    }
                    if ui.button("Назад").clicked() {
                        action = Action::CloseSettings;
                    }
                    let back = match back_to {
                        Screen::Playing | Screen::Inventory | Screen::Paused => "в игру",
                        _ => "в меню",
                    };
                    ui.label(format!("(назад {back})"));
                });
            });
        });
    if action == Action::None && changed {
        action = Action::ApplySettings(draft.clamped());
    }
    action
}

/// Рисует экипировку и оба контейнера. Возвращает действие перетаскивания.
fn containers(
    ui: &mut egui::Ui,
    inventory: &Inventory,
    stash: &[InventoryItem],
    weapon_id: u64,
    state: &mut InventoryUiState,
    in_raid: bool,
) -> Action {
    let mut action = Action::None;

    // Слот оружия — цель для перетаскивания оружия из любой сетки.
    let (weapon_rect, _) = ui.allocate_exact_size(egui::vec2(260.0, 38.0), egui::Sense::hover());
    let weapon_bg = if state.drag.is_some_and(|d| items::is_weapon(d.item_id)) {
        egui::Color32::from_rgb(55, 90, 55)
    } else {
        egui::Color32::from_rgb(35, 35, 32)
    };
    ui.painter().rect_filled(weapon_rect, 4.0, weapon_bg);
    ui.painter().rect_stroke(
        weapon_rect,
        4.0,
        egui::Stroke::new(1.0_f32, egui::Color32::from_gray(100)),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        weapon_rect.left_top() + egui::vec2(8.0, 6.0),
        egui::Align2::LEFT_TOP,
        "СЛОТ ОРУЖИЯ",
        egui::FontId::proportional(10.0),
        egui::Color32::from_gray(170),
    );
    ui.painter().text(
        weapon_rect.left_top() + egui::vec2(8.0, 20.0),
        egui::Align2::LEFT_TOP,
        item_name(weapon_id as i64),
        egui::FontId::proportional(14.0),
        egui::Color32::WHITE,
    );

    ui.add_space(4.0);
    let mut stash_grid = None;
    let mut inv_grid = None;
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            let (w, v) = totals(stash);
            ui.label(format!(
                "СХРОН  {}/{} клеток · {:.1} кг · {} ценность",
                used_cells(stash),
                STASH_GRID_W * STASH_GRID_H,
                w,
                v
            ));
            stash_grid = Some(draw_grid(
                ui,
                ItemContainer::Stash,
                stash,
                STASH_GRID_W,
                STASH_GRID_H,
                state,
            ));
        });
        ui.add_space(18.0);
        ui.vertical(|ui| {
            let (w, v) = totals(&inventory.items);
            ui.label(format!(
                "РЮКЗАК  {}/{} клеток · {:.1} кг · {} ценность",
                used_cells(&inventory.items),
                Inventory::capacity_cells(),
                w,
                v
            ));
            inv_grid = Some(draw_grid(
                ui,
                ItemContainer::Inventory,
                &inventory.items,
                RAID_GRID_W,
                RAID_GRID_H,
                state,
            ));
        });
    });

    // R обрабатывается один раз за кадр: если делать это в draw_grid, обе
    // сетки переключат флаг и поворот останется прежним.
    if state.drag.is_some()
        && ui.input(|i| i.key_pressed(egui::Key::R))
        && let Some(drag) = state.drag.as_mut()
    {
        drag.rotated = !drag.rotated;
    }

    // Выбор и быстрые действия: drag удобен мышью, кнопки — для клавиатуры
    // и для случая, когда предмет нужно положить в первую свободную клетку.
    let selected = state.selected;
    if let Some(sel) = selected {
        let source = match sel.from {
            ItemContainer::Inventory => &inventory.items,
            ItemContainer::Stash => stash,
        };
        if let Some(item) = find_item(source, sel.item_id as i64) {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(format!(
                    "{} · {} x{} · {} клеток",
                    item_name(item.item_id),
                    if item.rotated {
                        "повёрнут"
                    } else {
                        "не повёрнут"
                    },
                    item.quantity,
                    {
                        let (w, h) = item.cells();
                        format!("{w}×{h}")
                    }
                ));
                let destination = match sel.from {
                    ItemContainer::Inventory => ItemContainer::Stash,
                    ItemContainer::Stash => ItemContainer::Inventory,
                };
                let dest_items = match destination {
                    ItemContainer::Inventory => &inventory.items,
                    ItemContainer::Stash => stash,
                };
                let auto = if dest_items.iter().any(|i| i.item_id == item.item_id) {
                    Some((0, 0, false))
                } else {
                    find_spot(
                        dest_items,
                        item.item_id,
                        match destination {
                            ItemContainer::Inventory => RAID_GRID_W,
                            ItemContainer::Stash => STASH_GRID_W,
                        },
                        match destination {
                            ItemContainer::Inventory => RAID_GRID_H,
                            ItemContainer::Stash => STASH_GRID_H,
                        },
                    )
                };
                let label = match destination {
                    ItemContainer::Inventory => "В рюкзак",
                    ItemContainer::Stash => "В схрон",
                };
                if ui
                    .add_enabled(auto.is_some(), egui::Button::new(label))
                    .clicked()
                    && let Some((x, y, rotated)) = auto
                {
                    action = Action::MoveItem {
                        from: sel.from,
                        item_id: item.item_id.max(0) as u64,
                        to: destination,
                        x,
                        y,
                        rotated,
                    };
                }
                if sel.from == ItemContainer::Inventory
                    && items::is_weapon(item.item_id.max(0) as u64)
                    && ui.button("В руки").clicked()
                {
                    action = Action::SetWeapon(item.item_id.max(0) as u64);
                }
                if items::heal(item.item_id.max(0) as u64) > 0.0
                    && in_raid
                    && ui.button("Использовать").clicked()
                {
                    // Consume идёт отдельным путём: это не перенос, а эффект.
                    action = Action::ConsumeItem(item.item_id.max(0) as u64);
                }
                if ui.small_button("Снять выбор").clicked() {
                    state.selected = None;
                }
            });
        } else {
            state.selected = None;
        }
    }

    // Отпускание мыши разрешаем после обеих сеток: предмет мог уехать
    // из рюкзака в схрон, и координаты цели считаются по её прямоугольнику.
    if state.drag.is_some() && ui.input(|i| i.pointer.any_released()) {
        let pointer = ui.input(|i| i.pointer.interact_pos()).unwrap_or_default();
        if weapon_rect.contains(pointer)
            && let Some(drag) = state.drag
            && items::is_weapon(drag.item_id)
            && drag.from == ItemContainer::Inventory
        {
            action = Action::SetWeapon(drag.item_id);
            state.drag = None;
        } else if let (Some(drag), Some((rect, cell))) = (
            state.drag,
            drop_target(pointer, stash_grid.as_ref(), inv_grid.as_ref()),
        ) {
            let destination = if stash_grid.as_ref().is_some_and(|g| g.0 == rect) {
                ItemContainer::Stash
            } else {
                ItemContainer::Inventory
            };
            let dest_items = match destination {
                ItemContainer::Inventory => &inventory.items,
                ItemContainer::Stash => stash,
            };
            let same = destination == drag.from;
            let merge = dest_items.iter().any(|i| i.item_id == drag.item_id as i64);
            let (gw, gh) = match destination {
                ItemContainer::Inventory => (RAID_GRID_W, RAID_GRID_H),
                ItemContainer::Stash => (STASH_GRID_W, STASH_GRID_H),
            };
            let allowed = merge
                || can_place(
                    dest_items,
                    drag.item_id as i64,
                    cell.0,
                    cell.1,
                    drag.rotated,
                    (gw, gh),
                    if same {
                        Some(drag.item_id as i64)
                    } else {
                        None
                    },
                );
            if allowed {
                action = Action::MoveItem {
                    from: drag.from,
                    item_id: drag.item_id,
                    to: destination,
                    x: cell.0,
                    y: cell.1,
                    rotated: drag.rotated,
                };
            }
            state.drag = None;
        } else {
            state.drag = None;
        }
        crate::ui::clear_drag_icon(ui.ctx());
    }

    if let Some(drag) = state.drag {
        let size = zn_sim::cells_for(drag.item_id as i64, drag.rotated);
        let pointer = ui.input(|i| i.pointer.interact_pos()).unwrap_or_default();
        let ghost = egui::Rect::from_min_size(
            pointer - drag.grab,
            egui::vec2(size.0 as f32 * CELL, size.1 as f32 * CELL),
        );
        let painter = ui.ctx().layer_painter(egui::LayerId::new(
            egui::Order::Tooltip,
            egui::Id::new("inventory_drag_ghost"),
        ));
        painter.rect_filled(
            ghost,
            3.0,
            item_color(drag.item_id as i64).gamma_multiply(0.75),
        );
        painter.rect_stroke(
            ghost,
            3.0,
            egui::Stroke::new(1.0_f32, egui::Color32::WHITE),
            egui::StrokeKind::Inside,
        );
    }

    action
}

type GridInfo = (egui::Rect, egui::Response);

fn draw_grid(
    ui: &mut egui::Ui,
    container: ItemContainer,
    items: &[InventoryItem],
    grid_w: i32,
    grid_h: i32,
    state: &mut InventoryUiState,
) -> GridInfo {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(grid_w as f32 * CELL, grid_h as f32 * CELL),
        egui::Sense::click_and_drag(),
    );
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, egui::Color32::from_rgb(24, 25, 23));
    for x in 0..=grid_w {
        let px = rect.left() + x as f32 * CELL;
        painter.line_segment(
            [egui::pos2(px, rect.top()), egui::pos2(px, rect.bottom())],
            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(48)),
        );
    }
    for y in 0..=grid_h {
        let py = rect.top() + y as f32 * CELL;
        painter.line_segment(
            [egui::pos2(rect.left(), py), egui::pos2(rect.right(), py)],
            egui::Stroke::new(1.0_f32, egui::Color32::from_gray(48)),
        );
    }

    for item in items.iter().filter(|i| i.item_id > 0) {
        let (w, h) = item.cells();
        if item.x < 0 || item.y < 0 || item.x + w > grid_w || item.y + h > grid_h {
            continue;
        }
        let item_rect = egui::Rect::from_min_size(
            rect.left_top() + egui::vec2(item.x as f32 * CELL, item.y as f32 * CELL),
            egui::vec2(w as f32 * CELL, h as f32 * CELL),
        )
        .shrink(2.0);
        painter.rect_filled(item_rect, 3.0, item_color(item.item_id));
        painter.rect_stroke(
            item_rect,
            3.0,
            egui::Stroke::new(
                1.0_f32,
                if state
                    .selected
                    .is_some_and(|s| s.from == container && s.item_id as i64 == item.item_id)
                {
                    egui::Color32::WHITE
                } else {
                    egui::Color32::from_black_alpha(160)
                },
            ),
            egui::StrokeKind::Inside,
        );
        let clip = painter.with_clip_rect(item_rect);
        let name = item_name(item.item_id);
        let label = if w == 1 && h == 1 {
            short_name(&name)
        } else {
            name.clone()
        };
        clip.text(
            item_rect.left_top() + egui::vec2(3.0, 2.0),
            egui::Align2::LEFT_TOP,
            label,
            egui::FontId::proportional(if w == 1 { 10.0 } else { 12.0 }),
            egui::Color32::WHITE,
        );
        if item.quantity > 1 {
            clip.text(
                item_rect.right_bottom() - egui::vec2(3.0, 2.0),
                egui::Align2::RIGHT_BOTTOM,
                item.quantity.to_string(),
                egui::FontId::proportional(10.0),
                egui::Color32::from_rgb(240, 230, 180),
            );
        }
        let hover = ui.interact(
            item_rect,
            egui::Id::new(("inventory_item", container, item.item_id)),
            egui::Sense::hover(),
        );
        hover.on_hover_text(format!(
            "{}\n{} × {} · {}×{} клеток · {:.2} кг",
            name,
            item.item_id,
            item.quantity,
            w,
            h,
            items::weight(item.item_id.max(0) as u64) * item.quantity.max(0) as f32
        ));
    }

    if response.drag_started() {
        let press = ui
            .input(|i| i.pointer.press_origin())
            .or_else(|| response.interact_pointer_pos());
        if let Some(press) = press {
            let x = ((press.x - rect.left()) / CELL).floor() as i32;
            let y = ((press.y - rect.top()) / CELL).floor() as i32;
            if let Some(item) = item_at(items, x, y) {
                state.selected = Some(Selection {
                    from: container,
                    item_id: item.item_id.max(0) as u64,
                });
                state.drag = Some(DragState {
                    from: container,
                    item_id: item.item_id.max(0) as u64,
                    grab: press
                        - (rect.left_top()
                            + egui::vec2(item.x as f32 * CELL, item.y as f32 * CELL)),
                    rotated: item.rotated,
                });
            }
        }
    }
    if (response.clicked() || response.secondary_clicked())
        && let Some(pos) = response.interact_pointer_pos()
    {
        let x = ((pos.x - rect.left()) / CELL).floor() as i32;
        let y = ((pos.y - rect.top()) / CELL).floor() as i32;
        state.selected = item_at(items, x, y).map(|i| Selection {
            from: container,
            item_id: i.item_id.max(0) as u64,
        });
    }

    (rect, response)
}

fn drop_target(
    pointer: egui::Pos2,
    stash: Option<&GridInfo>,
    inv: Option<&GridInfo>,
) -> Option<(egui::Rect, (i32, i32))> {
    for grid in [stash, inv].into_iter().flatten() {
        let rect = grid.0;
        if rect.contains(pointer) {
            let x = ((pointer.x - rect.left()) / CELL).floor() as i32;
            let y = ((pointer.y - rect.top()) / CELL).floor() as i32;
            return Some((rect, (x, y)));
        }
    }
    None
}

/// Курсор при перетаскивании. egui не хранит его сам, а системный курсор
/// «рука» над сеткой сбивает ощущение захвата.
fn clear_drag_icon(ctx: &egui::Context) {
    ctx.set_cursor_icon(egui::CursorIcon::Default);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn raw() -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1280.0, 720.0),
            )),
            ..Default::default()
        }
    }

    #[test]
    fn screens_render_without_a_window() {
        let ctx = egui::Context::default();
        let mut state = InventoryUiState::default();
        let inv = Inventory::default();
        let stash: Vec<InventoryItem> = Vec::new();
        let prefs = Settings::default();

        let _ = ctx.run(raw(), |ctx| {
            let _ = menu(ctx, items::PISTOL, true, &inv, &stash, &mut state);
        });
        let _ = ctx.run(raw(), |ctx| {
            let _ = self::inventory(ctx, &inv, &stash, items::PISTOL, &mut state);
        });
        let _ = ctx.run(raw(), |ctx| {
            let _ = paused(ctx);
        });
        let _ = ctx.run(raw(), |ctx| {
            let _ = self::settings(ctx, &prefs, Screen::Menu);
        });
    }
}
