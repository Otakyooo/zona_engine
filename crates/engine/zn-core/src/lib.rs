//! zn-core: ECS-слой поверх hecs, шедулер, fixed timestep, event bus.

use hecs::World;
use std::time::Duration;

pub struct Engine {
    pub world: World,
    accumulator: Duration,
    fixed_dt: Duration,
}

impl Engine {
    pub fn new(fixed_hz: u32) -> Self {
        Self {
            world: World::new(),
            accumulator: Duration::ZERO,
            fixed_dt: Duration::from_secs_f64(1.0 / fixed_hz as f64),
        }
    }

    pub fn fixed_dt(&self) -> Duration {
        self.fixed_dt
    }

    /// Накапливает время и вызывает `step` нужное число тиков.
    pub fn advance<F: FnMut(&mut World, Duration)>(&mut self, elapsed: Duration, mut step: F) {
        self.accumulator += elapsed;
        while self.accumulator >= self.fixed_dt {
            step(&mut self.world, self.fixed_dt);
            self.accumulator -= self.fixed_dt;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_timestep_accumulates() {
        let mut e = Engine::new(60);
        let mut ticks = 0;
        e.advance(Duration::from_millis(50), |_w, _dt| {
            ticks += 1;
        });
        // 50ms ~ 3 ticks at 60hz (16.67ms each)
        assert!((2..=4).contains(&ticks), "ticks={}", ticks);
    }
}
