//! zn-scene: граф сцены, трансформы, стриминг чанков.

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.position)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SceneNodeFile {
    pub name: String,
    pub transform: Transform,
    pub mesh: Option<String>,
    pub color: [f32; 3],
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SceneFile {
    pub nodes: Vec<SceneNodeFile>,
}

pub fn save_scene(path: &Path, scene: &SceneFile) -> Result<(), Box<dyn std::error::Error>> {
    let s = ron::ser::to_string_pretty(scene, ron::ser::PrettyConfig::default())?;
    fs::write(path, s)?;
    Ok(())
}

pub fn load_scene(path: &Path) -> Result<SceneFile, Box<dyn std::error::Error>> {
    let s = fs::read_to_string(path)?;
    let scene: SceneFile = ron::from_str(&s)?;
    Ok(scene)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkCoord {
    pub x: i32,
    pub z: i32,
}

pub fn chunk_of(pos: Vec3, chunk_size: f32) -> ChunkCoord {
    ChunkCoord {
        x: (pos.x / chunk_size).floor() as i32,
        z: (pos.z / chunk_size).floor() as i32,
    }
}

pub fn select_lod(distance: f32, thresholds: &[f32]) -> usize {
    for (i, t) in thresholds.iter().enumerate() {
        if distance <= *t {
            return i;
        }
    }
    thresholds.len()
}

#[derive(Clone, Debug)]
pub struct SceneNode {
    pub name: String,
    pub transform: Transform,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

#[derive(Default)]
pub struct Scene {
    pub nodes: Vec<SceneNode>,
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add_node(&mut self, node: SceneNode) -> usize {
        let idx = self.nodes.len();
        self.nodes.push(node);
        idx
    }
}

#[derive(Default)]
pub struct ChunkManager {
    pub chunk_size: f32,
    pub loaded: HashMap<ChunkCoord, Vec<usize>>,
}

impl ChunkManager {
    pub fn new(chunk_size: f32) -> Self {
        Self {
            chunk_size,
            loaded: HashMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum ChunkEvent {
    Loaded {
        coord: ChunkCoord,
        nodes: Vec<SceneNodeFile>,
    },
    Unloaded {
        coord: ChunkCoord,
    },
}

pub async fn load_chunk_async(coord: ChunkCoord, _chunk_size: f32) -> ChunkEvent {
    // placeholder: return empty
    ChunkEvent::Loaded {
        coord,
        nodes: Vec::new(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ron_roundtrip() {
        let scene = SceneFile {
            nodes: vec![SceneNodeFile {
                name: "cube".into(),
                transform: Transform::default(),
                mesh: Some("cube.glb".into()),
                color: [1.0, 0.0, 0.0],
            }],
        };
        let dir = std::env::temp_dir();
        let path = dir.join("zona_test_scene.ron");
        save_scene(&path, &scene).unwrap();
        let loaded = load_scene(&path).unwrap();
        assert_eq!(loaded.nodes.len(), 1);
        assert_eq!(loaded.nodes[0].name, "cube");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn chunk_of_basic() {
        let c = chunk_of(Vec3::new(5.0, 0.0, -1.0), 32.0);
        assert_eq!(c.x, 0);
        assert_eq!(c.z, -1);
    }

    #[test]
    fn select_lod_thresholds() {
        let t = [10.0, 30.0];
        assert_eq!(select_lod(5.0, &t), 0);
        assert_eq!(select_lod(15.0, &t), 1);
        assert_eq!(select_lod(50.0, &t), 2);
    }
}
