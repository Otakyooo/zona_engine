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
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut has_normals = false;
    let mut indices = Vec::new();
    let mut has_colors = false;
    let mut has_uvs = false;
    let mut color_buf: Vec<[f32; 3]> = Vec::new();
    let mut uv_buf: Vec<[f32; 2]> = Vec::new();

    for mesh in document.meshes() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buf| buffers.get(buf.index()).map(|b| &b[..]));
            let base = positions.len() as u32;
            if let Some(pos_iter) = reader.read_positions() {
                for p in pos_iter {
                    positions.push(p);
                }
            }
            let nrm: Option<Vec<[f32; 3]>> = reader
                .read_normals()
                .map(|n| n.map(|nn| [nn[0], nn[1], nn[2]]).collect());
            if let Some(ns) = nrm {
                if !has_normals {
                    // pad previous
                    while normals.len() < positions.len() - ns.len() {
                        normals.push([0.0, 1.0, 0.0]);
                    }
                }
                normals.extend(ns);
                has_normals = true;
            }
            let col: Option<Vec<[f32; 3]>> = reader.read_colors(0).map(|c| {
                c.into_rgb_f32()
                    .map(|rgb| [rgb[0], rgb[1], rgb[2]])
                    .collect()
            });
            if let Some(cs) = col {
                if !has_colors {
                    color_buf.resize(positions.len() - cs.len(), [1.0, 1.0, 1.0]);
                }
                color_buf.extend(cs);
                has_colors = true;
            }
            let uv: Option<Vec<[f32; 2]>> = reader
                .read_tex_coords(0)
                .map(|t| t.into_f32().map(|uv| [uv[0], uv[1]]).collect());
            if let Some(us) = uv {
                if !has_uvs {
                    uv_buf.resize(positions.len() - us.len(), [0.0, 0.0]);
                }
                uv_buf.extend(us);
                has_uvs = true;
            }
            if let Some(idx_iter) = reader.read_indices() {
                for i in idx_iter.into_u32() {
                    indices.push(i + base);
                }
            } else {
                let count = positions.len() as u32 - base;
                for i in 0..count {
                    indices.push(base + i);
                }
            }
        }
    }

    if positions.is_empty() {
        return Err("no mesh in gltf".into());
    }
    let normals_opt = if has_normals {
        // pad to positions length
        while normals.len() < positions.len() {
            normals.push([0.0, 1.0, 0.0]);
        }
        Some(normals)
    } else {
        None
    };
    let colors_opt = if has_colors {
        while color_buf.len() < positions.len() {
            color_buf.push([1.0, 1.0, 1.0]);
        }
        Some(color_buf)
    } else {
        None
    };
    let uvs_opt = if has_uvs {
        while uv_buf.len() < positions.len() {
            uv_buf.push([0.0, 0.0]);
        }
        Some(uv_buf)
    } else {
        None
    };

    Ok(LoadedMesh {
        positions,
        normals: normals_opt,
        indices,
        colors: colors_opt,
        uvs: uvs_opt,
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
