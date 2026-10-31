//! zn-sim: общий код симуляции (клиент и сервер).

use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
}

impl SimWorld {
    pub fn new() -> Self {
        Self::default()
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

            if p.health <= 0.0 {
                p.health = 0.0;
                dead_players.push(p.id);
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

        // handle deaths
        for pid in &dead_players {
            let dropped_loot = Vec::new();
            let dropped_consumables = Vec::new();
            // For MVP: drop all loot items near player? We'll just emit event
            self.events.push(GameEvent::PlayerDeath {
                player_id: *pid,
                dropped_loot,
                dropped_consumables,
            });
            if let Some(p) = self.get_player_mut(*pid) {
                p.health = p.max_health;
                p.position = Vec3::ZERO;
                p.radiation = 0.0;
                p.hunger = 100.0;
                p.thirst = 100.0;
            }
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
}
