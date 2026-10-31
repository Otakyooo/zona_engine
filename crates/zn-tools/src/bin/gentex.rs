//! zn-gentex: генерация бесшовных процедурных текстур для ZONA.
//!
//! Запуск из корня репозитория:
//! `cargo run -p zn-tools --bin zn-gentex`
//!
//! Текстуры бесшовные по построению: решётка шума заворачивается по модулю
//! периода, а все периоды делят размер текстуры, поэтому кадр `u = 0` и
//! `u = 1` попадают в одну и ту же точку решётки.

use std::path::PathBuf;

const SIZE: u32 = 512;

/// Целочисленный хеш (splitmix32) -> [0, 1).
fn hash2(i: i32, j: i32, seed: u32) -> f32 {
    let mut h = seed
        .wrapping_add((i as u32).wrapping_mul(0x9E37_79B9))
        .wrapping_add((j as u32).wrapping_mul(0x85EB_CA6B));
    h ^= h >> 16;
    h = h.wrapping_mul(0x7FEB_352D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846C_A68B);
    h ^= h >> 16;
    (h >> 8) as f32 / 16_777_216.0
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Периодический value-noise. `px`/`py` — число ячеек решётки по осям.
fn vnoise(u: f32, v: f32, px: u32, py: u32, seed: u32) -> f32 {
    let x = u * px as f32;
    let y = v * py as f32;
    let ix = x.floor() as i32;
    let iy = y.floor() as i32;
    let fx = smooth(x - ix as f32);
    let fy = smooth(y - iy as f32);

    let wrap = |i: i32, p: u32| i.rem_euclid(p as i32);
    let p00 = hash2(wrap(ix, px), wrap(iy, py), seed);
    let p10 = hash2(wrap(ix + 1, px), wrap(iy, py), seed);
    let p01 = hash2(wrap(ix, px), wrap(iy + 1, py), seed);
    let p11 = hash2(wrap(ix + 1, px), wrap(iy + 1, py), seed);

    let a = p00 + (p10 - p00) * fx;
    let b = p01 + (p11 - p01) * fx;
    a + (b - a) * fy
}

fn fbm(u: f32, v: f32, px: u32, py: u32, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut norm = 0.0;
    let mut amp = 0.5;
    for o in 0..octaves {
        sum += vnoise(u, v, px << o, py << o, seed.wrapping_add(o * 1013)) * amp;
        norm += amp;
        amp *= 0.5;
    }
    sum / norm
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn speckle(u: f32, v: f32, seed: u32) -> f32 {
    let x = (u * SIZE as f32) as i32;
    let y = (v * SIZE as f32) as i32;
    hash2(x, y, seed) - 0.5
}

fn concrete(u: f32, v: f32, seed: u32) -> [f32; 3] {
    // крупные пятна + мелкий заполнитель, иначе поверхность выглядит мыльной
    let coarse = fbm(u, v, 8, 8, 4, seed);
    let fine = fbm(u, v, 64, 64, 3, seed ^ 0x1111);
    let t = coarse * 0.65 + fine * 0.35 + speckle(u, v, seed ^ 0x5F5F) * 0.14;
    lerp3([0.40, 0.41, 0.40], [0.64, 0.64, 0.62], t)
}

fn metal(u: f32, v: f32, seed: u32) -> [f32; 3] {
    // шлифовка: частота по v выше, чем по u — полосы тянутся вдоль u
    let n = fbm(u, v, 8, 128, 3, seed);
    lerp3([0.38, 0.40, 0.44], [0.55, 0.57, 0.60], n)
}

fn rust(u: f32, v: f32, seed: u32) -> [f32; 3] {
    let base = metal(u, v, seed);
    let mask = smoothstep(0.45, 0.62, fbm(u, v, 4, 4, 4, seed ^ 0x1234));
    let corrosion = lerp3(
        [0.36, 0.20, 0.10],
        [0.55, 0.30, 0.14],
        fbm(u, v, 8, 8, 3, seed ^ 0x0099),
    );
    lerp3(base, corrosion, mask)
}

fn dirt(u: f32, v: f32, seed: u32) -> [f32; 3] {
    let n = fbm(u, v, 8, 8, 5, seed);
    let t = n + speckle(u, v, seed ^ 0x2B2B) * 0.08;
    lerp3([0.30, 0.23, 0.16], [0.45, 0.36, 0.25], t)
}

fn grass(u: f32, v: f32, seed: u32) -> [f32; 3] {
    let patch = fbm(u, v, 4, 4, 3, seed ^ 0x0A0A);
    let n = fbm(u, v, 64, 64, 4, seed);
    let streaks = fbm(u, v, 128, 8, 2, seed ^ 0x0077);
    let t = patch * 0.30 + n * 0.45 + streaks * 0.25;
    lerp3([0.16, 0.30, 0.12], [0.38, 0.52, 0.20], t)
}

fn wood(u: f32, v: f32, seed: u32) -> [f32; 3] {
    let warp = fbm(u, v, 4, 4, 3, seed);
    let rings = (v * 6.0 + warp * 0.8).fract();
    // тёмная прожилка на стыке колец
    let ring = smoothstep(0.0, 0.10, rings) * smoothstep(1.0, 0.90, rings);
    let grain = fbm(u, v, 128, 16, 2, seed ^ 0x0033);
    let t = (0.22 + 0.78 * ring) * (0.85 + 0.30 * grain);
    lerp3([0.36, 0.24, 0.13], [0.60, 0.44, 0.28], t)
}

type Material = (&'static str, u32, fn(f32, f32, u32) -> [f32; 3]);

const MATERIALS: [Material; 6] = [
    ("concrete", 0x1001, concrete),
    ("metal", 0x2002, metal),
    ("rust", 0x3003, rust),
    ("dirt", 0x4004, dirt),
    ("grass", 0x5005, grass),
    ("wood", 0x6006, wood),
];

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/textures".to_string());
    let dir = PathBuf::from(out);
    std::fs::create_dir_all(&dir)?;

    for (name, seed, shade) in MATERIALS {
        let mut buf = Vec::with_capacity((SIZE * SIZE * 3) as usize);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let u = (x as f32 + 0.5) / SIZE as f32;
                let v = (y as f32 + 0.5) / SIZE as f32;
                for c in shade(u, v, seed) {
                    buf.push((c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
                }
            }
        }
        let path = dir.join(format!("{name}.png"));
        image::save_buffer(&path, &buf, SIZE, SIZE, image::ColorType::Rgb8)?;
        println!("{}", path.display());
    }
    Ok(())
}
