//! Генерация базовой геометрии.
//!
//! Кубы строятся по 4 вершины на грань (всего 24), а не по 8 общих: только так
//! у каждой грани могут быть собственные нормали и UV. UV считаются в метрах
//! (`размер_грани / метров_на_тайл`), поэтому плотность текселей одинакова на
//! полу 50×50 и на бордюре 50×2.

use glam::Vec3;
use zn_scene::Transform;

use crate::Vertex;

/// Параллелепипед по трансформу. `t.position` — центр, `t.scale` — полные размеры.
pub fn make_box(t: &Transform, color: [f32; 3], tile_m: f32) -> (Vec<Vertex>, Vec<u32>) {
    let tile = if tile_m > 0.0 { tile_m } else { 1.0 };
    let h = t.scale.abs() * 0.5;

    // (нормаль, ось u, ось v), причём u × v = n — это задаёт обход CCW снаружи.
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, -Vec3::Z, Vec3::Y),
        (-Vec3::X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::X, -Vec3::Z),
        (-Vec3::Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (-Vec3::Z, -Vec3::X, Vec3::Y),
    ];

    let mut verts = Vec::with_capacity(24);
    let mut idx = Vec::with_capacity(36);

    for (n, u_axis, v_axis) in faces {
        let extent_u = 2.0 * h.dot(u_axis.abs());
        let extent_v = 2.0 * h.dot(v_axis.abs());
        let center = n * h.dot(n.abs());
        let base = verts.len() as u32;

        for (s, t_v) in [(0.0f32, 0.0f32), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            let local =
                center + u_axis * ((s - 0.5) * extent_u) + v_axis * ((t_v - 0.5) * extent_v);
            let world = t.position + t.rotation * local;
            let normal = t.rotation * n;
            verts.push(Vertex {
                position: [world.x, world.y, world.z],
                normal: [normal.x, normal.y, normal.z],
                color,
                uv: [s * extent_u / tile, t_v * extent_v / tile],
            });
        }

        idx.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    (verts, idx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn unit_box() -> (Vec<Vertex>, Vec<u32>) {
        make_box(&Transform::default(), [1.0, 1.0, 1.0], 1.0)
    }

    #[test]
    fn box_has_24_verts_36_indices() {
        let (v, i) = unit_box();
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
        let (v, _) = unit_box();
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
        let (v, _) = unit_box();
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
}
