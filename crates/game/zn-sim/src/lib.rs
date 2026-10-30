//! zn-sim: общий код симуляции (клиент и сервер).

use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputCommand {
    pub seq: u32,
    pub yaw: f32,
    pub pitch: f32,
    pub move_x: f32,
    pub move_z: f32,
    pub dt: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub id: u64,
    pub name: String,
    pub position: Vec3,
    pub health: f32,
    pub max_health: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub velocity: Vec3,
}

impl Player {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            position: Vec3::ZERO,
            health: 100.0,
            max_health: 100.0,
            yaw: 0.0,
            pitch: 0.0,
            velocity: Vec3::ZERO,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerState {
    pub id: u64,
    pub name: String,
    pub position: Vec3,
    pub health: f32,
    pub yaw: f32,
    pub pitch: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub players: Vec<PlayerState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub name: String,
    pub value: u32,
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
}

impl PlayerProfile {
    pub fn new(user_id: i64, username: impl Into<String>) -> Self {
        Self {
            user_id,
            username: username.into(),
            level: 1,
            xp: 0,
            currency: 0,
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

#[derive(Default)]
pub struct SimWorld {
    pub tick: u64,
    pub players: Vec<Player>,
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
            let forward = Vec3::new(cmd.yaw.sin(), 0.0, cmd.yaw.cos());
            let right = Vec3::new(forward.z, 0.0, -forward.x);
            let mut vel = forward * (-cmd.move_z) + right * cmd.move_x;
            vel *= 5.0;
            p.velocity.x = vel.x;
            p.velocity.z = vel.z;
        }
    }
    pub fn tick(&mut self, dt: f32) {
        for p in self.players.iter_mut() {
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
        }
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
                    yaw: p.yaw,
                    pitch: p.pitch,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_input_moves_player() {
        let mut world = SimWorld::new();
        world.add_player(Player::new(1, "p"));
        let cmd = InputCommand {
            seq: 1,
            yaw: 0.0,
            pitch: 0.0,
            move_x: 0.0,
            move_z: -1.0,
            dt: 0.1,
        };
        world.apply_input(1, &cmd);
        world.tick(0.1);
        let p = world.get_player(1).unwrap();
        assert!(p.position.z > 0.0, "pos z = {}", p.position.z);
    }

    #[test]
    fn gravity_pulls_down() {
        let mut world = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.position.y = 5.0;
        world.add_player(p);
        for _ in 0..10 {
            world.tick(0.1);
        }
        let p = world.get_player(1).unwrap();
        assert!(p.position.y <= 0.8 + 0.01, "pos y = {}", p.position.y);
    }

    #[test]
    fn clamp_arena() {
        let mut world = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.position.x = 100.0;
        world.add_player(p);
        world.tick(0.1);
        let p = world.get_player(1).unwrap();
        assert!(p.position.x <= 19.0);
    }

    #[test]
    fn snapshot_contains_player() {
        let mut world = SimWorld::new();
        world.add_player(Player::new(1, "p"));
        let snap = world.snapshot();
        assert_eq!(snap.players.len(), 1);
        assert_eq!(snap.players[0].id, 1);
    }

    #[test]
    fn add_xp_levels_up() {
        let mut prof = PlayerProfile::new(1, "p");
        prof.add_xp(1500);
        assert_eq!(prof.level, 2);
        assert_eq!(prof.xp, 1500);
    }
}
