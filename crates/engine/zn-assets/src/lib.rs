//! zn-assets: импорт glTF, KTX2/Basis, кэш, hot-reload.

use std::collections::HashMap;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
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
    textures: HashMap<String, LoadedTexture>,
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
    pub fn get_texture(&self, key: &str) -> Option<&LoadedTexture> {
        self.textures.get(key)
    }
    pub fn insert_texture(&mut self, key: String, tex: LoadedTexture) {
        self.textures.insert(key, tex);
    }
}

#[derive(Clone, Debug)]
pub struct LoadedTexture {
    pub width: u32,
    pub height: u32,
    pub mip_levels: Vec<Vec<u8>>,
}

pub fn load_texture(path: &std::path::Path) -> Result<LoadedTexture, Box<dyn std::error::Error>> {
    let img = image::open(path)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut mip_levels = Vec::new();
    mip_levels.push(rgba.into_raw());
    // generate mipmaps by halving
    let mut cur_w = w;
    let mut cur_h = h;
    while cur_w > 1 || cur_h > 1 {
        let nw = (cur_w / 2).max(1);
        let nh = (cur_h / 2).max(1);
        // simple nearest-neighbor downscale from last level
        let prev = mip_levels.last().unwrap();
        let pw = cur_w as usize;
        let ph = cur_h as usize;
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                let sx = ((x * 2) as usize).min(pw - 1);
                let sy = ((y * 2) as usize).min(ph - 1);
                let idx = (sy * pw + sx) * 4;
                let nidx = (y as usize * nw as usize + x as usize) * 4;
                next[nidx] = prev[idx];
                next[nidx + 1] = prev[idx + 1];
                next[nidx + 2] = prev[idx + 2];
                next[nidx + 3] = prev[idx + 3];
            }
        }
        mip_levels.push(next);
        cur_w = nw;
        cur_h = nh;
    }
    Ok(LoadedTexture {
        width: w,
        height: h,
        mip_levels,
    })
}
pub fn load_gltf(path: &std::path::Path) -> Result<LoadedMesh, Box<dyn std::error::Error>> {
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
