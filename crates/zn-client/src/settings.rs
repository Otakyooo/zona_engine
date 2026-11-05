//! Пользовательские настройки клиента.
//!
//! Файл лежит рядом с `player.txt` (в `%APPDATA%\ZONA` для установленной игры
//! или в рабочем каталоге при `cargo run`). Значения применяются сразу, но
//! записываются только при выходе из экрана настроек: иначе перетаскивание
//! слайдера писало бы файл каждый кадр.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const SENSITIVITY_MIN: f32 = 0.2;
pub const SENSITIVITY_MAX: f32 = 3.0;
pub const FOV_MIN: f32 = 60.0;
pub const FOV_MAX: f32 = 110.0;
pub const RENDER_DISTANCE_MIN: f32 = 60.0;
pub const RENDER_DISTANCE_MAX: f32 = 300.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub mouse_sensitivity: f32,
    /// Вертикальный угол обзора в градусах.
    pub fov_deg: f32,
    /// Дальность отрисовки и дистанция тумана, метры.
    pub render_distance: f32,
    pub master_volume: f32,
    pub vsync: bool,
    pub invert_y: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mouse_sensitivity: 1.0,
            fov_deg: 75.0,
            render_distance: 120.0,
            master_volume: 0.8,
            vsync: true,
            invert_y: false,
        }
    }
}

impl Settings {
    /// Ограничивает значения после ручного редактирования RON.
    pub fn clamped(mut self) -> Self {
        self.mouse_sensitivity = self
            .mouse_sensitivity
            .clamp(SENSITIVITY_MIN, SENSITIVITY_MAX);
        self.fov_deg = self.fov_deg.clamp(FOV_MIN, FOV_MAX);
        self.render_distance = self
            .render_distance
            .clamp(RENDER_DISTANCE_MIN, RENDER_DISTANCE_MAX);
        self.master_volume = self.master_volume.clamp(0.0, 1.0);
        self
    }

    pub fn fov_radians(&self) -> f32 {
        self.fov_deg.to_radians()
    }

    /// `None`, если файла ещё нет: первый запуск не должен считать это ошибкой.
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        ron::from_str::<Self>(&text).ok().map(Self::clamped)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| e.to_string())?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamping_handles_broken_values() {
        let s = Settings {
            mouse_sensitivity: -5.0,
            fov_deg: 500.0,
            render_distance: 0.0,
            master_volume: 9.0,
            ..Settings::default()
        }
        .clamped();
        assert_eq!(s.mouse_sensitivity, SENSITIVITY_MIN);
        assert_eq!(s.fov_deg, FOV_MAX);
        assert_eq!(s.render_distance, RENDER_DISTANCE_MIN);
        assert_eq!(s.master_volume, 1.0);
    }

    #[test]
    fn ron_roundtrip_preserves_values() {
        let s = Settings {
            mouse_sensitivity: 1.7,
            fov_deg: 90.0,
            render_distance: 240.0,
            master_volume: 0.35,
            vsync: false,
            invert_y: true,
        };
        let text = ron::ser::to_string(&s).unwrap();
        let decoded: Settings = ron::from_str(&text).unwrap();
        assert_eq!(decoded, s);
    }

    #[test]
    fn missing_file_is_not_an_error() {
        assert!(Settings::load(Path::new("__zona_no_such_settings__.ron")).is_none());
    }
}
