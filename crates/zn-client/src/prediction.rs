//! Клиентское предсказание движения с подтверждением ввода.
//!
//! Раньше клиент двигался своей копией арифметики и «догонял» сервер лерпом
//! по расстоянию: ошибка сглаживалась, но причина не исчезала. Теперь клиент
//! применяет ту же функцию `zn_sim::step_body`, что и сервер, хранит
//! неподтверждённые команды и по `last_input_seq` из снапшота переигрывает
//! только их.
//!
//! Реплей идёт шагами `1/60`: сервер применяет ровно один вход на тик, и
//! число шагов должно совпадать, иначе предсказание систематически уезжает
//! вперёд или отстаёт.

use glam::Vec3;
use std::collections::VecDeque;
use zn_sim::{
    Aabb, Bounds, InputCommand, PLAYER_HALF_WIDTH, PLAYER_HEIGHT, desired_velocity, step_body,
};

/// Шаг сервера и предсказания, секунды.
pub const TICK_DT: f32 = 1.0 / 60.0;

/// Ошибка, после которой локальную позицию проще принять серверной, чем
/// догонять её шагами. Меньше 0.5 м — обычная разница одного-двух тиков.
const SNAP_DISTANCE: f32 = 0.5;
/// Расхождение скорости, при котором принимается серверная.
const SNAP_VELOCITY: f32 = 4.0;
/// Сколько команд хранить: три секунды при 60 Гц. Дольше ждать ack
/// бессмысленно — такое соединение уже неиграбельно.
const MAX_PENDING: usize = 180;

pub struct Prediction {
    pub position: Vec3,
    pub velocity: Vec3,
    pending: VecDeque<InputCommand>,
    last_acked: u32,
    corrections: u32,
}

impl Prediction {
    pub fn new(spawn: Vec3) -> Self {
        Self {
            position: spawn,
            velocity: Vec3::ZERO,
            pending: VecDeque::new(),
            last_acked: 0,
            corrections: 0,
        }
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn last_acked(&self) -> u32 {
        self.last_acked
    }

    /// Сколько раз позицию пришлось принимать серверной.
    pub fn corrections(&self) -> u32 {
        self.corrections
    }

    /// Применяет команду локально и запоминает её до подтверждения.
    pub fn predict(&mut self, cmd: InputCommand, bounds: Bounds, obstacles: &[Aabb]) {
        if cmd.seq <= self.last_acked {
            return;
        }
        let mut stored = cmd.clone();
        self.step(&mut stored, bounds, obstacles);
        self.pending.push_back(cmd);
        while self.pending.len() > MAX_PENDING {
            self.pending.pop_front();
        }
    }

    fn step(&mut self, cmd: &mut InputCommand, bounds: Bounds, obstacles: &[Aabb]) {
        let vel = desired_velocity(cmd.yaw, cmd.move_x, cmd.move_z);
        self.velocity.x = vel.x;
        self.velocity.z = vel.z;
        step_body(
            &mut self.position,
            &mut self.velocity,
            TICK_DT,
            bounds,
            obstacles,
            PLAYER_HALF_WIDTH,
            PLAYER_HEIGHT,
        );
    }

    /// Принимает серверное состояние и переигрывает неподтверждённые команды.
    ///
    /// Возвращает `true`, если позицию пришлось принять серверной: это сигнал
    /// для метрики рассинхрона, а не ошибка.
    pub fn reconcile(
        &mut self,
        server_position: Vec3,
        server_velocity: Vec3,
        acked: u32,
        bounds: Bounds,
        obstacles: &[Aabb],
    ) -> bool {
        // Старый снапшот не должен откатывать подтверждение.
        if acked < self.last_acked {
            return false;
        }
        self.last_acked = acked;
        self.pending.retain(|c| c.seq > acked);

        if self.position.distance(server_position) > SNAP_DISTANCE {
            self.position = server_position;
            self.velocity = server_velocity;
            self.corrections += 1;
            let replay: Vec<InputCommand> = self.pending.iter().cloned().collect();
            for mut cmd in replay {
                self.step(&mut cmd, bounds, obstacles);
            }
            return true;
        }

        if self.velocity.distance(server_velocity) > SNAP_VELOCITY {
            self.velocity = server_velocity;
        }
        false
    }

    /// Сбрасывает состояние на телепорт (респавн, смена карты).
    pub fn teleport(&mut self, position: Vec3) {
        self.position = position;
        self.velocity = Vec3::ZERO;
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Bounds {
        Bounds::default()
    }

    fn cmd(seq: u32, move_z: f32) -> InputCommand {
        InputCommand {
            seq,
            yaw: 0.0,
            pitch: 0.0,
            move_x: 0.0,
            move_z,
            dt: TICK_DT,
        }
    }

    #[test]
    fn prediction_moves_forward_with_yaw_zero() {
        let mut p = Prediction::new(Vec3::new(0.0, 0.8, 0.0));
        // yaw=0, move_z=-1 — движение по +X (см. desired_velocity).
        p.predict(cmd(1, -1.0), bounds(), &[]);
        assert!(p.position.x > 0.0, "позиция {:?}", p.position);
        assert_eq!(p.pending_len(), 1);
    }

    #[test]
    fn acked_commands_are_dropped() {
        let mut p = Prediction::new(Vec3::new(0.0, 0.8, 0.0));
        for seq in 1..=5 {
            p.predict(cmd(seq, -1.0), bounds(), &[]);
        }
        p.reconcile(p.position, p.velocity, 3, bounds(), &[]);
        assert_eq!(p.last_acked(), 3);
        assert_eq!(p.pending_len(), 2, "команды 1..=3 подтверждены");
    }

    #[test]
    fn large_error_snaps_and_replays_pending() {
        let mut p = Prediction::new(Vec3::new(0.0, 0.8, 0.0));
        p.predict(cmd(1, -1.0), bounds(), &[]);
        let server = Vec3::new(5.0, 0.8, 0.0);

        let corrected = p.reconcile(server, Vec3::ZERO, 0, bounds(), &[]);

        assert!(corrected, "ошибка 5 м — повод принять серверную позицию");
        // Одна неподтверждённая команда переиграна: сдвиг на один тик.
        assert!(
            p.position.x > server.x,
            "реплей не применился: {:?}",
            p.position
        );
        assert!(p.position.x < server.x + 0.2);
        assert_eq!(p.corrections(), 1);
    }

    #[test]
    fn small_error_keeps_local_state() {
        let mut p = Prediction::new(Vec3::new(0.0, 0.8, 0.0));
        p.predict(cmd(1, -1.0), bounds(), &[]);
        let local = p.position;
        let corrected = p.reconcile(
            local + Vec3::new(0.1, 0.0, 0.0),
            p.velocity,
            1,
            bounds(),
            &[],
        );
        assert!(!corrected);
        assert_eq!(p.position, local, "малая ошибка не дёргает игрока");
    }

    #[test]
    fn stale_snapshot_does_not_roll_back_ack() {
        let mut p = Prediction::new(Vec3::new(0.0, 0.8, 0.0));
        for seq in 1..=4 {
            p.predict(cmd(seq, -1.0), bounds(), &[]);
        }
        p.reconcile(p.position, p.velocity, 4, bounds(), &[]);
        p.reconcile(Vec3::new(100.0, 0.8, 0.0), Vec3::ZERO, 2, bounds(), &[]);
        assert_eq!(p.last_acked(), 4);
        assert!(
            p.position.x < 10.0,
            "старый снапшот не должен телепортировать"
        );
    }
}
