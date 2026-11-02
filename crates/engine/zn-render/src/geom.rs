//! Генерация базовой геометрии.
//!
//! Кубы строятся по 4 вершины на грань (всего 24), а не по 8 общих: только так
//! у каждой грани могут быть собственные нормали и UV. UV считаются в метрах
//! (`размер_грани / метров_на_тайл`), поэтому плотность текселей одинакова на
//! полу 50×50 и на бордюре 50×2.
//!
//! Силуэты (`humanoid_local`, `quadruped_local`) строятся в локальных
//! координатах: ступни в нуле, лицо в +Z. Позиция и поворот приходят матрицей
//! модели, поэтому меш создаётся один раз и переиспользуется всеми экземплярами.

use glam::{Quat, Vec3};
use zn_scene::Transform;

use crate::Vertex;

/// Накопитель геометрии одного меша.
#[derive(Default)]
struct MeshData {
    verts: Vec<Vertex>,
    idx: Vec<u32>,
}

impl MeshData {
    fn with_capacity(boxes: usize) -> Self {
        Self {
            verts: Vec::with_capacity(boxes * 24),
            idx: Vec::with_capacity(boxes * 36),
        }
    }

    /// Один бокс: `origin` и `rotation` переносят локальные координаты в мир.
    fn push_box(
        &mut self,
        origin: Vec3,
        rotation: Quat,
        center: Vec3,
        half: Vec3,
        color: [f32; 3],
        tile: f32,
    ) {
        let half = half.abs();
        // (нормаль, ось u, ось v), причём u × v = n — это задаёт обход CCW снаружи.
        let faces: [(Vec3, Vec3, Vec3); 6] = [
            (Vec3::X, -Vec3::Z, Vec3::Y),
            (-Vec3::X, Vec3::Z, Vec3::Y),
            (Vec3::Y, Vec3::X, -Vec3::Z),
            (-Vec3::Y, Vec3::X, Vec3::Z),
            (Vec3::Z, Vec3::X, Vec3::Y),
            (-Vec3::Z, -Vec3::X, Vec3::Y),
        ];

        for (n, u_axis, v_axis) in faces {
            let extent_u = 2.0 * half.dot(u_axis.abs());
            let extent_v = 2.0 * half.dot(v_axis.abs());
            let face_center = center + n * half.dot(n.abs());
            let base = self.verts.len() as u32;

            for (s, t_v) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                let local = face_center
                    + u_axis * ((s - 0.5) * extent_u)
                    + v_axis * ((t_v - 0.5) * extent_v);
                let world = origin + rotation * local;
                let normal = rotation * n;
                self.verts.push(Vertex {
                    position: [world.x, world.y, world.z],
                    normal: [normal.x, normal.y, normal.z],
                    color,
                    uv: [s * extent_u / tile, t_v * extent_v / tile],
                });
            }

            self.idx
                .extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }

    fn finish(self) -> (Vec<Vertex>, Vec<u32>) {
        (self.verts, self.idx)
    }
}

fn tile_or_default(tile_m: f32) -> f32 {
    if tile_m > 0.0 { tile_m } else { 1.0 }
}

/// Параллелепипед по трансформу. `t.position` — центр, `t.scale` — полные размеры.
pub fn make_box(t: &Transform, color: [f32; 3], tile_m: f32) -> (Vec<Vertex>, Vec<u32>) {
    let mut data = MeshData::with_capacity(1);
    data.push_box(
        t.position,
        t.rotation,
        Vec3::ZERO,
        t.scale.abs() * 0.5,
        color,
        tile_or_default(tile_m),
    );
    data.finish()
}

/// Единичный куб от −0.5 до +0.5: дальний LOD и мелкие предметы.
///
/// Масштаб задаётся матрицей модели, поэтому один меш обслуживает лут, аномалии
/// и грубый уровень детализации.
pub fn unit_box(color: [f32; 3], tile_m: f32) -> (Vec<Vertex>, Vec<u32>) {
    let mut data = MeshData::with_capacity(1);
    data.push_box(
        Vec3::ZERO,
        Quat::IDENTITY,
        Vec3::ZERO,
        Vec3::splat(0.5),
        color,
        tile_or_default(tile_m),
    );
    data.finish()
}

/// Части человека: (центр от ступней, полные размеры). Высота ровно 1.8 м,
/// чтобы силуэт совпадал с хитбоксом из `zn-sim` (тело 0…1.55, голова 1.55…1.8).
const HUMANOID_PARTS: [(Vec3, Vec3); 6] = [
    (Vec3::new(-0.1, 0.425, 0.0), Vec3::new(0.16, 0.85, 0.18)),
    (Vec3::new(0.1, 0.425, 0.0), Vec3::new(0.16, 0.85, 0.18)),
    (Vec3::new(0.0, 1.15, 0.0), Vec3::new(0.44, 0.6, 0.24)),
    (Vec3::new(-0.3, 1.175, 0.0), Vec3::new(0.12, 0.55, 0.14)),
    (Vec3::new(0.3, 1.175, 0.0), Vec3::new(0.12, 0.55, 0.14)),
    (Vec3::new(0.0, 1.675, 0.0), Vec3::new(0.26, 0.25, 0.26)),
];

/// Части собаки: высота 1.2 м, длина 1.0 м, морда смотрит в +Z.
const QUADRUPED_PARTS: [(Vec3, Vec3); 6] = [
    (Vec3::new(0.0, 0.7, 0.0), Vec3::new(0.42, 0.5, 1.0)),
    (Vec3::new(0.0, 1.0, 0.55), Vec3::new(0.3, 0.35, 0.35)),
    (Vec3::new(-0.14, 0.225, -0.35), Vec3::new(0.12, 0.45, 0.12)),
    (Vec3::new(0.14, 0.225, -0.35), Vec3::new(0.12, 0.45, 0.12)),
    (Vec3::new(-0.14, 0.225, 0.35), Vec3::new(0.12, 0.45, 0.12)),
    (Vec3::new(0.14, 0.225, 0.35), Vec3::new(0.12, 0.45, 0.12)),
];

fn silhouette(parts: &[(Vec3, Vec3)], color: [f32; 3], tile_m: f32) -> (Vec<Vertex>, Vec<u32>) {
    let tile = tile_or_default(tile_m);
    let mut data = MeshData::with_capacity(parts.len());
    for (center, size) in parts {
        data.push_box(
            Vec3::ZERO,
            Quat::IDENTITY,
            *center,
            *size * 0.5,
            color,
            tile,
        );
    }
    data.finish()
}

/// Человек в локальных координатах: ступни в нуле, высота 1.8 м.
pub fn humanoid_local(color: [f32; 3], tile_m: f32) -> (Vec<Vertex>, Vec<u32>) {
    silhouette(&HUMANOID_PARTS, color, tile_m)
}

/// Собака в локальных координатах: лапы в нуле, высота 1.2 м.
pub fn quadruped_local(color: [f32; 3], tile_m: f32) -> (Vec<Vertex>, Vec<u32>) {
    silhouette(&QUADRUPED_PARTS, color, tile_m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn unit_box_mesh() -> (Vec<Vertex>, Vec<u32>) {
        make_box(&Transform::default(), [1.0, 1.0, 1.0], 1.0)
    }

    fn bounds(verts: &[Vertex]) -> (Vec3, Vec3) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for v in verts {
            let p = Vec3::from(v.position);
            min = min.min(p);
            max = max.max(p);
        }
        (min, max)
    }

    #[test]
    fn box_has_24_verts_36_indices() {
        let (v, i) = unit_box_mesh();
        assert_eq!(v.len(), 24);
        assert_eq!(i.len(), 36);
        assert!(i.iter().all(|x| *x < 24));
    }

    #[test]
    fn box_uv_scales_with_size() {
        // 50 x 2 x 1 при тайле 1 м
        let t = Transform {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::new(50.0, 2.0, 1.0),
        };
        let (v, _) = make_box(&t, [1.0; 3], 1.0);

        // грани идут в порядке +X, -X, +Y, -Y, +Z, -Z
        let top = &v[8..12]; // +Y: протяжённость 50 по X и 1 по Z
        let max_u = top.iter().map(|x| x.uv[0]).fold(0.0f32, f32::max);
        let max_v = top.iter().map(|x| x.uv[1]).fold(0.0f32, f32::max);
        assert!((max_u - 50.0).abs() < 1e-4, "max_u = {max_u}");
        assert!((max_v - 1.0).abs() < 1e-4, "max_v = {max_v}");

        let side = &v[0..4]; // +X: протяжённость 1 по Z и 2 по Y
        let max_u = side.iter().map(|x| x.uv[0]).fold(0.0f32, f32::max);
        let max_v = side.iter().map(|x| x.uv[1]).fold(0.0f32, f32::max);
        assert!((max_u - 1.0).abs() < 1e-4, "max_u = {max_u}");
        assert!((max_v - 2.0).abs() < 1e-4, "max_v = {max_v}");
    }

    #[test]
    fn box_uv_halves_when_tile_doubles() {
        let t = Transform {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::splat(4.0),
        };
        let (one, _) = make_box(&t, [1.0; 3], 1.0);
        let (two, _) = make_box(&t, [1.0; 3], 2.0);
        assert!((one[1].uv[0] - 4.0).abs() < 1e-4);
        assert!((two[1].uv[0] - 2.0).abs() < 1e-4);
    }

    #[test]
    fn box_faces_have_distinct_uvs() {
        let (v, _) = unit_box_mesh();
        for face in 0..6 {
            let uvs: Vec<[f32; 2]> = v[face * 4..face * 4 + 4].iter().map(|x| x.uv).collect();
            for (i, a) in uvs.iter().enumerate() {
                for b in uvs.iter().skip(i + 1) {
                    assert_ne!(a, b, "грань {face} содержит совпадающие UV");
                }
            }
        }
    }

    #[test]
    fn box_normals_axis_aligned() {
        let (v, _) = unit_box_mesh();
        for face in 0..6 {
            for vert in &v[face * 4..face * 4 + 4] {
                let n = Vec3::from(vert.normal);
                assert!((n.length() - 1.0).abs() < 1e-4);
                let axes = [n.x.abs(), n.y.abs(), n.z.abs()];
                let ones = axes.iter().filter(|a| **a > 0.99).count();
                assert_eq!(ones, 1, "нормаль не по оси: {n:?}");
            }
        }
    }

    #[test]
    fn humanoid_is_six_boxes() {
        let (v, i) = humanoid_local([1.0; 3], 1.0);
        assert_eq!(v.len(), 6 * 24, "шесть частей по 24 вершины");
        assert_eq!(i.len(), 6 * 36);
        assert!(i.iter().all(|x| *x < v.len() as u32));
    }

    #[test]
    fn silhouettes_match_hitbox_height() {
        // Высоты совпадают с силуэтами из zn-sim (D16): человек 1.8, собака 1.2.
        let (hv, _) = humanoid_local([1.0; 3], 1.0);
        let (hmin, hmax) = bounds(&hv);
        assert!(hmin.y.abs() < 1e-4, "ступни в нуле, получили {}", hmin.y);
        assert!((hmax.y - 1.8).abs() < 1e-4, "рост 1.8, получили {}", hmax.y);

        let (cv, _) = quadruped_local([1.0; 3], 1.0);
        let (cmin, cmax) = bounds(&cv);
        assert!(cmin.y.abs() < 1e-4, "лапы в нуле, получили {}", cmin.y);
        assert!(
            (cmax.y - 1.2).abs() < 0.03,
            "высота 1.2, получили {}",
            cmax.y
        );
    }

    #[test]
    fn humanoid_is_centred_on_x() {
        let (v, _) = humanoid_local([1.0; 3], 1.0);
        let (min, max) = bounds(&v);
        assert!(
            (min.x + max.x).abs() < 1e-4,
            "силуэт не по центру: min={}, max={}",
            min.x,
            max.x
        );
    }

    #[test]
    fn silhouette_normals_are_unit_axis() {
        let (v, _) = humanoid_local([1.0; 3], 1.0);
        for vert in &v {
            let n = Vec3::from(vert.normal);
            assert!(
                (n.length() - 1.0).abs() < 1e-4,
                "нормаль не единичная: {n:?}"
            );
            let ones = [n.x.abs(), n.y.abs(), n.z.abs()]
                .iter()
                .filter(|a| **a > 0.99)
                .count();
            assert_eq!(ones, 1, "нормаль не по оси: {n:?}");
        }
    }
}
