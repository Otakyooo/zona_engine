//! zn-sim: общий код симуляции (клиент и сервер).

use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Пространства идентификаторов, общие для клиента и сервера.
///
/// Раньше id предметов, лута и артефактов жили в одном числовом ряду: артефакт
/// с id 1 в инвентаре съедался как аптечка. Разносим по диапазонам.
pub mod ids {
    /// Предметы инвентаря (1 — аптечка, 2 — бинт).
    pub const ITEM_MAX: u64 = 999;
    /// Выпавший лут: `LOOT_BASE + счётчик`.
    pub const LOOT_BASE: u64 = 1_000_000;
    /// Артефакты в инвентаре: `ARTIFACT_BASE + id артефакта`.
    pub const ARTIFACT_BASE: u64 = 2_000_000;
    /// Существа: `CREATURE_BASE + id`.
    pub const CREATURE_BASE: u64 = 3_000_000;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum IdKind {
        Item,
        Loot,
        Artifact,
        Creature,
    }

    pub fn classify(id: u64) -> Option<IdKind> {
        if (1..=ITEM_MAX).contains(&id) {
            Some(IdKind::Item)
        } else if (LOOT_BASE..LOOT_BASE + 1_000_000).contains(&id) {
            Some(IdKind::Loot)
        } else if (ARTIFACT_BASE..ARTIFACT_BASE + 1_000_000).contains(&id) {
            Some(IdKind::Artifact)
        } else if (CREATURE_BASE..CREATURE_BASE + 1_000_000).contains(&id) {
            Some(IdKind::Creature)
        } else {
            None
        }
    }
}

/// Допустимое расхождение углов, при котором коррекция не применяется (~20°).
const ANGLE_THRESHOLD: f32 = 0.35;
/// Доля, на которую серверное значение подмешивается к локальному.
const ANGLE_BLEND: f32 = 0.15;

/// Мягкая коррекция углов обзора.
///
/// Возвращает `None`, если расхождение в пределах нормы: снапшот приходит
/// 20 раз в секунду, и жёсткая перезапись yaw/pitch делала мышь неотзывчивой.
pub fn correct_angles(local: (f32, f32), server: (f32, f32)) -> Option<(f32, f32)> {
    let dyaw = server.0 - local.0;
    let dpitch = server.1 - local.1;
    if dyaw.abs() <= ANGLE_THRESHOLD && dpitch.abs() <= ANGLE_THRESHOLD {
        return None;
    }
    Some((local.0 + dyaw * ANGLE_BLEND, local.1 + dpitch * ANGLE_BLEND))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputCommand {
    pub seq: u32,
    pub yaw: f32,
    pub pitch: f32,
    pub move_x: f32,
    pub move_z: f32,
    pub dt: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum CreatureState {
    Patrol,
    Chase,
    Attack,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub id: u64,
    pub name: String,
    pub position: Vec3,
    pub health: f32,
    pub max_health: f32,
    pub radiation: f32,
    pub hunger: f32,
    pub thirst: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub velocity: Vec3,
    pub xp: i64,
    pub level: i32,
    pub currency: i64,
    pub skills: Vec<Skill>,
    pub equipped_weapon: String,
    pub equipped_armor: String,
    pub inventory: Inventory,
}

impl Player {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            position: Vec3::ZERO,
            health: 100.0,
            max_health: 100.0,
            radiation: 0.0,
            hunger: 100.0,
            thirst: 100.0,
            yaw: 0.0,
            pitch: 0.0,
            velocity: Vec3::ZERO,
            xp: 0,
            level: 1,
            currency: 0,
            skills: Vec::new(),
            equipped_weapon: "pistol".into(),
            equipped_armor: "none".into(),
            inventory: Inventory::default(),
        }
    }
    pub fn add_xp(&mut self, amount: i64) {
        self.xp += amount;
        self.level = 1 + (self.xp / 1000) as i32;
    }

    /// Переносит сохранённый прогресс в живого игрока. Без этого `save_profile`
    /// запишет в БД дефолтные `level = 1, xp = 0` и сотрёт прогресс.
    pub fn apply_profile(&mut self, profile: &PlayerProfile) {
        self.level = profile.level;
        self.xp = profile.xp;
        self.currency = profile.currency;
        self.position = profile.position;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerState {
    pub id: u64,
    pub name: String,
    pub position: Vec3,
    pub health: f32,
    pub radiation: f32,
    pub hunger: f32,
    pub thirst: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub level: i32,
    pub xp: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Anomaly {
    pub id: u64,
    pub position: Vec3,
    pub radius: f32,
    pub damage: f32,
    pub radiation: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub id: u64,
    pub position: Vec3,
    pub effect: String,
    pub value: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LootItem {
    pub id: u64,
    pub position: Vec3,
    pub item: Item,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Creature {
    pub id: u64,
    pub position: Vec3,
    pub health: f32,
    pub max_health: f32,
    pub state: CreatureState,
    pub target: Option<u64>,
    pub damage: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExtractionZone {
    pub id: u64,
    pub position: Vec3,
    pub radius: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub players: Vec<PlayerState>,
    pub anomalies: Vec<Anomaly>,
    pub artifacts: Vec<Artifact>,
    pub loot: Vec<LootItem>,
    pub creatures: Vec<Creature>,
    pub extraction_zones: Vec<ExtractionZone>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub level: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CraftingRecipe {
    pub id: String,
    pub inputs: Vec<(u64, u32)>,
    pub output: Item,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub name: String,
    pub value: u32,
    pub consumable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Loot {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerProfile {
    pub user_id: i64,
    pub username: String,
    pub level: i32,
    pub xp: i64,
    pub currency: i64,
    pub position: Vec3,
}

impl PlayerProfile {
    pub fn new(user_id: i64, username: impl Into<String>) -> Self {
        Self {
            user_id,
            username: username.into(),
            level: 1,
            xp: 0,
            currency: 0,
            position: Vec3::ZERO,
        }
    }
    pub fn add_xp(&mut self, amount: i64) {
        self.xp += amount;
        self.level = 1 + (self.xp / 1000) as i32;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InventoryItem {
    pub item_id: i64,
    pub quantity: i32,
    pub slot: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub items: Vec<InventoryItem>,
}

impl Inventory {
    pub fn add_item(&mut self, item_id: i64, quantity: i32) {
        if let Some(it) = self.items.iter_mut().find(|i| i.item_id == item_id) {
            it.quantity += quantity;
        } else {
            let slot = self.items.len() as i32;
            self.items.push(InventoryItem {
                item_id,
                quantity,
                slot,
            });
        }
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
    pub fn has_item(&self, item_id: i64, quantity: i32) -> bool {
        self.items
            .iter()
            .any(|i| i.item_id == item_id && i.quantity >= quantity)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GameEvent {
    PlayerDeath {
        player_id: u64,
        dropped_loot: Vec<Item>,
        dropped_consumables: Vec<Item>,
    },
    ExtractionSuccess {
        player_id: u64,
        loot: Vec<Item>,
    },
    CreatureKilled {
        creature_id: u64,
        killer: u64,
        xp: i64,
    },
}

#[derive(Default)]
pub struct SimWorld {
    pub tick: u64,
    pub players: Vec<Player>,
    pub anomalies: Vec<Anomaly>,
    pub artifacts: Vec<Artifact>,
    pub loot: Vec<LootItem>,
    pub creatures: Vec<Creature>,
    pub extraction_zones: Vec<ExtractionZone>,
    pub extraction_timers: HashMap<u64, f32>,
    pub events: Vec<GameEvent>,
    /// Следующий свободный id для выпавшего лута.
    pub next_loot_id: u64,
}

impl SimWorld {
    pub fn new() -> Self {
        let mut world = Self::default();
        world.seed_loot_ids();
        world
    }

    /// Счётчик стартует выше всего, что уже лежит в мире, и не ниже `LOOT_BASE`,
    /// чтобы выпавший лут не пересекался ни с предметами, ни с лутом карты.
    pub fn seed_loot_ids(&mut self) {
        let max_existing = self.loot.iter().map(|l| l.id).max().unwrap_or(0);
        self.next_loot_id = max_existing.max(ids::LOOT_BASE) + 1;
    }
    pub fn add_player(&mut self, p: Player) {
        self.players.push(p);
    }
    pub fn remove_player(&mut self, id: u64) {
        self.players.retain(|p| p.id != id);
    }
    pub fn get_player(&self, id: u64) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }
    pub fn get_player_mut(&mut self, id: u64) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }

    /// Выкладывает инвентарь игрока на землю в точке смерти и очищает его.
    ///
    /// Вызывать строго до респауна: позиция берётся у ещё не перемещённого
    /// игрока, иначе весь лут окажется в начале координат. Id выдаются из
    /// `next_loot_id`, поэтому кучи разных смертей не конфликтуют.
    pub fn drop_inventory_at(&mut self, player_id: u64, pos: Vec3) -> Vec<Item> {
        let inventory = match self.get_player_mut(player_id) {
            Some(p) => std::mem::take(&mut p.inventory),
            None => return Vec::new(),
        };
        let mut dropped = Vec::new();
        for it in inventory.items {
            let item = Item {
                id: it.item_id as u64,
                name: format!("item_{}", it.item_id),
                value: 10,
                consumable: true,
            };
            self.loot.push(LootItem {
                id: self.next_loot_id,
                position: pos,
                item: item.clone(),
            });
            self.next_loot_id += 1;
            dropped.push(item);
        }
        dropped
    }

    /// Помечает игроков с нулевым здоровьем.
    ///
    /// Вызывать после всех источников урона: урон существ применяется позже
    /// движения, и раньше такие смерти не попадали в обработку вообще —
    /// игрок оставался с нулевым HP без респауна.
    pub fn collect_deaths(&mut self, dead: &mut Vec<u64>) {
        for p in self.players.iter_mut() {
            if p.health <= 0.0 {
                p.health = 0.0;
                if !dead.contains(&p.id) {
                    dead.push(p.id);
                }
            }
        }
    }
    pub fn apply_input(&mut self, player_id: u64, cmd: &InputCommand) {
        if let Some(p) = self.get_player_mut(player_id) {
            p.yaw = cmd.yaw;
            p.pitch = cmd.pitch;
            let forward = Vec3::new(cmd.yaw.cos(), 0.0, cmd.yaw.sin());
            let right = Vec3::new(forward.z, 0.0, -forward.x);
            let mut vel = forward * (-cmd.move_z) + right * cmd.move_x;
            vel *= 5.0;
            p.velocity.x = vel.x;
            p.velocity.z = vel.z;
        }
    }
    pub fn craft(&mut self, player_id: u64, _recipe: &CraftingRecipe) -> bool {
        // For MVP: just add output item to loot near player? Simplified: add XP
        if let Some(p) = self.get_player_mut(player_id) {
            p.add_xp(10);
            true
        } else {
            false
        }
    }
    pub fn tick(&mut self, dt: f32) {
        self.events.clear();
        // players
        let mut dead_players: Vec<u64> = Vec::new();
        for p in self.players.iter_mut() {
            // gravity
            p.velocity.y -= 9.81 * dt;
            let new_y = p.position.y + p.velocity.y * dt;
            if new_y <= 0.8 {
                p.position.y = 0.8;
                p.velocity.y = 0.0;
            } else {
                p.position.y = new_y;
            }
            p.position.x += p.velocity.x * dt;
            p.position.z += p.velocity.z * dt;
            p.position.x = p.position.x.clamp(-19.0, 19.0);
            p.position.z = p.position.z.clamp(-19.0, 19.0);

            // survival decay
            p.hunger = (p.hunger - 0.5 * dt).max(0.0);
            p.thirst = (p.thirst - 0.8 * dt).max(0.0);
            if p.hunger <= 0.0 || p.thirst <= 0.0 {
                p.health -= 2.0 * dt;
            }

            // anomalies
            for a in &self.anomalies {
                let dist = (p.position - a.position).length();
                if dist <= a.radius {
                    p.health -= a.damage * dt;
                    p.radiation += a.radiation * dt;
                }
            }

            // radiation damage
            if p.radiation > 50.0 {
                p.health -= (p.radiation - 50.0) * 0.01 * dt;
            }
        }

        // extraction timers
        let mut completed: Vec<u64> = Vec::new();
        for p in &self.players {
            for z in &self.extraction_zones {
                if (p.position - z.position).length() <= z.radius {
                    let timer = self.extraction_timers.entry(p.id).or_insert(10.0);
                    *timer -= dt;
                    if *timer <= 0.0 {
                        completed.push(p.id);
                    }
                } else {
                    self.extraction_timers.remove(&p.id);
                }
            }
        }
        for pid in &completed {
            // collect loot? For now empty; client/server handle
            self.events.push(GameEvent::ExtractionSuccess {
                player_id: *pid,
                loot: Vec::new(),
            });
            if let Some(p) = self.get_player_mut(*pid) {
                p.add_xp(100);
            }
            self.extraction_timers.remove(pid);
        }

        // creature AI
        for c in self.creatures.iter_mut() {
            // find nearest player
            let mut nearest: Option<(u64, f32)> = None;
            for p in &self.players {
                let d = (p.position - c.position).length();
                if nearest.is_none() || d < nearest.unwrap().1 {
                    nearest = Some((p.id, d));
                }
            }
            match c.state {
                CreatureState::Patrol => {
                    if let Some((_, d)) = nearest
                        && d < 20.0
                    {
                        c.state = CreatureState::Chase;
                    }
                }
                CreatureState::Chase => {
                    if let Some((pid, d)) = nearest {
                        if d < 2.0 {
                            c.state = CreatureState::Attack;
                            c.target = Some(pid);
                        } else if d > 30.0 {
                            c.state = CreatureState::Patrol;
                        } else {
                            // move toward player
                            if let Some(p) = self.players.iter().find(|pl| pl.id == pid) {
                                let dir = (p.position - c.position).normalize_or_zero();
                                c.position += dir * 3.0 * dt;
                            }
                        }
                    }
                }
                CreatureState::Attack => {
                    if let Some(pid) = c.target {
                        if let Some(p) = self.players.iter().find(|pl| pl.id == pid) {
                            let d = (p.position - c.position).length();
                            if d > 2.5 {
                                c.state = CreatureState::Chase;
                            } else {
                                // damage player (handled below)
                            }
                        } else {
                            c.state = CreatureState::Patrol;
                            c.target = None;
                        }
                    }
                }
            }
        }
        // creature attacks
        let attacks: Vec<(u64, f32)> = self
            .creatures
            .iter()
            .filter(|c| c.state == CreatureState::Attack)
            .filter_map(|c| c.target.map(|pid| (pid, c.damage)))
            .collect();
        for (pid, dmg) in attacks {
            if let Some(p) = self.get_player_mut(pid) {
                p.health -= dmg * dt;
                if p.health <= 0.0 {
                    p.health = 0.0;
                }
            }
        }

        // Смерти собираются после всех источников урона: урон существ
        // применяется позже движения, и раньше такие смерти терялись.
        self.collect_deaths(&mut dead_players);

        // handle deaths
        for pid in &dead_players {
            // Позиция читается до респауна — иначе лут падает в начало координат.
            let death_pos = self
                .get_player(*pid)
                .map(|p| p.position)
                .unwrap_or_default();
            let dropped_loot = self.drop_inventory_at(*pid, death_pos);
            self.events.push(GameEvent::PlayerDeath {
                player_id: *pid,
                dropped_loot,
                dropped_consumables: Vec::new(),
            });
            if let Some(p) = self.get_player_mut(*pid) {
                p.health = p.max_health;
                p.position = Vec3::ZERO;
                p.radiation = 0.0;
                p.hunger = 100.0;
                p.thirst = 100.0;
            }
        }
        // remove dead creatures
        self.creatures.retain(|c| c.health > 0.0);

        self.tick += 1;
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tick: self.tick,
            players: self
                .players
                .iter()
                .map(|p| PlayerState {
                    id: p.id,
                    name: p.name.clone(),
                    position: p.position,
                    health: p.health,
                    radiation: p.radiation,
                    hunger: p.hunger,
                    thirst: p.thirst,
                    yaw: p.yaw,
                    pitch: p.pitch,
                    level: p.level,
                    xp: p.xp,
                })
                .collect(),
            anomalies: self.anomalies.clone(),
            artifacts: self.artifacts.clone(),
            loot: self.loot.clone(),
            creatures: self.creatures.clone(),
            extraction_zones: self.extraction_zones.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_applies_gravity() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        w.tick(1.0 / 60.0);
        // gravity should place player on ground at 0.8
        let y = w.get_player(1).unwrap().position.y;
        assert!((y - 0.8).abs() < 0.01);
    }

    #[test]
    fn anomaly_damages_player() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        // move player into anomaly at (5,0,5) radius 3
        w.get_player_mut(1).unwrap().position = Vec3::new(5.0, 0.8, 5.0);
        w.anomalies.push(Anomaly {
            id: 1,
            position: Vec3::new(5.0, 0.0, 5.0),
            radius: 3.0,
            damage: 5.0,
            radiation: 10.0,
        });
        let hp0 = w.get_player(1).unwrap().health;
        w.tick(1.0);
        let hp1 = w.get_player(1).unwrap().health;
        assert!(hp1 < hp0);
    }

    #[test]
    fn inventory_add_remove() {
        let mut inv = Inventory::default();
        inv.add_item(1, 2);
        assert!(inv.has_item(1, 2));
        assert!(inv.remove_item(1, 1));
        assert!(inv.has_item(1, 1));
        assert!(!inv.remove_item(1, 5));
    }

    #[test]
    fn extraction_awards_xp_once() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.position = Vec3::new(0.0, 0.8, 0.0);
        w.add_player(p);
        w.extraction_zones.push(ExtractionZone {
            id: 1,
            position: Vec3::new(0.0, 0.0, 0.0),
            radius: 3.0,
        });

        // ~11.7 с: таймер зоны (10 с) успевает сработать ровно один раз
        for _ in 0..700 {
            w.tick(1.0 / 60.0);
        }

        assert_eq!(
            w.get_player(1).unwrap().xp,
            100,
            "XP за экстракцию должен начисляться один раз"
        );
    }

    #[test]
    fn angle_correction_ignores_small_delta() {
        assert!(correct_angles((1.0, 0.0), (1.1, 0.1)).is_none());
        let (yaw, _) = correct_angles((1.0, 0.0), (2.0, 0.0)).unwrap();
        assert!(yaw > 1.0 && yaw < 2.0, "коррекция должна быть частичной");
    }

    #[test]
    fn artifact_id_does_not_collide_with_medkit() {
        use ids::{ARTIFACT_BASE, ITEM_MAX, IdKind, classify};
        assert_eq!(classify(1), Some(IdKind::Item));
        assert_eq!(classify(2), Some(IdKind::Item));
        assert_eq!(classify(ARTIFACT_BASE + 1), Some(IdKind::Artifact));
        assert_eq!(classify(ids::LOOT_BASE), Some(IdKind::Loot));
        // за пределами всех диапазонов — не опознаётся
        assert_eq!(classify(ITEM_MAX + 1), None);
        assert_eq!(classify(0), None);
    }

    #[test]
    fn death_loot_at_death_position() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.inventory.add_item(1, 2);
        w.add_player(p);

        let pos = Vec3::new(7.0, 0.5, -3.0);
        let dropped = w.drop_inventory_at(1, pos);
        assert_eq!(dropped.len(), 1);
        assert_eq!(w.loot.len(), 1);
        assert_eq!(w.loot[0].position, pos);
        assert!(w.get_player(1).unwrap().inventory.items.is_empty());
    }

    #[test]
    fn death_loot_unique_across_deaths() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "a"));
        w.add_player(Player::new(2, "b"));
        w.get_player_mut(1).unwrap().inventory.add_item(1, 1);
        w.get_player_mut(2).unwrap().inventory.add_item(2, 1);

        w.drop_inventory_at(1, Vec3::new(1.0, 0.0, 0.0));
        w.drop_inventory_at(2, Vec3::new(2.0, 0.0, 0.0));

        assert_eq!(w.loot.len(), 2);
        assert_ne!(w.loot[0].id, w.loot[1].id);
        assert!(w.loot.iter().all(|l| l.id >= ids::LOOT_BASE));
    }

    #[test]
    fn creature_kill_produces_one_death_event() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.position = Vec3::new(0.0, 0.8, 0.0);
        p.health = 1.0;
        p.inventory.add_item(1, 1);
        w.add_player(p);
        w.creatures.push(Creature {
            id: 1,
            position: Vec3::new(0.5, 0.8, 0.0),
            health: 50.0,
            max_health: 50.0,
            state: CreatureState::Attack,
            target: Some(1),
            damage: 100.0,
        });

        w.tick(1.0 / 60.0);

        let deaths = w
            .events
            .iter()
            .filter(|e| matches!(e, GameEvent::PlayerDeath { player_id: 1, .. }))
            .count();
        assert_eq!(
            deaths, 1,
            "смерть от существа должна дать ровно одно событие"
        );
        assert!(w.loot.iter().any(|l| l.id >= ids::LOOT_BASE));
        assert_eq!(w.get_player(1).unwrap().health, 100.0);
    }

    #[test]
    fn death_emits_event_and_drops_inventory() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.inventory.add_item(1, 1);
        p.health = 0.0;
        w.add_player(p);

        w.tick(1.0 / 60.0);

        assert!(
            w.events
                .iter()
                .any(|e| matches!(e, GameEvent::PlayerDeath { player_id: 1, .. }))
        );
        assert!(w.loot.iter().any(|l| l.id >= ids::LOOT_BASE));
        assert!(w.get_player(1).unwrap().inventory.items.is_empty());
    }
}
