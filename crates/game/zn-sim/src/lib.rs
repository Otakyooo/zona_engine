//! zn-sim: общий код симуляции (клиент и сервер).

use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

/// Пространства идентификаторов, общие для клиента и сервера.
///
/// Раньше id предметов, лута и артефактов жили в одном числовом ряду: артефакт
/// с id 1 в инвентаре съедался как аптечка. Разносим по диапазонам.
pub mod ids {
    /// Предметы инвентаря (1 — аптечка, 2 — бинт).
    pub const ITEM_MAX: u64 = 999;
    /// Выпавший лут: `LOOT_BASE + счётчик`.
    pub const LOOT_BASE: u64 = 1_000_000;
    /// Артефакты в инвентаре: `ARTIFACT_BASE + id артефакта`.
    pub const ARTIFACT_BASE: u64 = 2_000_000;
    /// Существа: `CREATURE_BASE + id`.
    pub const CREATURE_BASE: u64 = 3_000_000;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum IdKind {
        Item,
        Loot,
        Artifact,
        Creature,
    }

    pub fn classify(id: u64) -> Option<IdKind> {
        if (1..=ITEM_MAX).contains(&id) {
            Some(IdKind::Item)
        } else if (LOOT_BASE..LOOT_BASE + 1_000_000).contains(&id) {
            Some(IdKind::Loot)
        } else if (ARTIFACT_BASE..ARTIFACT_BASE + 1_000_000).contains(&id) {
            Some(IdKind::Artifact)
        } else if (CREATURE_BASE..CREATURE_BASE + 1_000_000).contains(&id) {
            Some(IdKind::Creature)
        } else {
            None
        }
    }
}

pub mod items;

/// Горизонтальная скорость игрока, м/с.
const MOVE_SPEED: f32 = 5.0;

/// Ускорение свободного падения, м/с². Одно на клиент и сервер: клиент
/// предсказывает движение той же функцией, что и сервер.
pub const GRAVITY: f32 = 9.81;
/// Высота ног над полом на земле, м. Арена собрана так, что верх пола — 0.5 м;
/// 0.8 оставляет запас и зафиксировано тестами.
pub const GROUND_Y: f32 = 0.8;

/// Высота глаз стрелка над позицией ног.
pub const EYE_HEIGHT: f32 = 1.6;
/// Допустимое расхождение присланной клиентом точки выстрела с глазами стрелка.
pub const SHOT_ORIGIN_TOLERANCE: f32 = 2.0;
/// Предельная дальность выстрела, м.
pub const SHOT_RANGE: f32 = 120.0;
/// XP за убитое существо.
pub const CREATURE_KILL_XP: i64 = 50;
/// Силуэт игрока: высота 1.8 м, ширина 0.6 м, голова — верхние 0.25 м.
pub const PLAYER_HEIGHT: f32 = 1.8;
pub const PLAYER_HALF_WIDTH: f32 = 0.3;
const PLAYER_HEAD_HEIGHT: f32 = 0.25;
/// Силуэт существа по умолчанию: высота 1.2 м, ширина 0.8 м.
pub const CREATURE_HEIGHT: f32 = 1.2;
pub const CREATURE_HALF_WIDTH: f32 = 0.4;
const CREATURE_HEAD_HEIGHT: f32 = 0.25;
/// Голова уже корпуса: попасть в неё труднее, чем в грудь.
const HEAD_WIDTH_RATIO: f32 = 0.66;
/// Предел накопленных событий: сервер забирает их каждый тик, но если
/// вызывающий этого не делает, память не должна расти бесконечно.
const MAX_EVENTS: usize = 256;

/// Допустимое расхождение углов, при котором коррекция не применяется (~20°).
const ANGLE_THRESHOLD: f32 = 0.35;
/// Доля, на которую серверное значение подмешивается к локальному.
const ANGLE_BLEND: f32 = 0.15;

/// Мягкая коррекция углов обзора.
///
/// Возвращает `None`, если расхождение в пределах нормы: снапшот приходит
/// 20 раз в секунду, и жёсткая перезапись yaw/pitch делала мышь неотзывчивой.
pub fn correct_angles(local: (f32, f32), server: (f32, f32)) -> Option<(f32, f32)> {
    let dyaw = server.0 - local.0;
    let dpitch = server.1 - local.1;
    if dyaw.abs() <= ANGLE_THRESHOLD && dpitch.abs() <= ANGLE_THRESHOLD {
        return None;
    }
    Some((local.0 + dyaw * ANGLE_BLEND, local.1 + dpitch * ANGLE_BLEND))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InputCommand {
    pub seq: u32,
    pub yaw: f32,
    pub pitch: f32,
    pub move_x: f32,
    pub move_z: f32,
    pub dt: f32,
}

/// Ввод приходит из сети без проверок: нечисловая команда отравляет позицию
/// (`f32::clamp` на NaN возвращает NaN), поэтому она отбрасывается целиком.
pub fn input_is_finite(cmd: &InputCommand) -> bool {
    cmd.yaw.is_finite()
        && cmd.pitch.is_finite()
        && cmd.move_x.is_finite()
        && cmd.move_z.is_finite()
        && cmd.dt.is_finite()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum CreatureState {
    Patrol,
    Chase,
    Attack,
}

/// Границы проходимой части карты по горизонтали.
///
/// Раньше предел был зашит в `tick` числом `19.0`, и любая карта другого
/// размера ломалась молча: игрок упирался в невидимую стену там, где её нет.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            min: Vec3::new(-24.0, 0.0, -24.0),
            max: Vec3::new(24.0, 50.0, 24.0),
        }
    }
}

impl Bounds {
    /// Держит игрока внутри карты. Высота не трогается: за неё отвечает гравитация.
    pub fn clamp_xz(&self, position: Vec3) -> Vec3 {
        let min = Vec3::new(
            self.min.x.min(self.max.x),
            position.y,
            self.min.z.min(self.max.z),
        );
        let max = Vec3::new(
            self.min.x.max(self.max.x),
            position.y,
            self.min.z.max(self.max.z),
        );
        position.clamp(min, max)
    }

    pub fn contains_xz(&self, position: Vec3) -> bool {
        let lo_x = self.min.x.min(self.max.x);
        let hi_x = self.min.x.max(self.max.x);
        let lo_z = self.min.z.min(self.max.z);
        let hi_z = self.min.z.max(self.max.z);
        position.x >= lo_x && position.x <= hi_x && position.z >= lo_z && position.z <= hi_z
    }
}

/// Шаблон спавна существа из карты: по нему существо и возвращается в мир после смерти.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreatureSpawn {
    pub id: u64,
    pub position: Vec3,
    #[serde(default)]
    pub kind: String,
    pub health: f32,
    pub damage: f32,
    /// Задержка до возвращения в мир после смерти; `0.0` — не возрождать.
    #[serde(default)]
    pub respawn_secs: f32,
}

/// Выровненный по осям бокс. Стены карты и хитбоксы — всё это `Aabb`.
///
/// `Serialize` нужен, чтобы сервер один раз отправил статические боксы карты
/// клиенту: предсказание движения обязано видеть те же стены, что и сервер.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Aabb {
    pub center: Vec3,
    pub half_extents: Vec3,
}

impl Aabb {
    pub fn new(center: Vec3, half_extents: Vec3) -> Self {
        Self {
            center,
            half_extents: half_extents.abs(),
        }
    }

    /// Бокс вокруг позиции ног: центр поднимается на половину высоты,
    /// иначе хитбокс уйдёт наполовину в землю.
    pub fn around(position: Vec3, half_extents: Vec3) -> Self {
        let half = half_extents.abs();
        Self {
            center: position + Vec3::Y * half.y,
            half_extents: half,
        }
    }
}

/// Пересечение луча с боксом (слэб-тест). `dir` должен быть нормализован.
///
/// Возвращает расстояние до входа в бокс; `None` — промах. Луч, начавшийся
/// внутри бокса, даёт `0.0`: стрелять ближе нуля метров нельзя.
pub fn ray_aabb(origin: Vec3, dir: Vec3, max_dist: f32, b: &Aabb) -> Option<f32> {
    let mut t_min = 0.0f32;
    let mut t_max = max_dist;
    for axis in 0..3 {
        let o = origin[axis];
        let d = dir[axis];
        let lo = b.center[axis] - b.half_extents[axis];
        let hi = b.center[axis] + b.half_extents[axis];
        if d.abs() < 1e-6 {
            // Луч параллелен этим граням: либо внутри полосы, либо мимо.
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d;
        let mut t1 = (lo - o) * inv;
        let mut t2 = (hi - o) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        t_min = t_min.max(t1);
        t_max = t_max.min(t2);
        if t_min > t_max {
            return None;
        }
    }
    Some(t_min)
}

/// Желаемая горизонтальная скорость по команде ввода.
///
/// Вынесено из `apply_input`, чтобы клиентское предсказание считало скорость
/// той же формулой, а не своей копией: расхождение «диагональ быстрее» или
/// другой знак `move_z` иначе всплывает только под нагрузкой.
pub fn desired_velocity(yaw: f32, move_x: f32, move_z: f32) -> Vec3 {
    let forward = Vec3::new(yaw.cos(), 0.0, yaw.sin());
    let right = Vec3::new(forward.z, 0.0, -forward.x);
    let mut vel = forward * (-move_z) + right * move_x;
    // Диагональ не должна быть быстрее прямого хода.
    let speed = vel.length();
    if speed > 1.0 {
        vel /= speed;
    }
    vel * MOVE_SPEED
}

/// Пересекается ли тело с боксом по вертикали и горизонтали.
fn body_overlaps(a: &Aabb, pos: Vec3, radius: f32, height: f32) -> bool {
    let body_min_y = pos.y;
    let body_max_y = pos.y + height;
    let box_min_y = a.center.y - a.half_extents.y;
    let box_max_y = a.center.y + a.half_extents.y;
    if box_max_y <= body_min_y || box_min_y >= body_max_y {
        return false;
    }
    let reach = radius;
    (pos.x + reach > a.center.x - a.half_extents.x)
        && (pos.x - reach < a.center.x + a.half_extents.x)
        && (pos.z + reach > a.center.z - a.half_extents.z)
        && (pos.z - reach < a.center.z + a.half_extents.z)
}

/// Двигает тело по горизонтали, разрешая столкновения по одной оси за шаг.
///
/// Разрешение по осям даёт скольжение вдоль стены: игрок, упёршийся в стену
/// по X, продолжает двигаться по Z. Полноценный солвер здесь не нужен —
/// карта собрана из осевых блоков, а сервер обязан быть детерминированным.
pub fn move_horizontal(
    position: &mut Vec3,
    dx: f32,
    dz: f32,
    bounds: Bounds,
    obstacles: &[Aabb],
    radius: f32,
    height: f32,
) {
    if dx != 0.0 {
        let old_x = position.x;
        position.x += dx;
        for a in obstacles {
            if !body_overlaps(a, *position, radius, height) {
                continue;
            }
            let min_x = a.center.x - a.half_extents.x - radius;
            let max_x = a.center.x + a.half_extents.x + radius;
            position.x = if old_x <= min_x {
                min_x
            } else if old_x >= max_x {
                max_x
            } else if (old_x - min_x).abs() < (max_x - old_x).abs() {
                min_x
            } else {
                max_x
            };
        }
    }
    if dz != 0.0 {
        let old_z = position.z;
        position.z += dz;
        for a in obstacles {
            if !body_overlaps(a, *position, radius, height) {
                continue;
            }
            let min_z = a.center.z - a.half_extents.z - radius;
            let max_z = a.center.z + a.half_extents.z + radius;
            position.z = if old_z <= min_z {
                min_z
            } else if old_z >= max_z {
                max_z
            } else if (old_z - min_z).abs() < (max_z - old_z).abs() {
                min_z
            } else {
                max_z
            };
        }
    }
    *position = bounds.clamp_xz(*position);
}

/// Один шаг физики тела: гравитация, земля и горизонтальное движение со стенами.
///
/// Единственная реализация движения для клиента и сервера. Клиент вызывает её
/// для предсказания, сервер — в `tick`; расхождение возможно только через
/// разницу во входных данных, а не через вторую копию арифметики.
pub fn step_body(
    position: &mut Vec3,
    velocity: &mut Vec3,
    dt: f32,
    bounds: Bounds,
    obstacles: &[Aabb],
    radius: f32,
    height: f32,
) {
    velocity.y -= GRAVITY * dt;
    position.y += velocity.y * dt;
    if position.y <= GROUND_Y {
        position.y = GROUND_Y;
        velocity.y = 0.0;
    }
    move_horizontal(
        position,
        velocity.x * dt,
        velocity.z * dt,
        bounds,
        obstacles,
        radius,
        height,
    );
}

/// Максимальный скачок позиции, который имеет смысл сглаживать, м.
///
/// Респавн и телепорт — это честный прыжок: интерполяция через полкарты
/// выглядит хуже, чем мгновенное перемещение.
pub const INTERP_MAX_JUMP: f32 = 5.0;

/// Позиция между двумя снапшотами для отрисовки.
///
/// `alpha` клампится в 0…1: кадров приходит больше, чем снапшотов, и клиент
/// может опоздать или поспеть раньше. Скачок дальше `max_jump` не сглаживается.
pub fn interpolate(prev: Vec3, cur: Vec3, alpha: f32, max_jump: f32) -> Vec3 {
    let a = alpha.clamp(0.0, 1.0);
    if (cur - prev).length() > max_jump {
        return cur;
    }
    prev.lerp(cur, a)
}

/// Кратчайшая интерполяция угла.
///
/// Без перехода через границу углы 170° и −170° соединялись бы дугой через ноль,
/// то есть поворотом на 340° вместо 20°.
pub fn lerp_angle(prev: f32, cur: f32, alpha: f32) -> f32 {
    let a = alpha.clamp(0.0, 1.0);
    let pi = std::f32::consts::PI;
    let diff = (cur - prev + pi).rem_euclid(std::f32::consts::TAU) - pi;
    prev + diff * a
}

/// Зона попадания. Голова — отдельный хитбокс, а не множитель «на глаз».
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HitZone {
    Body,
    Head,
}

impl HitZone {
    /// Множитель базового урона оружия.
    ///
    /// 3× делает винтовку (35) смертельной с одного попадания в голову:
    /// экстракшн-шутер должен наказывать за точный выстрел. Броня, когда
    /// появится, будет гасить это через `equipped_armor`.
    pub fn multiplier(self) -> f32 {
        match self {
            HitZone::Body => 1.0,
            HitZone::Head => 3.0,
        }
    }
}

/// Хитбоксы силуэта: тело снизу, голова сверху.
///
/// Боксы не перекрываются — тело заканчивается ровно там, где начинается
/// голова, поэтому у попадания всегда ровно одна зона. Лежи голова внутри
/// общего AABB, корпус давал бы меньшее `t`, и зона зависела бы от порядка
/// перебора, а не от геометрии.
fn hitboxes(
    position: Vec3,
    half_width: f32,
    height: f32,
    head_height: f32,
) -> [(Aabb, HitZone); 2] {
    let body_height = (height - head_height).max(0.0);
    let head_half = half_width * HEAD_WIDTH_RATIO;
    [
        (
            Aabb::new(
                position + Vec3::Y * (body_height * 0.5),
                Vec3::new(half_width, body_height * 0.5, half_width),
            ),
            HitZone::Body,
        ),
        (
            Aabb::new(
                position + Vec3::Y * (body_height + head_height * 0.5),
                Vec3::new(head_half, head_height * 0.5, head_half),
            ),
            HitZone::Head,
        ),
    ]
}

/// Кандидат на попадание: дистанция, id цели, индексы в `players`/`creatures`
/// (ровно один из них задан) и зона. Вынесено в алиас, иначе кортеж из пяти
/// полей раздувает сигнатуры и ловится `clippy::type_complexity`.
type HitCandidate = (f32, u64, Option<usize>, Option<usize>, HitZone);

#[cfg(test)]
mod interp_tests {
    use super::*;

    #[test]
    fn interpolate_hits_both_ends() {
        // 2 м — заведомо меньше лимита скачка, иначе сглаживание выключится.
        let a = Vec3::ZERO;
        let b = Vec3::new(2.0, 0.0, 0.0);
        assert_eq!(interpolate(a, b, 0.0, INTERP_MAX_JUMP), a);
        assert_eq!(interpolate(a, b, 1.0, INTERP_MAX_JUMP), b);
        let mid = interpolate(a, b, 0.5, INTERP_MAX_JUMP);
        assert!((mid.x - 1.0).abs() < 1e-4, "середина = {mid:?}");
    }

    #[test]
    fn interpolate_clamps_alpha() {
        let a = Vec3::ZERO;
        let b = Vec3::new(4.0, 0.0, 0.0);
        assert_eq!(interpolate(a, b, -3.0, INTERP_MAX_JUMP), a);
        assert_eq!(interpolate(a, b, 7.0, INTERP_MAX_JUMP), b);
    }

    #[test]
    fn interpolate_snaps_on_teleport() {
        // 100 м за один снапшот — респавн или телепорт, сглаживать нечего.
        let a = Vec3::ZERO;
        let b = Vec3::new(100.0, 0.0, 0.0);
        assert_eq!(interpolate(a, b, 0.5, INTERP_MAX_JUMP), b);
    }

    #[test]
    fn lerp_angle_takes_short_path() {
        // 170° → −170°: кратчайший путь через ±180°, то есть 20°, а не 340°.
        let prev = 170f32.to_radians();
        let cur = (-170f32).to_radians();
        let mid = lerp_angle(prev, cur, 0.5);
        assert!(
            mid.abs() > 3.0,
            "середина должна быть у границы ±π, получили {mid}"
        );

        let quarter = lerp_angle(prev, cur, 0.25);
        assert!(quarter > prev, "угол должен идти дальше вверх: {quarter}");
    }

    #[test]
    fn lerp_angle_keeps_equal_angles() {
        let a = 1.234f32;
        assert!((lerp_angle(a, a, 0.5) - a).abs() < 1e-6);
        assert!((lerp_angle(a, a + 1.0, 0.0) - a).abs() < 1e-6);
        assert!((lerp_angle(a, a + 1.0, 1.0) - (a + 1.0)).abs() < 1e-6);
    }
}

/// Результат принятого выстрела.
///
/// `zone` и `damage` нужны клиенту для маркера попадания и звука: урон
/// считает сервер, клиент его только показывает.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShotOutcome {
    pub hit: Option<u64>,
    pub killed: bool,
    pub zone: Option<HitZone>,
    /// Урон с учётом зоны; `0.0` при промахе. Может превышать остаток
    /// здоровья цели — здоровье клампится в нуль, а не уходит в минус.
    pub damage: f32,
    /// Тик истории, по которому проверялись цели.
    pub rewind_tick: u64,
    /// Запрошенный тик вышел за окно истории и был зажат к краю.
    /// Это явное поведение: выстрел всё равно проверяется, но сервер
    /// видит и логирует, что откат неточный.
    pub rewind_clamped: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Player {
    pub id: u64,
    pub name: String,
    pub position: Vec3,
    pub health: f32,
    pub max_health: f32,
    pub radiation: f32,
    pub hunger: f32,
    pub thirst: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub velocity: Vec3,
    pub xp: i64,
    pub level: i32,
    pub currency: i64,
    pub skills: Vec<Skill>,
    /// Id оружия из `items::CATALOG`, а не строка: имя и урон больше не
    /// дублируются в HUD, сервере и симуляции.
    pub equipped_weapon: u64,
    pub equipped_armor: u64,
    /// Патроны, доступные оружию. Тратится в `apply_shot`.
    pub ammo: u32,
    /// Остаток перезарядки, секунды. Сервер уменьшает его в `tick`.
    pub shot_cooldown: f32,
    /// Номер последней применённой команды ввода: клиент по нему понимает,
    /// какие команды сервер уже учёл, и не переигрывает их повторно.
    pub last_input_seq: u32,
    pub inventory: Inventory,
}

impl Player {
    pub fn new(id: u64, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            position: Vec3::ZERO,
            health: 100.0,
            max_health: 100.0,
            radiation: 0.0,
            hunger: 100.0,
            thirst: 100.0,
            yaw: 0.0,
            pitch: 0.0,
            velocity: Vec3::ZERO,
            xp: 0,
            level: 1,
            currency: 0,
            skills: Vec::new(),
            equipped_weapon: items::DEFAULT_WEAPON,
            equipped_armor: 0,
            ammo: items::START_AMMO,
            shot_cooldown: 0.0,
            last_input_seq: 0,
            inventory: Inventory::default(),
        }
    }

    /// Оружие в руках; `None` — предмет не является оружием.
    pub fn weapon(&self) -> Option<items::WeaponDef> {
        items::weapon(self.equipped_weapon)
    }

    /// Может ли игрок выстрелить прямо сейчас.
    pub fn can_fire(&self) -> bool {
        self.shot_cooldown <= 0.0 && self.ammo >= self.weapon().map_or(1, |w| w.ammo_per_shot)
    }

    pub fn add_xp(&mut self, amount: i64) {
        self.xp += amount;
        self.level = 1 + (self.xp / 1000) as i32;
    }

    /// Переносит сохранённый прогресс в живого игрока. Без этого `save_profile`
    /// запишет в БД дефолтные `level = 1, xp = 0` и сотрёт прогресс.
    pub fn apply_profile(&mut self, profile: &PlayerProfile) {
        self.level = profile.level;
        self.xp = profile.xp;
        self.currency = profile.currency;
        self.position = profile.position;
        // Выбранное оружие — часть прогресса: иначе каждый вход сбрасывал бы
        // его на пистолет, а патроны обнулялись бы вместе с ним.
        self.equipped_weapon = if items::is_weapon(profile.weapon_id) {
            profile.weapon_id
        } else {
            items::DEFAULT_WEAPON
        };
        self.ammo = profile.ammo.min(items::MAX_AMMO);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerState {
    pub id: u64,
    pub name: String,
    pub position: Vec3,
    pub health: f32,
    pub radiation: f32,
    pub hunger: f32,
    pub thirst: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub level: i32,
    pub xp: i64,
    /// Скорость нужна клиенту как начало отсчёта для реплея
    /// неподтверждённых команд: без неё предсказание расходится после первого
    /// же попадания в стену.
    pub velocity: Vec3,
    pub weapon_id: u64,
    pub ammo: u32,
    /// Последняя команда, которую сервер применил. Клиент отбрасывает всё,
    /// что старше этого номера, и переигрывает только оставшееся.
    pub last_input_seq: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Anomaly {
    pub id: u64,
    pub position: Vec3,
    pub radius: f32,
    pub damage: f32,
    pub radiation: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    pub id: u64,
    pub position: Vec3,
    pub effect: String,
    pub value: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LootItem {
    pub id: u64,
    pub position: Vec3,
    pub item: Item,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Creature {
    pub id: u64,
    pub position: Vec3,
    pub health: f32,
    pub max_health: f32,
    pub state: CreatureState,
    pub target: Option<u64>,
    pub damage: f32,
    /// Вид существа из шаблона карты (`"snork"`, `"dog"`...).
    #[serde(default)]
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ExtractionZone {
    pub id: u64,
    pub position: Vec3,
    pub radius: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub tick: u64,
    pub players: Vec<PlayerState>,
    pub anomalies: Vec<Anomaly>,
    pub artifacts: Vec<Artifact>,
    pub loot: Vec<LootItem>,
    pub creatures: Vec<Creature>,
    pub extraction_zones: Vec<ExtractionZone>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Skill {
    pub id: String,
    pub level: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
    pub name: String,
    pub value: u32,
    pub consumable: bool,
}

impl Item {
    /// Предмет по каталогу: лут и награда больше не придумывают имя и цену
    /// на месте, иначе один и тот же id в инвентаре и на земле выглядел бы
    /// по-разному.
    pub fn from_catalog(id: u64) -> Self {
        let def = items::get(id);
        Self {
            id,
            name: def
                .map(|d| d.name.to_string())
                .unwrap_or_else(|| format!("item_{id}")),
            value: def.map_or(0, |d| d.value),
            consumable: def.is_some_and(|d| d.consumable),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Loot {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerProfile {
    pub user_id: i64,
    pub username: String,
    pub level: i32,
    pub xp: i64,
    pub currency: i64,
    pub position: Vec3,
    /// Выбранное оружие и остаток патронов переживают выход из рейда.
    pub weapon_id: u64,
    pub ammo: u32,
}

impl PlayerProfile {
    pub fn new(user_id: i64, username: impl Into<String>) -> Self {
        Self {
            user_id,
            username: username.into(),
            level: 1,
            xp: 0,
            currency: 0,
            position: Vec3::ZERO,
            weapon_id: items::DEFAULT_WEAPON,
            ammo: items::START_AMMO,
        }
    }
    pub fn add_xp(&mut self, amount: i64) {
        self.xp += amount;
        self.level = 1 + (self.xp / 1000) as i32;
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InventoryItem {
    pub item_id: i64,
    pub quantity: i32,
    pub slot: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub items: Vec<InventoryItem>,
}

impl Inventory {
    pub fn add_item(&mut self, item_id: i64, quantity: i32) {
        if let Some(it) = self.items.iter_mut().find(|i| i.item_id == item_id) {
            it.quantity += quantity;
        } else {
            let slot = self.items.len() as i32;
            self.items.push(InventoryItem {
                item_id,
                quantity,
                slot,
            });
        }
    }
    pub fn remove_item(&mut self, item_id: i64, quantity: i32) -> bool {
        if let Some(it) = self.items.iter_mut().find(|i| i.item_id == item_id)
            && it.quantity >= quantity
        {
            it.quantity -= quantity;
            if it.quantity <= 0 {
                self.items.retain(|i| i.item_id != item_id);
            }
            return true;
        }
        false
    }
    pub fn has_item(&self, item_id: i64, quantity: i32) -> bool {
        self.items
            .iter()
            .any(|i| i.item_id == item_id && i.quantity >= quantity)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GameEvent {
    PlayerDeath {
        player_id: u64,
        dropped_loot: Vec<Item>,
        dropped_consumables: Vec<Item>,
    },
    ExtractionSuccess {
        player_id: u64,
        loot: Vec<Item>,
    },
    CreatureKilled {
        creature_id: u64,
        killer: u64,
        xp: i64,
    },
}

#[derive(Default)]
pub struct SimWorld {
    pub tick: u64,
    pub players: Vec<Player>,
    pub anomalies: Vec<Anomaly>,
    pub artifacts: Vec<Artifact>,
    pub loot: Vec<LootItem>,
    pub creatures: Vec<Creature>,
    pub extraction_zones: Vec<ExtractionZone>,
    pub extraction_timers: HashMap<u64, f32>,
    pub events: Vec<GameEvent>,
    /// Следующий свободный id для выпавшего лута.
    pub next_loot_id: u64,
    /// Границы карты; задаются зоной, а не кодом.
    pub bounds: Bounds,
    /// Статические стены карты. Раньше они нужны были только рейкасту, и
    /// игрок проходил сквозь них; теперь это ещё и коллизии движения.
    pub obstacles: Vec<Aabb>,
    /// Последняя команда ввода на игрока, ждущая ближайшего тика.
    ///
    /// Применять ввод прямо в обработчике пакета значило бы, что при двух
    /// пакетах за тик сервер двигает игрока дважды, а клиент предсказывает
    /// иначе. Один вход на тик делает реплей детерминированным.
    pub queued_inputs: HashMap<u64, InputCommand>,
    /// Короткая история позиций целей для компенсации лага.
    ///
    /// Ключ — id игрока или существа, значение — кольцо `(tick, position)`.
    /// Размер ограничен `LAG_HISTORY_TICKS` на сущность; мёртвые сущности
    /// вычищаются при записи, поэтому память не растёт с числом смертей.
    pub position_history: HashMap<u64, VecDeque<(u64, Vec3)>>,
    /// Шаблоны существ из карты — нужны, чтобы возрождать убитых.
    pub creature_spawns: Vec<CreatureSpawn>,
    /// Точка появления игрока; берётся из карты.
    pub player_spawn: Vec3,
    /// Ожидающие возрождения: `(id существа, секунд до появления)`.
    respawns: Vec<(u64, f32)>,
}

/// Срез мира для периодического лога сервера.
///
/// Это не состояние, а счётчики: дешёвая структура, которую можно писать раз
/// в десять секунд, не трогая тик.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorldMetrics {
    pub tick: u64,
    pub players: usize,
    pub creatures: usize,
    pub loot: usize,
    /// Накопленные, но ещё не забранные события: растут, если сервер забыл
    /// вызвать `take_events`.
    pub pending_events: usize,
}

impl SimWorld {
    pub fn new() -> Self {
        let mut world = Self::default();
        world.seed_loot_ids();
        world
    }

    /// Счётчик стартует выше всего, что уже лежит в мире, и не ниже `LOOT_BASE`,
    /// чтобы выпавший лут не пересекался ни с предметами, ни с лутом карты.
    pub fn seed_loot_ids(&mut self) {
        let max_existing = self.loot.iter().map(|l| l.id).max().unwrap_or(0);
        self.next_loot_id = max_existing.max(ids::LOOT_BASE) + 1;
    }
    pub fn add_player(&mut self, p: Player) {
        self.players.push(p);
    }
    pub fn remove_player(&mut self, id: u64) {
        self.players.retain(|p| p.id != id);
        // Таймер эвакуации и очередь ввода не должны пережить игрока: иначе
        // после возвращения в рейд сработает чужой прогресс или старый вход.
        self.extraction_timers.remove(&id);
        self.queued_inputs.remove(&id);
        self.position_history.remove(&id);
    }
    pub fn get_player(&self, id: u64) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }
    pub fn get_player_mut(&mut self, id: u64) -> Option<&mut Player> {
        self.players.iter_mut().find(|p| p.id == id)
    }
    /// Забирает накопленные события.
    pub fn take_events(&mut self) -> Vec<GameEvent> {
        std::mem::take(&mut self.events)
    }

    /// Счётчики для лога: живой ли тик, сколько сущностей, копится ли очередь
    /// событий. Дороже клонирования снапшота не стоит.
    pub fn metrics(&self) -> WorldMetrics {
        WorldMetrics {
            tick: self.tick,
            players: self.players.len(),
            creatures: self.creatures.len(),
            loot: self.loot.len(),
            pending_events: self.events.len(),
        }
    }

    /// Выкладывает лут и расходники игрока на землю в точке смерти.
    ///
    /// Снаряжение (надетое оружие и броня) остаётся у игрока: в открытом мире
    /// смерть стоит добычи, но не возможности вернуться в рейд. Каждая единица
    /// становится отдельным лутом — раньше стек из двух аптечек превращался
    /// в одну, и вторая исчезала.
    ///
    /// Вызывать строго до респауна: позиция берётся у ещё не перемещённого
    /// игрока, иначе весь лут окажется в начале координат. Id выдаются из
    /// `next_loot_id`, поэтому кучи разных смертей не конфликтуют.
    pub fn drop_inventory_at(&mut self, player_id: u64, pos: Vec3) -> Vec<Item> {
        let (equipped, inventory) = match self.get_player_mut(player_id) {
            Some(p) => (p.equipped_weapon, std::mem::take(&mut p.inventory)),
            None => return Vec::new(),
        };
        let mut kept = Inventory::default();
        let mut dropped = Vec::new();
        for it in inventory.items {
            let id = it.item_id.max(0) as u64;
            // Надетое оружие и броня — снаряжение, а не добыча.
            let equipped_here = id == equipped && items::is_weapon(id);
            let keep = if equipped_here || id == items::LIGHT_ARMOR {
                1
            } else {
                0
            };
            if keep > 0 {
                kept.add_item(it.item_id, keep);
            }
            for _ in 0..(it.quantity - keep).max(0) {
                let item = Item::from_catalog(id);
                self.loot.push(LootItem {
                    id: self.next_loot_id,
                    position: pos,
                    item: item.clone(),
                });
                self.next_loot_id += 1;
                dropped.push(item);
            }
        }
        if let Some(p) = self.get_player_mut(player_id) {
            p.inventory = kept;
        }
        dropped
    }

    /// Помечает игроков с нулевым здоровьем.
    ///
    /// Вызывать после всех источников урона: урон существ применяется позже
    /// движения, и раньше такие смерти не попадали в обработку вообще —
    /// игрок оставался с нулевым HP без респауна.
    pub fn collect_deaths(&mut self, dead: &mut Vec<u64>) {
        for p in self.players.iter_mut() {
            if p.health <= 0.0 {
                p.health = 0.0;
                if !dead.contains(&p.id) {
                    dead.push(p.id);
                }
            }
        }
    }
    /// Применяет команду ввода, пришедшую от клиента.
    ///
    /// Команда приходит по сети без проверок, поэтому сервер валидирует её сам:
    /// NaN в `yaw` отравлял позицию (`f32::clamp` на NaN возвращает NaN), а
    /// неограниченный вектор движения разгонял игрока до сотен метров в секунду.
    /// Некорректная команда отбрасывается целиком, а не применяется частично.
    pub fn apply_input(&mut self, player_id: u64, cmd: &InputCommand) {
        if !input_is_finite(cmd) {
            log::debug!("rejected non-finite input from player {player_id}: {cmd:?}");
            return;
        }
        let Some(p) = self.get_player_mut(player_id) else {
            return;
        };
        // `seq` монотонен на клиенте: команда, пришедшая с опозданием, не
        // должна откатывать уже применённое состояние и ломать ack.
        if cmd.seq < p.last_input_seq {
            return;
        }
        p.yaw = cmd.yaw;
        p.pitch = cmd
            .pitch
            .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2);
        let vel = desired_velocity(cmd.yaw, cmd.move_x, cmd.move_z);
        p.velocity.x = vel.x;
        p.velocity.z = vel.z;
        p.last_input_seq = cmd.seq;
    }

    /// Кладёт команду в очередь ближайшего тика.
    ///
    /// Очередь хранит только последнюю команду: более старые уже не нужны,
    /// если сервер не успел их применить, — важно лишь итоговое направление.
    /// Команда не применяется здесь, иначе клиент и сервер расходились бы
    /// на числе пакетов, пришедших между тиками.
    pub fn queue_input(&mut self, player_id: u64, cmd: &InputCommand) {
        if !input_is_finite(cmd) {
            log::debug!("rejected non-finite input from player {player_id}: {cmd:?}");
            return;
        }
        if self.get_player(player_id).is_none() {
            return;
        }
        self.queued_inputs.insert(player_id, cmd.clone());
    }

    fn apply_queued_inputs(&mut self) {
        if self.queued_inputs.is_empty() {
            return;
        }
        let queued = std::mem::take(&mut self.queued_inputs);
        for (pid, cmd) in queued {
            self.apply_input(pid, &cmd);
        }
    }
    /// Разрешает выстрел. `None` — выстрел отклонён (чужая точка, мусорный вектор).
    ///
    /// Вся арифметика урона живёт здесь, а не в сервере: правило «simulation owns
    /// damage, death, loot, XP» иначе держалось бы на честном слове. Смерть
    /// игрока на месте не обрабатывается: здоровье падает, а лут и `PlayerDeath`
    /// выдаёт `tick`, иначе награда задвоилась бы.
    /// Сколько тиков истории хранится на сущность. 60 при 60 Гц — одна
    /// секунда: с запасом больше любого разумного RTT/2.
    pub const LAG_HISTORY_TICKS: usize = 60;

    /// Записывает текущие позиции целей в кольцо истории.
    ///
    /// Вызывается в конце тика, после движения и респавнов. Снимки берутся
    /// одним проходом: иначе заимствование `self` не даст писать в историю.
    pub fn record_history(&mut self) {
        let tick = self.tick;
        let samples: Vec<(u64, Vec3)> = self
            .players
            .iter()
            .map(|p| (p.id, p.position))
            .chain(self.creatures.iter().map(|c| (c.id, c.position)))
            .collect();
        for (id, position) in samples {
            let ring = self.position_history.entry(id).or_default();
            if ring.back().is_none_or(|(t, _)| *t != tick) {
                ring.push_back((tick, position));
            }
            while ring.len() > Self::LAG_HISTORY_TICKS {
                ring.pop_front();
            }
        }
        // Мёртвые и вышедшие сущности не должны оставлять за собой кольца.
        self.position_history.retain(|id, _| {
            self.players.iter().any(|p| p.id == *id) || self.creatures.iter().any(|c| c.id == *id)
        });
    }

    /// Зажимает запрошенный тик к доступному окну истории.
    ///
    /// Возвращает `(tick, clamped)`. Пустая история — это `(self.tick, true)`:
    /// проверка идёт по текущим позициям, и вызывающий видит, что отката не
    /// было. Так выстрел не пропадает молча и не выдаёт себя за точный.
    pub fn clamp_history_tick(&self, requested: u64) -> (u64, bool) {
        let oldest = self
            .position_history
            .values()
            .filter_map(|ring| ring.front().map(|(tick, _)| *tick))
            .min();
        let Some(oldest) = oldest else {
            return (self.tick, requested != self.tick);
        };
        let clamped = requested < oldest || requested > self.tick;
        (requested.clamp(oldest, self.tick), clamped)
    }

    /// Позиция сущности на указанном тике: последний снимок не позже него.
    fn position_at(&self, id: u64, tick: u64, current: Vec3) -> Vec3 {
        // Нулевой откат всегда означает текущий кадр: иначе последний
        // записанный тик (self.tick - 1) считался бы историей и «без отката»
        // на самом деле стрелял бы в прошлое.
        if tick >= self.tick {
            return current;
        }
        let Some(ring) = self.position_history.get(&id) else {
            return current;
        };
        ring.iter()
            .rev()
            .find(|(sample_tick, _)| *sample_tick <= tick)
            .map(|(_, position)| *position)
            .or_else(|| ring.front().map(|(_, position)| *position))
            .unwrap_or(current)
    }

    pub fn apply_shot(&mut self, shooter: u64, origin: Vec3, dir: Vec3) -> Option<ShotOutcome> {
        self.apply_shot_rewound(shooter, origin, dir, 0)
    }

    /// Разрешает выстрел с откатом целей на `rewind_ticks` назад.
    ///
    /// Откат применяется только к позициям целей; здоровье, стены и расход
    /// патронов остаются текущими. Если истории нет или она короче запроса,
    /// тик зажимается, а `ShotOutcome.rewind_clamped` говорит об этом явно.
    pub fn apply_shot_rewound(
        &mut self,
        shooter: u64,
        origin: Vec3,
        dir: Vec3,
        rewind_ticks: u32,
    ) -> Option<ShotOutcome> {
        if !origin.is_finite() || !dir.is_finite() {
            return None;
        }
        let (eye, weapon) = {
            let p = self.get_player(shooter)?;
            // Темп стрельбы — свойство оружия. Без этой проверки автоклик
            // выдавал любое число выстрелов в секунду.
            let weapon = p.weapon()?;
            if !p.can_fire() {
                return None;
            }
            (p.position + Vec3::Y * EYE_HEIGHT, weapon)
        };
        if (origin - eye).length() > SHOT_ORIGIN_TOLERANCE || dir.length() < 0.1 {
            return None;
        }
        let d = dir.normalize();
        let (target_tick, rewind_clamped) =
            self.clamp_history_tick(self.tick.saturating_sub(rewind_ticks as u64));

        // Выстрел принят: патрон и задержка списываются даже при промахе.
        if let Some(p) = self.get_player_mut(shooter) {
            p.ammo = p.ammo.saturating_sub(weapon.ammo_per_shot);
            p.shot_cooldown = weapon.cooldown_secs;
        }

        // Стена ближе цели делает выстрел промахом.
        let mut wall = SHOT_RANGE;
        for b in &self.obstacles {
            if let Some(t) = ray_aabb(origin, d, wall, b) {
                wall = t;
            }
        }

        let mut best: Option<HitCandidate> = None;
        for (i, p) in self.players.iter().enumerate() {
            if p.id == shooter {
                continue;
            }
            let target_position = self.position_at(p.id, target_tick, p.position);
            for (b, zone) in hitboxes(
                target_position,
                PLAYER_HALF_WIDTH,
                PLAYER_HEIGHT,
                PLAYER_HEAD_HEIGHT,
            ) {
                if let Some(t) = ray_aabb(origin, d, wall, &b)
                    && best.is_none_or(|(best_t, ..)| t < best_t)
                {
                    best = Some((t, p.id, Some(i), None, zone));
                }
            }
        }
        for (i, c) in self.creatures.iter().enumerate() {
            let target_position = self.position_at(c.id, target_tick, c.position);
            for (b, zone) in hitboxes(
                target_position,
                CREATURE_HALF_WIDTH,
                CREATURE_HEIGHT,
                CREATURE_HEAD_HEIGHT,
            ) {
                if let Some(t) = ray_aabb(origin, d, wall, &b)
                    && best.is_none_or(|(best_t, ..)| t < best_t)
                {
                    best = Some((t, c.id, None, Some(i), zone));
                }
            }
        }

        let Some((_, target, player_idx, creature_idx, zone)) = best else {
            return Some(ShotOutcome {
                hit: None,
                killed: false,
                zone: None,
                damage: 0.0,
                rewind_tick: target_tick,
                rewind_clamped,
            });
        };

        let damage = weapon.damage * zone.multiplier();

        // Индексы взяты из того же состояния, по которому шёл поиск.
        let killed = if let Some(i) = creature_idx {
            let c = &mut self.creatures[i];
            // Здоровье не уходит в минус: иначе снапшот между выстрелом и
            // тиком показывал бы отрицательный HP.
            c.health = (c.health - damage).max(0.0);
            let killed = c.health <= 0.0;
            if killed {
                // Само существо уберёт `tick`, здесь только награда и событие.
                self.events.push(GameEvent::CreatureKilled {
                    creature_id: target,
                    killer: shooter,
                    xp: CREATURE_KILL_XP,
                });
                if let Some(p) = self.get_player_mut(shooter) {
                    p.add_xp(CREATURE_KILL_XP);
                }
            }
            killed
        } else if let Some(i) = player_idx {
            let p = &mut self.players[i];
            p.health = (p.health - damage).max(0.0);
            p.health <= 0.0
        } else {
            false
        };

        Some(ShotOutcome {
            hit: Some(target),
            killed,
            zone: Some(zone),
            damage,
            rewind_tick: target_tick,
            rewind_clamped,
        })
    }
    pub fn tick(&mut self, dt: f32) {
        // События НЕ чистятся здесь: выстрел может прийти между тиками, и его
        // `CreatureKilled` было бы стёрто до того, как сервер его заберёт.
        // Забирает их `SimWorld::take_events`; переполнение ограничено ниже.
        // Один вход на тик: клиент предсказывает ровно один шаг на команду.
        self.apply_queued_inputs();
        let bounds = self.bounds;
        // players
        let mut dead_players: Vec<u64> = Vec::new();
        for p in self.players.iter_mut() {
            p.shot_cooldown = (p.shot_cooldown - dt).max(0.0);
            step_body(
                &mut p.position,
                &mut p.velocity,
                dt,
                bounds,
                &self.obstacles,
                PLAYER_HALF_WIDTH,
                PLAYER_HEIGHT,
            );

            // survival decay
            p.hunger = (p.hunger - 0.5 * dt).max(0.0);
            p.thirst = (p.thirst - 0.8 * dt).max(0.0);
            if p.hunger <= 0.0 || p.thirst <= 0.0 {
                p.health -= 2.0 * dt;
            }

            // anomalies
            for a in &self.anomalies {
                let dist = (p.position - a.position).length();
                if dist <= a.radius {
                    p.health -= a.damage * dt;
                    p.radiation += a.radiation * dt;
                }
            }

            // radiation damage
            if p.radiation > 50.0 {
                p.health -= (p.radiation - 50.0) * 0.01 * dt;
            }
        }

        // extraction timers
        let mut completed: Vec<u64> = Vec::new();
        for p in &self.players {
            for z in &self.extraction_zones {
                if (p.position - z.position).length() <= z.radius {
                    let timer = self.extraction_timers.entry(p.id).or_insert(10.0);
                    *timer -= dt;
                    if *timer <= 0.0 {
                        completed.push(p.id);
                    }
                } else {
                    self.extraction_timers.remove(&p.id);
                }
            }
        }
        for pid in &completed {
            // collect loot? For now empty; client/server handle
            self.events.push(GameEvent::ExtractionSuccess {
                player_id: *pid,
                loot: Vec::new(),
            });
            if let Some(p) = self.get_player_mut(*pid) {
                p.add_xp(100);
            }
            self.extraction_timers.remove(pid);
        }

        // creature AI
        for c in self.creatures.iter_mut() {
            // find nearest player
            let mut nearest: Option<(u64, f32)> = None;
            for p in &self.players {
                let d = (p.position - c.position).length();
                if nearest.is_none() || d < nearest.unwrap().1 {
                    nearest = Some((p.id, d));
                }
            }
            match c.state {
                CreatureState::Patrol => {
                    if let Some((_, d)) = nearest
                        && d < 20.0
                    {
                        c.state = CreatureState::Chase;
                    }
                }
                CreatureState::Chase => {
                    if let Some((pid, d)) = nearest {
                        if d < 2.0 {
                            c.state = CreatureState::Attack;
                            c.target = Some(pid);
                        } else if d > 30.0 {
                            c.state = CreatureState::Patrol;
                        } else {
                            // move toward player; стены общие с игроком
                            if let Some(p) = self.players.iter().find(|pl| pl.id == pid) {
                                let dir = (p.position - c.position).normalize_or_zero();
                                let delta = dir * 3.0 * dt;
                                move_horizontal(
                                    &mut c.position,
                                    delta.x,
                                    delta.z,
                                    bounds,
                                    &self.obstacles,
                                    CREATURE_HALF_WIDTH,
                                    CREATURE_HEIGHT,
                                );
                            }
                        }
                    }
                }
                CreatureState::Attack => {
                    if let Some(pid) = c.target {
                        if let Some(p) = self.players.iter().find(|pl| pl.id == pid) {
                            let d = (p.position - c.position).length();
                            if d > 2.5 {
                                c.state = CreatureState::Chase;
                            } else {
                                // damage player (handled below)
                            }
                        } else {
                            c.state = CreatureState::Patrol;
                            c.target = None;
                        }
                    }
                }
            }
        }
        // creature attacks
        let attacks: Vec<(u64, f32)> = self
            .creatures
            .iter()
            .filter(|c| c.state == CreatureState::Attack)
            .filter_map(|c| c.target.map(|pid| (pid, c.damage)))
            .collect();
        for (pid, dmg) in attacks {
            if let Some(p) = self.get_player_mut(pid) {
                p.health -= dmg * dt;
                if p.health <= 0.0 {
                    p.health = 0.0;
                }
            }
        }

        // Смерти собираются после всех источников урона: урон существ
        // применяется позже движения, и раньше такие смерти терялись.
        self.collect_deaths(&mut dead_players);

        // handle deaths
        let respawn_point = self.player_spawn;
        for pid in &dead_players {
            // Позиция читается до респауна — иначе лут падает в начало координат.
            let death_pos = self
                .get_player(*pid)
                .map(|p| p.position)
                .unwrap_or_default();
            let dropped_loot = self.drop_inventory_at(*pid, death_pos);
            self.events.push(GameEvent::PlayerDeath {
                player_id: *pid,
                dropped_loot,
                dropped_consumables: Vec::new(),
            });
            if let Some(p) = self.get_player_mut(*pid) {
                p.health = p.max_health;
                // Точка респавна — из карты, а не начало координат.
                p.position = respawn_point;
                p.radiation = 0.0;
                p.hunger = 100.0;
                p.thirst = 100.0;
            }
        }

        // Убитые существа уходят из мира и возвращаются по шаблону карты.
        let dead_creatures: Vec<u64> = self
            .creatures
            .iter()
            .filter(|c| c.health <= 0.0)
            .map(|c| c.id)
            .collect();
        self.creatures.retain(|c| c.health > 0.0);
        for id in dead_creatures {
            let Some(template) = self.creature_spawns.iter().find(|t| t.id == id) else {
                continue;
            };
            let secs = template.respawn_secs;
            if secs <= 0.0 || self.respawns.iter().any(|(queued, _)| *queued == id) {
                continue;
            }
            self.respawns.push((id, secs));
        }

        // Таймеры идут всегда: существа должны возвращаться и в пустой мир.
        for entry in self.respawns.iter_mut() {
            entry.1 -= dt;
        }
        let ready: Vec<u64> = self
            .respawns
            .iter()
            .filter(|(_, left)| *left <= 0.0)
            .map(|(id, _)| *id)
            .collect();
        self.respawns.retain(|(_, left)| *left > 0.0);
        for id in ready {
            if self.creatures.iter().any(|c| c.id == id) {
                continue;
            }
            if let Some(t) = self.creature_spawns.iter().find(|t| t.id == id) {
                self.creatures.push(Creature {
                    id: t.id,
                    position: t.position,
                    health: t.health,
                    max_health: t.health,
                    state: CreatureState::Patrol,
                    target: None,
                    damage: t.damage,
                    kind: t.kind.clone(),
                });
            }
        }

        if self.events.len() > MAX_EVENTS {
            let excess = self.events.len() - MAX_EVENTS;
            self.events.drain(0..excess);
        }

        self.record_history();
        self.tick += 1;
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            tick: self.tick,
            players: self
                .players
                .iter()
                .map(|p| PlayerState {
                    id: p.id,
                    name: p.name.clone(),
                    position: p.position,
                    health: p.health,
                    radiation: p.radiation,
                    hunger: p.hunger,
                    thirst: p.thirst,
                    yaw: p.yaw,
                    pitch: p.pitch,
                    level: p.level,
                    xp: p.xp,
                    velocity: p.velocity,
                    weapon_id: p.equipped_weapon,
                    ammo: p.ammo,
                    last_input_seq: p.last_input_seq,
                })
                .collect(),
            anomalies: self.anomalies.clone(),
            artifacts: self.artifacts.clone(),
            loot: self.loot.clone(),
            creatures: self.creatures.clone(),
            extraction_zones: self.extraction_zones.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_applies_gravity() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        w.tick(1.0 / 60.0);
        // gravity should place player on ground at 0.8
        let y = w.get_player(1).unwrap().position.y;
        assert!((y - 0.8).abs() < 0.01);
    }

    #[test]
    fn metrics_count_world_contents() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        w.creatures.push(Creature {
            id: ids::CREATURE_BASE + 1,
            position: Vec3::ZERO,
            health: 10.0,
            max_health: 10.0,
            state: CreatureState::Patrol,
            target: None,
            damage: 1.0,
            kind: "snork".into(),
        });
        w.loot.push(LootItem {
            id: ids::LOOT_BASE,
            position: Vec3::ZERO,
            item: Item {
                id: 1,
                name: "Medkit".into(),
                value: 1,
                consumable: true,
            },
        });
        w.events.push(GameEvent::ExtractionSuccess {
            player_id: 1,
            loot: Vec::new(),
        });

        let m = w.metrics();
        assert_eq!(m.players, 1);
        assert_eq!(m.creatures, 1);
        assert_eq!(m.loot, 1);
        assert_eq!(m.pending_events, 1);
        assert_eq!(m.tick, w.tick);

        // Забранные события перестают считаться ожидающими.
        let _ = w.take_events();
        assert_eq!(w.metrics().pending_events, 0);
    }

    #[test]
    fn anomaly_damages_player() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        // move player into anomaly at (5,0,5) radius 3
        w.get_player_mut(1).unwrap().position = Vec3::new(5.0, 0.8, 5.0);
        w.anomalies.push(Anomaly {
            id: 1,
            position: Vec3::new(5.0, 0.0, 5.0),
            radius: 3.0,
            damage: 5.0,
            radiation: 10.0,
        });
        let hp0 = w.get_player(1).unwrap().health;
        w.tick(1.0);
        let hp1 = w.get_player(1).unwrap().health;
        assert!(hp1 < hp0);
    }

    #[test]
    fn inventory_add_remove() {
        let mut inv = Inventory::default();
        inv.add_item(1, 2);
        assert!(inv.has_item(1, 2));
        assert!(inv.remove_item(1, 1));
        assert!(inv.has_item(1, 1));
        assert!(!inv.remove_item(1, 5));
    }

    #[test]
    fn extraction_awards_xp_once() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.position = Vec3::new(0.0, 0.8, 0.0);
        w.add_player(p);
        w.extraction_zones.push(ExtractionZone {
            id: 1,
            position: Vec3::new(0.0, 0.0, 0.0),
            radius: 3.0,
        });

        // ~11.7 с: таймер зоны (10 с) успевает сработать ровно один раз
        for _ in 0..700 {
            w.tick(1.0 / 60.0);
        }

        assert_eq!(
            w.get_player(1).unwrap().xp,
            100,
            "XP за экстракцию должен начисляться один раз"
        );
    }

    #[test]
    fn angle_correction_ignores_small_delta() {
        assert!(correct_angles((1.0, 0.0), (1.1, 0.1)).is_none());
        let (yaw, _) = correct_angles((1.0, 0.0), (2.0, 0.0)).unwrap();
        assert!(yaw > 1.0 && yaw < 2.0, "коррекция должна быть частичной");
    }

    #[test]
    fn artifact_id_does_not_collide_with_medkit() {
        use ids::{ARTIFACT_BASE, ITEM_MAX, IdKind, classify};
        assert_eq!(classify(1), Some(IdKind::Item));
        assert_eq!(classify(2), Some(IdKind::Item));
        assert_eq!(classify(ARTIFACT_BASE + 1), Some(IdKind::Artifact));
        assert_eq!(classify(ids::LOOT_BASE), Some(IdKind::Loot));
        // за пределами всех диапазонов — не опознаётся
        assert_eq!(classify(ITEM_MAX + 1), None);
        assert_eq!(classify(0), None);
    }

    #[test]
    fn death_loot_at_death_position() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.inventory.add_item(1, 2);
        w.add_player(p);

        let pos = Vec3::new(7.0, 0.5, -3.0);
        let dropped = w.drop_inventory_at(1, pos);
        assert_eq!(dropped.len(), 2, "каждая единица стека выпадает отдельно");
        assert_eq!(w.loot.len(), 2);
        assert!(w.loot.iter().all(|l| l.position == pos));
        assert!(w.get_player(1).unwrap().inventory.items.is_empty());
    }

    #[test]
    fn equipment_survives_death() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.equipped_weapon = items::RIFLE;
        p.inventory.add_item(items::RIFLE as i64, 1);
        p.inventory.add_item(items::LIGHT_ARMOR as i64, 1);
        p.inventory.add_item(items::MEDKIT as i64, 2);
        p.inventory.add_item(items::AMMO as i64, 1);
        w.add_player(p);

        let dropped = w.drop_inventory_at(1, Vec3::ZERO);

        let inv = &w.get_player(1).unwrap().inventory;
        assert!(inv.has_item(items::RIFLE as i64, 1), "оружие остаётся");
        assert!(inv.has_item(items::LIGHT_ARMOR as i64, 1), "броня остаётся");
        assert!(
            !inv.has_item(items::MEDKIT as i64, 1),
            "расходники теряются"
        );
        assert!(
            !inv.has_item(items::AMMO as i64, 1),
            "патроны-предмет теряются"
        );
        assert_eq!(dropped.len(), 3, "2 аптечки + 1 пачка патронов");
        assert_eq!(w.loot.len(), 3);
    }

    #[test]
    fn death_loot_unique_across_deaths() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "a"));
        w.add_player(Player::new(2, "b"));
        w.get_player_mut(1).unwrap().inventory.add_item(1, 1);
        w.get_player_mut(2).unwrap().inventory.add_item(2, 1);

        w.drop_inventory_at(1, Vec3::new(1.0, 0.0, 0.0));
        w.drop_inventory_at(2, Vec3::new(2.0, 0.0, 0.0));

        assert_eq!(w.loot.len(), 2);
        assert_ne!(w.loot[0].id, w.loot[1].id);
        assert!(w.loot.iter().all(|l| l.id >= ids::LOOT_BASE));
    }

    #[test]
    fn creature_kill_produces_one_death_event() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.position = Vec3::new(0.0, 0.8, 0.0);
        p.health = 1.0;
        p.inventory.add_item(1, 1);
        w.add_player(p);
        w.creatures.push(Creature {
            id: 1,
            position: Vec3::new(0.5, 0.8, 0.0),
            health: 50.0,
            max_health: 50.0,
            state: CreatureState::Attack,
            target: Some(1),
            damage: 100.0,
            kind: String::new(),
        });

        w.tick(1.0 / 60.0);

        let deaths = w
            .events
            .iter()
            .filter(|e| matches!(e, GameEvent::PlayerDeath { player_id: 1, .. }))
            .count();
        assert_eq!(
            deaths, 1,
            "смерть от существа должна дать ровно одно событие"
        );
        assert!(w.loot.iter().any(|l| l.id >= ids::LOOT_BASE));
        assert_eq!(w.get_player(1).unwrap().health, 100.0);
    }

    #[test]
    fn death_emits_event_and_drops_inventory() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.inventory.add_item(1, 1);
        p.health = 0.0;
        w.add_player(p);

        w.tick(1.0 / 60.0);

        assert!(
            w.events
                .iter()
                .any(|e| matches!(e, GameEvent::PlayerDeath { player_id: 1, .. }))
        );
        assert!(w.loot.iter().any(|l| l.id >= ids::LOOT_BASE));
        assert!(w.get_player(1).unwrap().inventory.items.is_empty());
    }

    #[test]
    fn non_finite_input_does_not_corrupt_position() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        let before = w.get_player(1).unwrap().position;
        let cmd = InputCommand {
            seq: 1,
            yaw: f32::NAN,
            pitch: 0.0,
            move_x: 1.0,
            move_z: 0.0,
            dt: 1.0 / 60.0,
        };

        w.apply_input(1, &cmd);

        let p = w.get_player(1).unwrap();
        assert!(
            p.position.to_array().iter().all(|v| v.is_finite()),
            "NaN-ввод не должен попадать в мир"
        );
        assert_eq!(p.position, before, "команда должна быть отброшена целиком");
        assert_eq!(p.velocity.x, 0.0);
        assert_eq!(p.velocity.z, 0.0);
    }

    #[test]
    fn movement_vector_is_clamped() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        let cmd = InputCommand {
            seq: 1,
            yaw: 0.0,
            pitch: 0.0,
            move_x: 100.0,
            move_z: 100.0,
            dt: 1.0 / 60.0,
        };

        w.apply_input(1, &cmd);

        let p = w.get_player(1).unwrap();
        let speed = (p.velocity.x.powi(2) + p.velocity.z.powi(2)).sqrt();
        assert!(speed > 0.0, "игрок должен двигаться");
        assert!(
            speed <= MOVE_SPEED + 0.001,
            "скорость {speed} м/с превышает {MOVE_SPEED} м/с"
        );
    }

    #[test]
    fn pitch_is_clamped() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        let cmd = InputCommand {
            seq: 1,
            yaw: 0.0,
            pitch: 10.0,
            move_x: 0.0,
            move_z: 0.0,
            dt: 1.0 / 60.0,
        };

        w.apply_input(1, &cmd);

        let pitch = w.get_player(1).unwrap().pitch;
        assert!(
            pitch.abs() <= std::f32::consts::FRAC_PI_2 + 1e-4,
            "pitch {pitch} вышел за ±π/2"
        );
    }

    #[test]
    fn bounds_limit_movement() {
        let mut w = SimWorld::new();
        w.bounds = Bounds {
            min: Vec3::new(-5.0, 0.0, -5.0),
            max: Vec3::new(5.0, 10.0, 5.0),
        };
        let mut p = Player::new(1, "p");
        p.position = Vec3::new(4.0, 0.8, 4.0);
        // Заведомо больше границы: симуляция обязана зажать позицию.
        p.velocity = Vec3::new(500.0, 0.0, 500.0);
        w.add_player(p);

        w.tick(1.0);

        let pos = w.get_player(1).unwrap().position;
        assert!((pos.x - 5.0).abs() < 1e-3, "x={} вне границы", pos.x);
        assert!((pos.z - 5.0).abs() < 1e-3, "z={} вне границы", pos.z);
    }

    #[test]
    fn creature_respawns_after_delay() {
        let mut w = SimWorld::new();
        w.creature_spawns.push(CreatureSpawn {
            id: ids::CREATURE_BASE + 1,
            position: Vec3::new(3.0, 0.8, 3.0),
            kind: "snork".into(),
            health: 40.0,
            damage: 5.0,
            respawn_secs: 2.0,
        });
        w.creatures.push(Creature {
            id: ids::CREATURE_BASE + 1,
            position: Vec3::new(3.0, 0.8, 3.0),
            health: 0.0,
            max_health: 40.0,
            state: CreatureState::Patrol,
            target: None,
            damage: 5.0,
            kind: "snork".into(),
        });

        w.tick(0.5);
        assert!(w.creatures.is_empty(), "мёртвое существо должно уйти сразу");
        assert!(
            w.respawns.len() == 1,
            "смерть должна поставить респавн в очередь"
        );

        w.tick(1.0);
        assert!(
            w.creatures.is_empty(),
            "до истечения задержки существо не должно появиться"
        );

        w.tick(1.0);
        assert_eq!(w.creatures.len(), 1, "после задержки существо возвращается");
        assert_eq!(w.creatures[0].health, 40.0);
        assert_eq!(w.creatures[0].kind, "snork");
        assert!(w.respawns.is_empty(), "очередь должна опустеть");
    }

    fn command(seq: u32, move_x: f32, move_z: f32) -> InputCommand {
        InputCommand {
            seq,
            yaw: 0.0,
            pitch: 0.0,
            move_x,
            move_z,
            dt: 1.0 / 60.0,
        }
    }

    #[test]
    fn input_ack_ignores_stale_sequences() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));

        w.apply_input(1, &command(5, 0.0, 0.0));
        assert_eq!(w.get_player(1).unwrap().last_input_seq, 5);

        // Опоздавшая команда не должна откатывать ack и менять скорость.
        w.apply_input(1, &command(4, 1.0, 0.0));
        let p = w.get_player(1).unwrap();
        assert_eq!(p.last_input_seq, 5);
        assert_eq!(p.velocity.x, 0.0);
        assert_eq!(p.velocity.z, 0.0);
    }

    #[test]
    fn queued_input_is_applied_on_tick() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.position = Vec3::new(0.0, GROUND_Y, 0.0);
        w.add_player(p);

        w.queue_input(1, &command(1, 0.0, -1.0));
        assert_eq!(
            w.get_player(1).unwrap().velocity.x,
            0.0,
            "очередь не двигает игрока до тика"
        );

        w.tick(1.0 / 60.0);
        let p = w.get_player(1).unwrap();
        assert_eq!(p.last_input_seq, 1);
        assert!(p.position.x > 0.0, "игрок должен двинуться по +X");
    }

    #[test]
    fn wall_blocks_movement_and_allows_sliding() {
        let mut w = SimWorld::new();
        let mut p = Player::new(1, "p");
        p.position = Vec3::new(0.0, GROUND_Y, 0.0);
        w.add_player(p);
        // Стена x = 1..2, достаточно высокая, чтобы пересечься с телом.
        w.obstacles.push(Aabb::new(
            Vec3::new(1.5, 1.0, 0.0),
            Vec3::new(0.5, 1.0, 5.0),
        ));

        w.queue_input(1, &command(1, 0.0, -1.0));
        for _ in 0..120 {
            w.queue_input(1, &command(1, 0.0, -1.0));
            w.tick(1.0 / 60.0);
        }
        let p = w.get_player(1).unwrap();
        let stop = 1.0 - PLAYER_HALF_WIDTH;
        assert!(
            p.position.x <= stop + 1e-3,
            "x={} прошёл сквозь стену",
            p.position.x
        );
        assert!(
            p.position.x > stop - 0.1,
            "игрок не дошёл до стены: {}",
            p.position.x
        );

        // Вдоль стены движение сохраняется: блокируется только ось X.
        w.queue_input(1, &command(2, 1.0, 0.0));
        w.tick(1.0 / 60.0);
        let p = w.get_player(1).unwrap();
        assert!(
            p.position.z < -0.01,
            "скольжение по Z не работает: {:?}",
            p.position
        );
        assert!(
            (p.position.x - stop).abs() < 1e-2,
            "x сместился: {}",
            p.position.x
        );
    }

    #[test]
    fn shot_cooldown_limits_fire_rate() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));

        assert!(
            w.apply_shot(1, Vec3::new(0.0, EYE_HEIGHT, 0.0), Vec3::Z)
                .is_some()
        );
        assert!(
            w.apply_shot(1, Vec3::new(0.0, EYE_HEIGHT, 0.0), Vec3::Z)
                .is_none(),
            "второй выстрел внутри кулдауна должен отклоняться"
        );

        w.tick(0.3);
        assert!(
            w.apply_shot(1, Vec3::new(0.0, EYE_HEIGHT, 0.0), Vec3::Z)
                .is_some(),
            "после кулдауна выстрел снова разрешён"
        );
    }

    #[test]
    fn shot_spends_ammo_and_stops_when_empty() {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "p"));
        let start = w.get_player(1).unwrap().ammo;

        assert!(
            w.apply_shot(1, Vec3::new(0.0, EYE_HEIGHT, 0.0), Vec3::Z)
                .is_some()
        );
        assert_eq!(w.get_player(1).unwrap().ammo, start - 1);

        w.get_player_mut(1).unwrap().ammo = 0;
        w.get_player_mut(1).unwrap().shot_cooldown = 0.0;
        assert!(
            w.apply_shot(1, Vec3::new(0.0, EYE_HEIGHT, 0.0), Vec3::Z)
                .is_none(),
            "без патронов стрелять нельзя"
        );
    }
}

#[cfg(test)]
mod shot_tests;

#[cfg(test)]
mod lag_tests {
    use super::*;

    fn arena() -> (SimWorld, Vec3) {
        let mut w = SimWorld::new();
        w.add_player(Player::new(1, "shooter"));
        let mut victim = Player::new(2, "victim");
        victim.position = Vec3::new(0.0, 0.8, 3.0);
        w.add_player(victim);
        let origin = w.get_player(1).unwrap().position + Vec3::Y * EYE_HEIGHT;
        (w, origin)
    }

    #[test]
    fn firing_without_rewind_misses_moved_target() {
        let (mut w, origin) = arena();
        w.tick = 10;
        w.record_history();
        w.get_player_mut(2).unwrap().position.x = 2.0;
        w.tick = 11;

        let outcome = w.apply_shot_rewound(1, origin, Vec3::Z, 0).unwrap();
        assert_eq!(outcome.hit, None, "по текущей позиции цель уже ушла");
        assert!(!outcome.rewind_clamped);
    }

    #[test]
    fn rewind_hits_target_where_it_was_on_screen() {
        let (mut w, origin) = arena();
        w.tick = 10;
        w.record_history();
        w.get_player_mut(2).unwrap().position.x = 2.0;
        w.tick = 11;

        let outcome = w.apply_shot_rewound(1, origin, Vec3::Z, 1).unwrap();
        assert_eq!(outcome.hit, Some(2), "откат возвращает цель на линию огня");
        assert_eq!(outcome.rewind_tick, 10);
        assert!(!outcome.rewind_clamped);
    }

    #[test]
    fn stale_and_empty_history_are_explicitly_clamped() {
        let (mut w, _) = arena();
        assert_eq!(
            w.clamp_history_tick(5),
            (w.tick, true),
            "без истории откат невозможен, это видно вызывающему"
        );

        w.tick = 10;
        w.record_history();
        w.tick = 100;
        assert_eq!(
            w.clamp_history_tick(5),
            (10, true),
            "слишком старый тик зажимается к самому старому кадру"
        );
        assert_eq!(
            w.clamp_history_tick(200),
            (100, true),
            "будущий тик зажимается к текущему"
        );
        assert_eq!(w.clamp_history_tick(100), (100, false));
    }

    #[test]
    fn history_is_bounded_per_entity() {
        let (mut w, _) = arena();
        for _ in 0..1000 {
            w.record_history();
            w.tick += 1;
        }
        for ring in w.position_history.values() {
            assert!(
                ring.len() <= SimWorld::LAG_HISTORY_TICKS,
                "кольцо выросло до {}",
                ring.len()
            );
        }
        assert!(w.position_history.len() <= 2, "история только по живым");
    }
}
