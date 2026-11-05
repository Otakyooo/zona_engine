//! Сеточный инвентарь: клетки, поворот и проверка пересечений.
//!
//! Раньше инвентарь был плоским списком стеков, а `slot` лишь нумеровал
//! строки. Для интерфейса в духе extraction-shooter этого мало: предмет
//! занимает несколько клеток, его можно повернуть, а перенос between
//! контейнерами должен проверяться на сервере. Модель живёт в `zn-sim`, чтобы
//! клиент и сервер считали занятость одинаково.

use serde::{Deserialize, Serialize};

use crate::items;

/// Сетка рюкзака в рейде.
pub const RAID_GRID_W: i32 = 10;
pub const RAID_GRID_H: i32 = 6;
/// Схрон больше рюкзака: вынесенный лут не должен упираться в те же 60 клеток.
pub const STASH_GRID_W: i32 = 12;
pub const STASH_GRID_H: i32 = 12;

/// Где лежит предмет. Контейнер — часть протокола переноса.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ItemContainer {
    Inventory,
    Stash,
}

/// Один стек на сетке. `slot` остаётся для совместимости со старыми БД и
/// сортировки, а геометрию задают `x`/`y`/`rotated`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryItem {
    pub item_id: i64,
    pub quantity: i32,
    pub slot: i32,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub rotated: bool,
}

impl InventoryItem {
    pub fn new(item_id: i64, quantity: i32, x: i32, y: i32, rotated: bool) -> Self {
        Self {
            item_id,
            quantity,
            slot: 0,
            x,
            y,
            rotated,
        }
    }

    /// Размер с учётом поворота.
    pub fn cells(&self) -> (i32, i32) {
        let size = items::grid_size(self.item_id.max(0) as u64).rotated(self.rotated);
        (size.w as i32, size.h as i32)
    }
}

/// Рюкзак игрока. Схрон использует те же свободные функции, но лежит в `Vec`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub items: Vec<InventoryItem>,
}

impl Inventory {
    /// Складывает предмет в существующий стек или ищет свободное место.
    ///
    /// Возвращает `false`, если сетка заполнена: вызывающий код обязан не
    /// удалять лут из мира, пока предмет не размещён.
    pub fn add_item(&mut self, item_id: i64, quantity: i32) -> bool {
        add_to_items(&mut self.items, item_id, quantity, RAID_GRID_W, RAID_GRID_H)
    }

    pub fn remove_item(&mut self, item_id: i64, quantity: i32) -> bool {
        if let Some(it) = self.items.iter_mut().find(|i| i.item_id == item_id)
            && it.quantity >= quantity
        {
            it.quantity -= quantity;
            if it.quantity <= 0 {
                self.items.retain(|i| i.item_id != item_id);
            }
            return true;
        }
        false
    }

    /// Забирает стек целиком. Нужен переносу между контейнерами.
    pub fn take_item(&mut self, item_id: i64) -> Option<InventoryItem> {
        let idx = self.items.iter().position(|i| i.item_id == item_id)?;
        Some(self.items.remove(idx))
    }

    pub fn has_item(&self, item_id: i64, quantity: i32) -> bool {
        self.items
            .iter()
            .any(|i| i.item_id == item_id && i.quantity >= quantity)
    }

    /// Ставит уже существующий стек в конкретную клетку.
    pub fn place_item(&mut self, item_id: i64, x: i32, y: i32, rotated: bool) -> bool {
        move_in_items(
            &mut self.items,
            item_id,
            x,
            y,
            rotated,
            RAID_GRID_W,
            RAID_GRID_H,
        )
    }

    /// Пересобирает раскладку после загрузки старой БД или изменения размера
    /// сетки. Валидные позиции сохраняются, пересечения разъезжаются.
    pub fn normalize(&mut self) {
        normalize_items(&mut self.items, RAID_GRID_W, RAID_GRID_H);
    }

    pub fn used_cells(&self) -> i32 {
        self.items
            .iter()
            .map(|i| {
                let (w, h) = i.cells();
                w * h
            })
            .sum()
    }

    pub fn capacity_cells() -> i32 {
        RAID_GRID_W * RAID_GRID_H
    }
}

/// Размер предмета на сетке без экземпляра.
pub fn cells_for(item_id: i64, rotated: bool) -> (i32, i32) {
    let size = items::grid_size(item_id.max(0) as u64).rotated(rotated);
    (size.w as i32, size.h as i32)
}

/// Можно ли положить предмет в клетку, не задев другие.
///
/// `ignore` — id перемещаемого стека: он не должен пересекаться сам с собой.
pub fn can_place(
    items: &[InventoryItem],
    item_id: i64,
    x: i32,
    y: i32,
    rotated: bool,
    grid: (i32, i32),
    ignore: Option<i64>,
) -> bool {
    let (grid_w, grid_h) = grid;
    let (w, h) = cells_for(item_id, rotated);
    if x < 0 || y < 0 || x + w > grid_w || y + h > grid_h {
        return false;
    }
    items.iter().all(|other| {
        if ignore == Some(other.item_id) {
            return true;
        }
        let (ow, oh) = other.cells();
        x + w <= other.x || other.x + ow <= x || y + h <= other.y || other.y + oh <= y
    })
}

/// Первое свободное место, сначала без поворота, затем повёрнутое.
pub fn find_spot(
    items: &[InventoryItem],
    item_id: i64,
    grid_w: i32,
    grid_h: i32,
) -> Option<(i32, i32, bool)> {
    for rotated in [false, true] {
        let (w, h) = cells_for(item_id, rotated);
        for y in 0..=(grid_h - h) {
            for x in 0..=(grid_w - w) {
                if can_place(items, item_id, x, y, rotated, (grid_w, grid_h), None) {
                    return Some((x, y, rotated));
                }
            }
        }
    }
    None
}

/// Добавляет стек в произвольный контейнер. Сначала ищет такой же предмет —
/// иначе одинаковые аптечки занимали бы отдельные клетки.
pub fn add_to_items(
    items: &mut Vec<InventoryItem>,
    item_id: i64,
    quantity: i32,
    grid_w: i32,
    grid_h: i32,
) -> bool {
    if quantity <= 0 {
        return false;
    }
    if let Some(it) = items.iter_mut().find(|i| i.item_id == item_id) {
        it.quantity += quantity;
        return true;
    }
    let Some((x, y, rotated)) = find_spot(items, item_id, grid_w, grid_h) else {
        return false;
    };
    let mut item = InventoryItem::new(item_id, quantity, x, y, rotated);
    item.slot = items.len() as i32;
    items.push(item);
    true
}

/// Удаляет предмет из произвольного контейнера.
pub fn take_from_items(items: &mut Vec<InventoryItem>, item_id: i64) -> Option<InventoryItem> {
    let idx = items.iter().position(|i| i.item_id == item_id)?;
    Some(items.remove(idx))
}

/// Двигает существующий стек внутри одного контейнера.
pub fn move_in_items(
    items: &mut [InventoryItem],
    item_id: i64,
    x: i32,
    y: i32,
    rotated: bool,
    grid_w: i32,
    grid_h: i32,
) -> bool {
    if !can_place(
        items,
        item_id,
        x,
        y,
        rotated,
        (grid_w, grid_h),
        Some(item_id),
    ) {
        return false;
    }
    let Some(it) = items.iter_mut().find(|i| i.item_id == item_id) else {
        return false;
    };
    it.x = x;
    it.y = y;
    it.rotated = rotated;
    true
}

/// Разводит пересекающиеся стеки по свободным клеткам.
pub fn normalize_items(items: &mut [InventoryItem], grid_w: i32, grid_h: i32) {
    let mut placed: Vec<InventoryItem> = Vec::with_capacity(items.len());
    for item in items.iter_mut() {
        let valid = can_place(
            &placed,
            item.item_id,
            item.x,
            item.y,
            item.rotated,
            (grid_w, grid_h),
            None,
        );
        if !valid && let Some((x, y, rotated)) = find_spot(&placed, item.item_id, grid_w, grid_h) {
            item.x = x;
            item.y = y;
            item.rotated = rotated;
        }
        placed.push(item.clone());
    }
    for (i, item) in items.iter_mut().enumerate() {
        item.slot = i as i32;
        *item = placed[i].clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::items::{MEDKIT, PISTOL, RIFLE, SMG};

    #[test]
    fn new_items_are_placed_without_overlap() {
        let mut inv = Inventory::default();
        assert!(inv.add_item(PISTOL as i64, 1));
        assert!(inv.add_item(SMG as i64, 1));
        assert_eq!(inv.items.len(), 2);
        assert_ne!(
            (inv.items[0].x, inv.items[0].y),
            (inv.items[1].x, inv.items[1].y)
        );
    }

    #[test]
    fn stacks_share_one_cell() {
        let mut inv = Inventory::default();
        assert!(inv.add_item(MEDKIT as i64, 1));
        assert!(inv.add_item(MEDKIT as i64, 2));
        assert_eq!(inv.items.len(), 1);
        assert_eq!(inv.items[0].quantity, 3);
    }

    #[test]
    fn rotation_changes_footprint() {
        let size = items::grid_size(RIFLE);
        assert_eq!(
            cells_for(RIFLE as i64, false),
            (size.w as i32, size.h as i32)
        );
        assert_eq!(
            cells_for(RIFLE as i64, true),
            (size.h as i32, size.w as i32)
        );
    }

    #[test]
    fn overlap_and_bounds_are_rejected() {
        let mut inv = Inventory::default();
        assert!(inv.add_item(PISTOL as i64, 1));
        let first = inv.items[0].clone();
        assert!(!can_place(
            &inv.items,
            SMG as i64,
            first.x,
            first.y,
            false,
            (RAID_GRID_W, RAID_GRID_H),
            None
        ));
        assert!(!can_place(
            &inv.items,
            MEDKIT as i64,
            RAID_GRID_W,
            0,
            false,
            (RAID_GRID_W, RAID_GRID_H),
            None
        ));
    }

    #[test]
    fn moving_ignores_self_but_not_others() {
        let mut inv = Inventory::default();
        assert!(inv.add_item(PISTOL as i64, 1));
        assert!(inv.add_item(MEDKIT as i64, 1));
        let pos = (inv.items[0].x, inv.items[0].y);
        assert!(inv.place_item(PISTOL as i64, pos.0, pos.1, false));
        let med = inv.items[1].clone();
        assert!(!inv.place_item(PISTOL as i64, med.x, med.y, false));
    }

    #[test]
    fn normalize_repairs_legacy_overlaps() {
        let mut inv = Inventory {
            items: vec![
                InventoryItem::new(PISTOL as i64, 1, 0, 0, false),
                InventoryItem::new(SMG as i64, 1, 0, 0, false),
                InventoryItem::new(MEDKIT as i64, 1, 0, 0, false),
            ],
        };
        inv.normalize();
        for (i, a) in inv.items.iter().enumerate() {
            for b in inv.items.iter().skip(i + 1) {
                assert!(
                    !overlaps(a, b),
                    "{} и {} остались в одной клетке",
                    a.item_id,
                    b.item_id
                );
            }
        }
    }

    #[test]
    fn full_grid_rejects_new_items() {
        let mut inv = Inventory::default();
        for i in 0..(RAID_GRID_W * RAID_GRID_H) {
            let id = 10_000 + i as i64;
            assert!(inv.add_item(id, 1));
        }
        assert!(!inv.add_item(99_999, 1));
    }

    fn overlaps(a: &InventoryItem, b: &InventoryItem) -> bool {
        let (aw, ah) = a.cells();
        let (bw, bh) = b.cells();
        !(a.x + aw <= b.x || b.x + bw <= a.x || a.y + ah <= b.y || b.y + bh <= a.y)
    }
}
