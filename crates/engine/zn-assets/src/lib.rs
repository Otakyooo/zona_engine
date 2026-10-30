//! zn-assets: импорт glTF, KTX2/Basis, кэш, hot-reload.

use std::collections::HashMap;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct LoadedMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Option<Vec<[f32; 3]>>,
    pub indices: Vec<u32>,
    pub colors: Option<Vec<[f32; 3]>>,
    pub uvs: Option<Vec<[f32; 2]>>,
}

#[derive(Default)]
pub struct AssetCache {
    meshes: HashMap<String, LoadedMesh>,
}

impl AssetCache {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn get_mesh(&self, key: &str) -> Option<&LoadedMesh> {
        self.meshes.get(key)
    }
    pub fn insert_mesh(&mut self, key: String, mesh: LoadedMesh) {
        self.meshes.insert(key, mesh);
    }
}

pub fn load_gltf(path: &Path) -> Result<LoadedMesh, Box<dyn std::error::Error>> {
    let (document, buffers, _images) = gltf::import(path)?;
    let mesh = document.meshes().next().ok_or("no mesh in gltf")?;
    let primitive = mesh.primitives().next().ok_or("no primitive")?;
    let reader = primitive.reader(|buf| buffers.get(buf.index()).map(|b| &b[..]));

    let mut positions = Vec::new();
    if let Some(pos_iter) = reader.read_positions() {
        for p in pos_iter {
            positions.push(p);
        }
    }
    let normals = reader
        .read_normals()
        .map(|n| n.map(|nn| [nn[0], nn[1], nn[2]]).collect());
    let mut indices = Vec::new();
    if let Some(idx_iter) = reader.read_indices() {
        for i in idx_iter.into_u32() {
            indices.push(i);
        }
    }
    let colors = reader.read_colors(0).map(|c| {
        c.into_rgb_f32()
            .map(|rgb| [rgb[0], rgb[1], rgb[2]])
            .collect()
    });
    let uvs = reader
        .read_tex_coords(0)
        .map(|t| t.into_f32().map(|uv| [uv[0], uv[1]]).collect());

    Ok(LoadedMesh {
        positions,
        normals,
        indices,
        colors,
        uvs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_insert_get() {
        let mut c = AssetCache::new();
        c.insert_mesh(
            "k".into(),
            LoadedMesh {
                positions: vec![[0.0, 0.0, 0.0]],
                normals: None,
                indices: vec![],
                colors: None,
                uvs: None,
            },
        );
        assert!(c.get_mesh("k").is_some());
    }
}
