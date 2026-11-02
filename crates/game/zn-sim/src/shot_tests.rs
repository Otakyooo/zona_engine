//! Тесты рейкаста и разрешения выстрела.
//!
//! Вынесены отдельным модулем: в `tests` уже живут движение и выживание, а
//! боёвка — самостоятельный слой, который удобнее читать целиком.

use super::*;

fn shooter() -> Player {
    let mut p = Player::new(1, "shooter");
    p.position = Vec3::ZERO;
    p
}

fn creature_at(id: u64, position: Vec3, health: f32) -> Creature {
    Creature {
        id,
        position,
        health,
        max_health: health,
        state: CreatureState::Patrol,
        target: None,
        damage: 5.0,
        kind: "snork".into(),
    }
}

/// Направление из глаз стрелка в существо высотой 1.2 м на расстоянии 5 м.
const AIM_DOWN: Vec3 = Vec3::new(0.0, -0.17, 1.0);

#[test]
fn ray_aabb_hits_box_in_front() {
    let b = Aabb::new(Vec3::new(0.0, 0.0, 5.0), Vec3::splat(1.0));
    let t = ray_aabb(Vec3::ZERO, Vec3::Z, 10.0, &b).expect("луч должен попасть");
    assert!((t - 4.0).abs() < 1e-4, "t={t}");
}

#[test]
fn ray_aabb_misses_parallel_box() {
    let b = Aabb::new(Vec3::new(0.0, 0.0, 5.0), Vec3::splat(1.0));
    assert!(ray_aabb(Vec3::new(5.0, 0.0, 0.0), Vec3::Z, 10.0, &b).is_none());
}

#[test]
fn ray_aabb_from_inside_returns_zero() {
    let b = Aabb::new(Vec3::new(0.0, 0.0, 5.0), Vec3::splat(1.0));
    assert_eq!(
        ray_aabb(Vec3::new(0.0, 0.0, 5.0), Vec3::Z, 10.0, &b),
        Some(0.0)
    );
}

#[test]
fn shot_hits_creature_in_front() {
    let mut w = SimWorld::new();
    w.add_player(shooter());
    let cid = ids::CREATURE_BASE + 1;
    w.creatures
        .push(creature_at(cid, Vec3::new(0.0, 0.8, 5.0), 50.0));

    let outcome = w
        .apply_shot(1, Vec3::new(0.0, 2.4, 0.0), AIM_DOWN, &[])
        .expect("выстрел должен быть принят");

    assert_eq!(outcome.hit, Some(cid));
    assert!(!outcome.killed);
    assert!(w.creatures[0].health < 50.0);
}

#[test]
fn shot_misses_when_creature_is_behind() {
    let mut w = SimWorld::new();
    w.add_player(shooter());
    w.creatures.push(creature_at(
        ids::CREATURE_BASE + 1,
        Vec3::new(0.0, 0.8, 5.0),
        50.0,
    ));

    let outcome = w
        .apply_shot(1, Vec3::new(0.0, 2.4, 0.0), Vec3::new(0.0, 0.0, -1.0), &[])
        .expect("выстрел должен быть принят");

    assert_eq!(outcome.hit, None);
    assert_eq!(w.creatures[0].health, 50.0);
}

#[test]
fn wall_blocks_shot() {
    let mut w = SimWorld::new();
    w.add_player(shooter());
    w.creatures.push(creature_at(
        ids::CREATURE_BASE + 1,
        Vec3::new(0.0, 0.8, 5.0),
        50.0,
    ));
    let wall = Aabb::new(Vec3::new(0.0, 2.0, 2.5), Vec3::new(3.0, 2.0, 0.2));

    let outcome = w
        .apply_shot(1, Vec3::new(0.0, 2.4, 0.0), AIM_DOWN, &[wall])
        .expect("выстрел должен быть принят");

    assert_eq!(outcome.hit, None, "стена ближе цели — попадания нет");
    assert_eq!(w.creatures[0].health, 50.0);
}

#[test]
fn shot_from_foreign_origin_rejected() {
    let mut w = SimWorld::new();
    w.add_player(shooter());

    assert!(
        w.apply_shot(1, Vec3::new(100.0, 100.0, 100.0), Vec3::Z, &[])
            .is_none(),
        "стрельба из чужой точки должна отклоняться"
    );
    assert!(
        w.apply_shot(1, Vec3::new(0.0, 2.4, 0.0), Vec3::ZERO, &[])
            .is_none(),
        "нулевое направление недопустимо"
    );
}

#[test]
fn kill_awards_xp_once() {
    let mut w = SimWorld::new();
    w.add_player(shooter());
    let cid = ids::CREATURE_BASE + 1;
    w.creatures
        .push(creature_at(cid, Vec3::new(0.0, 0.8, 5.0), 20.0));

    let outcome = w
        .apply_shot(1, Vec3::new(0.0, 2.4, 0.0), AIM_DOWN, &[])
        .expect("выстрел принят");

    assert!(outcome.killed);
    assert_eq!(w.get_player(1).unwrap().xp, CREATURE_KILL_XP);
    assert!(w.events.iter().any(|e| matches!(
        e,
        GameEvent::CreatureKilled { creature_id, xp, .. }
            if *creature_id == cid && *xp == CREATURE_KILL_XP
    )));

    w.tick(1.0 / 60.0);
    assert!(w.creatures.is_empty());
    assert_eq!(
        w.get_player(1).unwrap().xp,
        CREATURE_KILL_XP,
        "повторной награды быть не должно"
    );
}

#[test]
fn shot_kills_player_and_drops_loot_once() {
    let mut w = SimWorld::new();
    w.add_player(shooter());
    let mut victim = Player::new(2, "victim");
    victim.position = Vec3::new(0.0, 0.8, 3.0);
    victim.health = 20.0;
    victim.inventory.add_item(1, 1);
    w.add_player(victim);

    let outcome = w
        .apply_shot(1, Vec3::new(0.0, 2.4, 0.0), Vec3::Z, &[])
        .expect("выстрел принят");

    assert_eq!(outcome.hit, Some(2), "попадание по игроку должно работать");
    assert!(outcome.killed);
    assert_eq!(w.get_player(2).unwrap().health, 0.0);

    w.tick(1.0 / 60.0);
    let events = w.take_events();
    let deaths = events
        .iter()
        .filter(|e| matches!(e, GameEvent::PlayerDeath { player_id: 2, .. }))
        .count();
    assert_eq!(deaths, 1, "смерть должна случиться ровно один раз");
    assert_eq!(w.loot.len(), 1, "инвентарь выпал ровно один раз");

    w.tick(1.0 / 60.0);
    assert_eq!(w.loot.len(), 1, "второго лута быть не должно");
    assert!(
        !w.take_events()
            .iter()
            .any(|e| matches!(e, GameEvent::PlayerDeath { player_id: 2, .. })),
        "второго события смерти быть не должно"
    );
}

fn shooter_with(weapon: &str) -> Player {
    let mut p = shooter();
    p.equipped_weapon = weapon.into();
    p
}

/// Стоящий на земле игрок в 3 м по +Z: глаза стрелка на 1.6 м.
fn duel(weapon: &str) -> SimWorld {
    let mut w = SimWorld::new();
    w.add_player(shooter_with(weapon));
    let mut victim = Player::new(2, "victim");
    victim.position = Vec3::new(0.0, 0.0, 3.0);
    w.add_player(victim);
    w
}

#[test]
fn headshot_kills_with_one_rifle_bullet() {
    let mut w = duel("rifle");

    // Чуть вверх: к голове жертвы (1.55…1.8 м) на дистанции 3 м.
    let outcome = w
        .apply_shot(1, Vec3::new(0.0, 1.6, 0.0), Vec3::new(0.0, 0.05, 1.0), &[])
        .expect("выстрел принят");

    assert_eq!(outcome.hit, Some(2));
    assert_eq!(outcome.zone, Some(HitZone::Head));
    assert!(
        (outcome.damage - 105.0).abs() < 1e-3,
        "35 × 3 = 105, получено {}",
        outcome.damage
    );
    assert!(
        outcome.killed,
        "винтовка в голову должна убивать с одного раза"
    );
    assert_eq!(w.get_player(2).unwrap().health, 0.0);
}

#[test]
fn body_shot_deals_base_damage() {
    let mut w = duel("rifle");

    // В грудь: на дистанции 3 м луч идёт на 1.0 м.
    let outcome = w
        .apply_shot(1, Vec3::new(0.0, 1.6, 0.0), Vec3::new(0.0, -0.2, 1.0), &[])
        .expect("выстрел принят");

    assert_eq!(outcome.zone, Some(HitZone::Body));
    assert!(
        (outcome.damage - 35.0).abs() < 1e-3,
        "damage={}",
        outcome.damage
    );
    assert!(!outcome.killed);
    assert!((w.get_player(2).unwrap().health - 65.0).abs() < 1e-3);
}

#[test]
fn shot_above_head_misses() {
    let mut w = duel("rifle");

    let outcome = w
        .apply_shot(1, Vec3::new(0.0, 1.6, 0.0), Vec3::new(0.0, 0.3, 1.0), &[])
        .expect("выстрел принят");

    assert_eq!(outcome.hit, None);
    assert_eq!(outcome.zone, None);
    assert_eq!(outcome.damage, 0.0);
    assert_eq!(w.get_player(2).unwrap().health, 100.0);
}

/// Выстрел на уровне глаз попадает в голову: глаза на 1.6 м, голова — 1.55…1.8 м.
/// Поведение зафиксировано тестом, чтобы его нельзя было изменить молча.
#[test]
fn eye_level_shot_counts_as_head() {
    let mut w = duel("pistol");

    let outcome = w
        .apply_shot(1, Vec3::new(0.0, 1.6, 0.0), Vec3::Z, &[])
        .expect("выстрел принят");

    assert_eq!(outcome.zone, Some(HitZone::Head));
    assert!((outcome.damage - 60.0).abs() < 1e-3, "20 × 3 = 60");
    assert!((w.get_player(2).unwrap().health - 40.0).abs() < 1e-3);
}

#[test]
fn headshot_scales_creature_damage() {
    let mut w = SimWorld::new();
    w.add_player(shooter_with("pistol"));
    let cid = ids::CREATURE_BASE + 1;
    w.creatures
        .push(creature_at(cid, Vec3::new(0.0, 0.8, 5.0), 100.0));

    // Существо стоит с 0.8 м, голова — 1.75…2.0 м: луч приходит на 1.9 м.
    let outcome = w
        .apply_shot(1, Vec3::new(0.0, 1.6, 0.0), Vec3::new(0.0, 0.06, 1.0), &[])
        .expect("выстрел принят");

    assert_eq!(outcome.hit, Some(cid));
    assert_eq!(outcome.zone, Some(HitZone::Head));
    assert!(
        (outcome.damage - 60.0).abs() < 1e-3,
        "damage={}",
        outcome.damage
    );
    assert!((w.creatures[0].health - 40.0).abs() < 1e-3);
}
