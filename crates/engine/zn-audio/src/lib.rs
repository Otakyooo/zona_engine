//! zn-audio: 3D-аудио, микшер (kira).

pub struct AudioManager {
    pub master_volume: f32,
}

impl Default for AudioManager {
    fn default() -> Self {
        Self { master_volume: 1.0 }
    }
}

impl AudioManager {
    pub fn new() -> Self {
        Self::default()
    }
}
