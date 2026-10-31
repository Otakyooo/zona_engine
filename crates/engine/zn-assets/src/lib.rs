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

/// Строит цепочку мипмапов усреднением блоков 2x2 (box filter).
///
/// Диапазоны источника считаются как `x0 = x*w/nw .. (x+1)*w/nw`, поэтому
/// нечётные размеры не теряют столбцы и не выходят за границы. Усреднение
/// идёт в sRGB-байтах, что слегка затемняет мипы — для наших материалов
/// это допустимо и заметно меньше, чем алиасинг nearest-neighbour.
pub fn generate_mips(rgba: &[u8], width: u32, height: u32) -> Vec<Vec<u8>> {
    let mut levels = Vec::new();
    if width == 0 || height == 0 {
        return levels;
    }
    levels.push(rgba.to_vec());

    let mut w = width;
    let mut h = height;
    while w > 1 || h > 1 {
        let prev = levels.last().expect("уровень только что добавлен");
        let nw = (w / 2).max(1);
        let nh = (h / 2).max(1);
        let mut next = vec![0u8; (nw * nh * 4) as usize];

        for y in 0..nh {
            let y0 = (y * h / nh) as usize;
            let y1 = (((y + 1) * h / nh) as usize).clamp(y0 + 1, h as usize);
            for x in 0..nw {
                let x0 = (x * w / nw) as usize;
                let x1 = (((x + 1) * w / nw) as usize).clamp(x0 + 1, w as usize);
                let mut sum = [0u32; 4];
                let mut count = 0u32;
                for sy in y0..y1 {
                    for sx in x0..x1 {
                        let idx = (sy * w as usize + sx) * 4;
                        for (c, acc) in sum.iter_mut().enumerate() {
                            *acc += prev[idx + c] as u32;
                        }
                        count += 1;
                    }
                }
                let nidx = (y as usize * nw as usize + x as usize) * 4;
                for (c, acc) in sum.iter().enumerate() {
                    next[nidx + c] = ((acc + count / 2) / count) as u8;
                }
            }
        }

        levels.push(next);
        w = nw;
        h = nh;
    }
    levels
}

pub fn load_texture(path: &std::path::Path) -> Result<LoadedTexture, Box<dyn std::error::Error>> {
    let img = image::open(path)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mip_levels = generate_mips(&rgba.into_raw(), w, h);
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

    fn expected_chain_len(mut w: u32, mut h: u32) -> usize {
        let mut n = 1;
        while w > 1 || h > 1 {
            w = (w / 2).max(1);
            h = (h / 2).max(1);
            n += 1;
        }
        n
    }

    #[test]
    fn mip_chain_length() {
        let square = generate_mips(&vec![128u8; 512 * 512 * 4], 512, 512);
        assert_eq!(square.len(), 10);
        assert_eq!(square.last().unwrap().len(), 4);

        let levels = generate_mips(&vec![64u8; 100 * 60 * 4], 100, 60);
        assert_eq!(levels.len(), expected_chain_len(100, 60));
        assert_eq!(levels.last().unwrap().len(), 4);

        let mut w = 100u32;
        let mut h = 60u32;
        for lvl in &levels {
            assert_eq!(lvl.len(), (w * h * 4) as usize);
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
    }

    #[test]
    fn mip_box_average_preserved() {
        let px = [10u8, 20, 30, 40];
        let mut uniform = Vec::new();
        for _ in 0..16 {
            uniform.extend_from_slice(&px);
        }
        for lvl in generate_mips(&uniform, 4, 4) {
            assert!(lvl.chunks(4).all(|c| c == px.as_slice()));
        }

        // шахматка 2x2: среднее 127.5 округляется до 128
        let checker = vec![
            0u8, 0, 0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0, 0, 0, 255,
        ];
        let levels = generate_mips(&checker, 2, 2);
        assert_eq!(levels.len(), 2);
        assert_eq!(levels[1], vec![128, 128, 128, 255]);
    }

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
