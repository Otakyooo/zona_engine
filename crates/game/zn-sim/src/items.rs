//! Каталог предметов и оружия.
//!
//! Раньше оружие было строкой (`equipped_weapon: "rifle"`), а предмет в
//! инвентаре — только `id` + количество: имя и эффект жили в `match` по всему
//! коду (HUD, сервер, лут). Здесь одна таблица, из которой читают симуляция,
//! сервер и клиент. Новый предмет добавляется строкой в `CATALOG`, а не
//! правкой трёх `match`.
//!
//! Id остаются в общем пространстве `ids`: 1..=999 — предметы, поэтому
//! оружие и расходники не конфликтуют с лутом (1_000_000+) и артефактами.

/// Оружие: урон, темп и расход патронов.
///
/// Кулдаун — свойство оружия, а не клиента: без него автоклик давал любой DPS,
/// потому что сервер принимал выстрелы без ограничения частоты.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponDef {
    pub id: u64,
    pub name: &'static str,
    pub damage: f32,
    pub cooldown_secs: f32,
    pub ammo_per_shot: u32,
}

/// Описание предмета. `heal = 0.0` — не расходник.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemDef {
    pub id: u64,
    pub name: &'static str,
    pub value: u32,
    pub consumable: bool,
    pub heal: f32,
    pub weapon: Option<WeaponDef>,
}

impl ItemDef {
    pub fn is_weapon(&self) -> bool {
        self.weapon.is_some()
    }
}

pub const MEDKIT: u64 = 1;
pub const BANDAGE: u64 = 2;
pub const PISTOL: u64 = 10;
pub const SMG: u64 = 11;
pub const RIFLE: u64 = 12;
pub const LIGHT_ARMOR: u64 = 20;
pub const AMMO: u64 = 100;

/// Оружие по умолчанию: с ним игрок появляется без выбора снаряжения.
pub const DEFAULT_WEAPON: u64 = PISTOL;
/// Стартовый и максимальный боезапас.
pub const START_AMMO: u32 = 90;
pub const MAX_AMMO: u32 = 300;

pub const CATALOG: &[ItemDef] = &[
    ItemDef {
        id: MEDKIT,
        name: "Medkit",
        value: 50,
        consumable: true,
        heal: 30.0,
        weapon: None,
    },
    ItemDef {
        id: BANDAGE,
        name: "Bandage",
        value: 20,
        consumable: true,
        heal: 15.0,
        weapon: None,
    },
    ItemDef {
        id: PISTOL,
        name: "PM",
        value: 100,
        consumable: false,
        heal: 0.0,
        weapon: Some(WeaponDef {
            id: PISTOL,
            name: "PM",
            damage: 20.0,
            cooldown_secs: 0.28,
            ammo_per_shot: 1,
        }),
    },
    ItemDef {
        id: SMG,
        name: "SMG",
        value: 250,
        consumable: false,
        heal: 0.0,
        weapon: Some(WeaponDef {
            id: SMG,
            name: "SMG",
            damage: 22.0,
            cooldown_secs: 0.10,
            ammo_per_shot: 1,
        }),
    },
    ItemDef {
        id: RIFLE,
        name: "Rifle",
        value: 400,
        consumable: false,
        heal: 0.0,
        weapon: Some(WeaponDef {
            id: RIFLE,
            name: "Rifle",
            damage: 35.0,
            cooldown_secs: 0.60,
            ammo_per_shot: 1,
        }),
    },
    ItemDef {
        id: LIGHT_ARMOR,
        name: "Light armor",
        value: 300,
        consumable: false,
        heal: 0.0,
        weapon: None,
    },
    ItemDef {
        id: AMMO,
        name: "Ammo",
        value: 2,
        consumable: false,
        heal: 0.0,
        weapon: None,
    },
];

pub fn get(id: u64) -> Option<&'static ItemDef> {
    CATALOG.iter().find(|d| d.id == id)
}

pub fn weapon(id: u64) -> Option<WeaponDef> {
    get(id).and_then(|d| d.weapon)
}

pub fn name(id: u64) -> &'static str {
    get(id).map(|d| d.name).unwrap_or("unknown item")
}

pub fn value(id: u64) -> u32 {
    get(id).map(|d| d.value).unwrap_or(0)
}

pub fn heal(id: u64) -> f32 {
    get(id).map(|d| d.heal).unwrap_or(0.0)
}

pub fn is_weapon(id: u64) -> bool {
    weapon(id).is_some()
}

/// Снаряжение переживает смерть: надетое оружие и броня.
pub fn is_equipment(id: u64) -> bool {
    is_weapon(id) || id == LIGHT_ARMOR
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{IdKind, classify};

    #[test]
    fn catalog_ids_are_items() {
        for def in CATALOG {
            assert_eq!(
                classify(def.id),
                Some(IdKind::Item),
                "{} вне пространства предметов",
                def.name
            );
        }
    }

    #[test]
    fn catalog_ids_are_unique() {
        for (i, a) in CATALOG.iter().enumerate() {
            for b in CATALOG.iter().skip(i + 1) {
                assert_ne!(a.id, b.id, "{} и {} делят id", a.name, b.name);
            }
        }
    }

    #[test]
    fn weapon_lookup_matches_catalog() {
        assert_eq!(weapon(PISTOL).unwrap().damage, 20.0);
        assert_eq!(weapon(SMG).unwrap().damage, 22.0);
        assert_eq!(weapon(RIFLE).unwrap().damage, 35.0);
        assert!(weapon(MEDKIT).is_none());
        assert!(weapon(999_999).is_none());
    }

    #[test]
    fn consumables_have_positive_heal() {
        for def in CATALOG.iter().filter(|d| d.consumable) {
            assert!(def.heal > 0.0, "{} не лечит", def.name);
        }
    }

    #[test]
    fn every_weapon_has_a_rate_limit() {
        for def in CATALOG.iter().filter(|d| d.is_weapon()) {
            let w = def.weapon.unwrap();
            assert!(w.cooldown_secs > 0.0, "{} стреляет без задержки", w.name);
            assert!(w.ammo_per_shot > 0, "{} не тратит патроны", w.name);
            assert!(w.damage > 0.0, "{} безвредно", w.name);
        }
    }

    #[test]
    fn equipment_is_weapon_or_armor() {
        assert!(is_equipment(PISTOL));
        assert!(is_equipment(RIFLE));
        assert!(is_equipment(LIGHT_ARMOR));
        assert!(!is_equipment(MEDKIT));
        assert!(!is_equipment(AMMO));
    }
}
