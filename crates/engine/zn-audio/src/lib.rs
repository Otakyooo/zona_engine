//! zn-audio: звук на kira.
//!
//! Клипы синтезируются в памяти при старте — внешних звуковых файлов нет.
//! Так же поступили с текстурами: ноль ассетов и лицензионных вопросов.
//! Синтез детерминированный (свой PRNG с фиксированным seed), поэтому тесты
//! не зависят от случайности.

use kira::manager::AudioManagerSettings;
use kira::manager::backend::DefaultBackend;
use kira::sound::static_sound::{StaticSoundData, StaticSoundSettings};
use kira::{Frame, Volume};
use std::sync::Arc;

/// Частота дискретизации синтезированных клипов.
pub const SAMPLE_RATE: u32 = 44_100;

/// Все звуки игры.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sfx {
    Shot,
    Hit,
    Pickup,
    Hurt,
    Step,
    Death,
}

/// Количество звуков — размер массива клипов.
pub const SFX_COUNT: usize = 6;

impl Sfx {
    fn index(self) -> usize {
        self as usize
    }
}

struct Clip {
    frames: Arc<[Frame]>,
    /// Базовая громкость клипа, 0..1.
    volume: f32,
}

impl Clip {
    fn new(frames: Arc<[Frame]>, volume: f32) -> Self {
        Self { frames, volume }
    }
}

pub struct AudioManager {
    pub master_volume: f32,
    /// `None`, если аудиоустройство недоступно: игра должна работать без звука,
    /// а не падать.
    manager: Option<kira::manager::AudioManager<DefaultBackend>>,
    clips: [Clip; SFX_COUNT],
}

impl Default for AudioManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioManager {
    pub fn new() -> Self {
        let manager = match kira::manager::AudioManager::<DefaultBackend>::new(
            AudioManagerSettings::default(),
        ) {
            Ok(m) => Some(m),
            Err(e) => {
                log::warn!("audio disabled, no output device: {e:?}");
                None
            }
        };
        Self {
            master_volume: 0.8,
            manager,
            clips: [
                Clip::new(synth_shot(), 0.9),
                Clip::new(synth_hit(), 0.7),
                Clip::new(synth_pickup(), 0.5),
                Clip::new(synth_hurt(), 0.6),
                Clip::new(synth_step(), 0.22),
                Clip::new(synth_death(), 0.8),
            ],
        }
    }

    /// Есть ли куда играть (устройство открылось).
    pub fn available(&self) -> bool {
        self.manager.is_some()
    }

    /// Проигрывает звук. `gain` — затухание по расстоянию в диапазоне 0..1.
    pub fn play(&mut self, sfx: Sfx, gain: f32) {
        let Some(manager) = self.manager.as_mut() else {
            return;
        };
        let clip = &self.clips[sfx.index()];
        let amplitude = (clip.volume * gain * self.master_volume).clamp(0.0, 1.0) as f64;
        if amplitude <= 0.001 {
            return;
        }
        let data = StaticSoundData {
            sample_rate: SAMPLE_RATE,
            frames: clip.frames.clone(),
            settings: StaticSoundSettings::new().volume(Volume::Amplitude(amplitude)),
            slice: None,
        };
        // Лимит одновременных звуков может быть исчерпан — это не ошибка.
        let _ = manager.play(data);
    }

    /// Затухание по расстоянию: 1 вплотную, 0 дальше `max_dist`.
    pub fn distance_gain(distance: f32, max_dist: f32) -> f32 {
        if max_dist <= 0.0 {
            return 0.0;
        }
        (1.0 - distance / max_dist).clamp(0.0, 1.0)
    }
}

/// Детерминированный шум в диапазоне -1..1.
fn xorshift(seed: &mut u32) -> f32 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 17;
    *seed ^= *seed << 5;
    (*seed as f32 / u32::MAX as f32) * 2.0 - 1.0
}

fn to_frames(samples: Vec<f32>) -> Arc<[Frame]> {
    samples
        .into_iter()
        .map(|s| {
            let s = s.clamp(-1.0, 1.0);
            Frame { left: s, right: s }
        })
        .collect::<Vec<_>>()
        .into()
}

/// Выстрел: шумовой всплеск с быстрым спадом плюс низкий «толчок».
fn synth_shot() -> Arc<[Frame]> {
    let n = (SAMPLE_RATE as f32 * 0.18) as usize;
    let mut seed = 0x1234_5678u32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let env = (-t * 28.0).exp();
        let noise = xorshift(&mut seed) * env;
        let thump = (t * 90.0 * std::f32::consts::TAU).sin() * (-t * 14.0).exp() * 0.6;
        out.push((noise * 0.8 + thump) * 0.9);
    }
    to_frames(out)
}

/// Попадание: короткий высокий сигнал.
fn synth_hit() -> Arc<[Frame]> {
    let n = (SAMPLE_RATE as f32 * 0.07) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let env = (-t * 55.0).exp();
        let tone = (t * 900.0 * std::f32::consts::TAU).sin();
        out.push(tone * env * 0.8);
    }
    to_frames(out)
}

/// Подбор: два коротких тона вверх.
fn synth_pickup() -> Arc<[Frame]> {
    let n = (SAMPLE_RATE as f32 * 0.14) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let env = (-t * 12.0).exp();
        let freq = if t < 0.07 { 660.0 } else { 990.0 };
        out.push((t * freq * std::f32::consts::TAU).sin() * env * 0.6);
    }
    to_frames(out)
}

/// Урон: низкий гудящий сигнал с шумом.
fn synth_hurt() -> Arc<[Frame]> {
    let n = (SAMPLE_RATE as f32 * 0.22) as usize;
    let mut seed = 0x0BAD_F00Du32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let env = (-t * 9.0).exp();
        let tone = (t * 140.0 * std::f32::consts::TAU).sin() * 0.7;
        let noise = xorshift(&mut seed) * 0.3;
        out.push((tone + noise) * env * 0.8);
    }
    to_frames(out)
}

/// Шаг: тихий короткий шорох.
fn synth_step() -> Arc<[Frame]> {
    let n = (SAMPLE_RATE as f32 * 0.05) as usize;
    let mut seed = 0x00C0_FFEEu32;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let env = (-t * 60.0).exp();
        out.push(xorshift(&mut seed) * env * 0.5);
    }
    to_frames(out)
}

/// Смерть: нисходящий тон.
fn synth_death() -> Arc<[Frame]> {
    let n = (SAMPLE_RATE as f32 * 0.5) as usize;
    let mut out = Vec::with_capacity(n);
    let mut phase = 0.0f32;
    for i in 0..n {
        let t = i as f32 / SAMPLE_RATE as f32;
        let freq = 300.0 - 220.0 * (t / 0.5);
        phase += freq / SAMPLE_RATE as f32;
        let env = (-t * 3.0).exp();
        out.push((phase * std::f32::consts::TAU).sin() * env * 0.7);
    }
    to_frames(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_clip_has_samples() {
        for (name, frames) in [
            ("shot", synth_shot()),
            ("hit", synth_hit()),
            ("pickup", synth_pickup()),
            ("hurt", synth_hurt()),
            ("step", synth_step()),
            ("death", synth_death()),
        ] {
            assert!(!frames.is_empty(), "клип {name} пустой");
        }
    }

    #[test]
    fn samples_stay_in_range() {
        for frames in [synth_shot(), synth_hurt(), synth_death()] {
            for f in frames.iter() {
                assert!(
                    (-1.0..=1.0).contains(&f.left) && (-1.0..=1.0).contains(&f.right),
                    "сэмпл вне диапазона: {}",
                    f.left
                );
            }
        }
    }

    #[test]
    fn distance_gain_falls_off() {
        assert_eq!(AudioManager::distance_gain(0.0, 20.0), 1.0);
        assert_eq!(AudioManager::distance_gain(20.0, 20.0), 0.0);
        assert_eq!(AudioManager::distance_gain(30.0, 20.0), 0.0);
        let mid = AudioManager::distance_gain(10.0, 20.0);
        assert!((mid - 0.5).abs() < 1e-6, "ожидали 0.5, получили {mid}");
    }

    #[test]
    fn synthesis_is_deterministic() {
        assert_eq!(synth_shot(), synth_shot());
    }
}
