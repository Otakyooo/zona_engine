//! zn-content: карта зоны — геометрия плюс всё, что живёт на ней.
//!
//! Раньше существа, лут и аномалии были захардкожены в сервере под
//! комментарием «M4: spawn sample entities». Теперь источник истины — файл
//! карты, а сервер только загружает его и отдаёт симуляции.
//!
//! Слой намеренно тонкий: карта хранит типы `zn-sim` напрямую, без
//! промежуточных DTO и маппинга, который пришлось бы держать в согласии руками.
//! Обратной зависимости нет: `zn-sim` ничего не знает про карты.

use std::path::Path;

use serde::{Deserialize, Serialize};
use zn_scene::SceneNodeFile;
use zn_sim::{
    Aabb, Anomaly, Artifact, Bounds, Creature, CreatureSpawn, CreatureState, ExtractionZone,
    LootItem, SimWorld, ids,
};

/// Точка появления игрока на карте.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlayerSpawn {
    pub position: [f32; 3],
    #[serde(default)]
    pub yaw: f32,
}

/// Файл карты целиком.
///
/// Все секции кроме `nodes` — `#[serde(default)]`, поэтому карта, написанная
/// до появления секций, читается без правок.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ZoneMap {
    pub nodes: Vec<SceneNodeFile>,
    #[serde(default)]
    pub bounds: Bounds,
    #[serde(default)]
    pub players: Vec<PlayerSpawn>,
    #[serde(default)]
    pub creature_spawns: Vec<CreatureSpawn>,
    #[serde(default)]
    pub loot: Vec<LootItem>,
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
    #[serde(default)]
    pub anomalies: Vec<Anomaly>,
    #[serde(default)]
    pub extractions: Vec<ExtractionZone>,
}

/// Что именно не так с картой. Строкой это не описывается: сервер должен
/// уметь отличить «спавн за границей» от «два одинаковых id».
#[derive(Clone, Debug, PartialEq)]
pub enum Problem {
    OutsideBounds {
        what: &'static str,
        index: usize,
    },
    DuplicateId {
        what: &'static str,
        id: u64,
    },
    WrongIdRange {
        what: &'static str,
        id: u64,
    },
    NonPositive {
        what: &'static str,
        index: usize,
        value: f32,
    },
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Problem::OutsideBounds { what, index } => {
                write!(f, "{what} #{index} за границей карты")
            }
            Problem::DuplicateId { what, id } => write!(f, "{what} с id {id} встречается дважды"),
            Problem::WrongIdRange { what, id } => {
                write!(f, "{what} с id {id} вне своего диапазона id")
            }
            Problem::NonPositive { what, index, value } => {
                write!(f, "{what} #{index} имеет неположительное значение {value}")
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ZoneError {
    #[error("не удалось прочитать карту {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("карта {path} не разбирается: {source}")]
    Parse {
        path: String,
        #[source]
        source: ron::error::SpannedError,
    },
    #[error("карта {path} невалидна: {problems}")]
    Invalid { path: String, problems: String },
}

impl ZoneError {
    fn invalid(path: &Path, problems: Vec<Problem>) -> Self {
        let joined = problems
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        ZoneError::Invalid {
            path: path.display().to_string(),
            problems: joined,
        }
    }
}

impl ZoneMap {
    /// Читает карту и сразу проверяет её: невалидная карта не должна доехать
    /// до симуляции, иначе ошибку увидит уже игрок в виде спавна в стене.
    pub fn load(path: &Path) -> Result<Self, ZoneError> {
        let text = std::fs::read_to_string(path).map_err(|source| ZoneError::Io {
            path: path.display().to_string(),
            source,
        })?;
        let zone: ZoneMap = ron::from_str(&text).map_err(|source| ZoneError::Parse {
            path: path.display().to_string(),
            source,
        })?;
        let problems = zone.validate();
        if !problems.is_empty() {
            return Err(ZoneError::invalid(path, problems));
        }
        Ok(zone)
    }

    pub fn save(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())?;
        std::fs::write(path, text)?;
        Ok(())
    }

    pub fn validate(&self) -> Vec<Problem> {
        let mut problems = Vec::new();

        for (i, p) in self.players.iter().enumerate() {
            if !self.bounds.contains_xz(p.position.into()) {
                problems.push(Problem::OutsideBounds {
                    what: "точка спавна игрока",
                    index: i,
                });
            }
        }

        let mut creature_ids: Vec<u64> = Vec::new();
        for (i, c) in self.creature_spawns.iter().enumerate() {
            if !self.bounds.contains_xz(c.position) {
                problems.push(Problem::OutsideBounds {
                    what: "спавн существа",
                    index: i,
                });
            }
            if ids::classify(c.id) != Some(ids::IdKind::Creature) {
                problems.push(Problem::WrongIdRange {
                    what: "существо",
                    id: c.id,
                });
            }
            if creature_ids.contains(&c.id) {
                problems.push(Problem::DuplicateId {
                    what: "существо",
                    id: c.id,
                });
            }
            creature_ids.push(c.id);
            if c.health <= 0.0 {
                problems.push(Problem::NonPositive {
                    what: "здоровье существа",
                    index: i,
                    value: c.health,
                });
            }
            if c.respawn_secs < 0.0 {
                problems.push(Problem::NonPositive {
                    what: "задержка респавна существа",
                    index: i,
                    value: c.respawn_secs,
                });
            }
        }

        let mut loot_ids: Vec<u64> = Vec::new();
        for (i, l) in self.loot.iter().enumerate() {
            if !self.bounds.contains_xz(l.position) {
                problems.push(Problem::OutsideBounds {
                    what: "лут",
                    index: i,
                });
            }
            if ids::classify(l.id) != Some(ids::IdKind::Loot) {
                problems.push(Problem::WrongIdRange {
                    what: "лут",
                    id: l.id,
                });
            }
            if loot_ids.contains(&l.id) {
                problems.push(Problem::DuplicateId {
                    what: "лут",
                    id: l.id,
                });
            }
            loot_ids.push(l.id);
        }

        let mut anomaly_ids: Vec<u64> = Vec::new();
        for (i, a) in self.anomalies.iter().enumerate() {
            if !self.bounds.contains_xz(a.position) {
                problems.push(Problem::OutsideBounds {
                    what: "аномалия",
                    index: i,
                });
            }
            if a.radius <= 0.0 {
                problems.push(Problem::NonPositive {
                    what: "радиус аномалии",
                    index: i,
                    value: a.radius,
                });
            }
            if anomaly_ids.contains(&a.id) {
                problems.push(Problem::DuplicateId {
                    what: "аномалия",
                    id: a.id,
                });
            }
            anomaly_ids.push(a.id);
        }

        let mut extraction_ids: Vec<u64> = Vec::new();
        for (i, z) in self.extractions.iter().enumerate() {
            if !self.bounds.contains_xz(z.position) {
                problems.push(Problem::OutsideBounds {
                    what: "зона эвакуации",
                    index: i,
                });
            }
            if z.radius <= 0.0 {
                problems.push(Problem::NonPositive {
                    what: "радиус зоны эвакуации",
                    index: i,
                    value: z.radius,
                });
            }
            if extraction_ids.contains(&z.id) {
                problems.push(Problem::DuplicateId {
                    what: "зона эвакуации",
                    id: z.id,
                });
            }
            extraction_ids.push(z.id);
        }

        problems
    }

    /// Наполняет мир содержимым карты.
    ///
    /// `seed_loot_ids` вызывается после расстановки: иначе счётчик выпавшего
    /// лута стартовал бы ниже лута карты и id пересеклись бы.
    pub fn apply_to(&self, world: &mut SimWorld) {
        world.bounds = self.bounds;
        if let Some(spawn) = self.players.first() {
            world.player_spawn = spawn.position.into();
        }
        for c in &self.creature_spawns {
            world.creatures.push(Creature {
                id: c.id,
                position: c.position,
                health: c.health,
                max_health: c.health,
                state: CreatureState::Patrol,
                target: None,
                damage: c.damage,
                kind: c.kind.clone(),
            });
        }
        world.creature_spawns = self.creature_spawns.clone();
        world.loot.extend(self.loot.iter().cloned());
        world.artifacts.extend(self.artifacts.iter().cloned());
        world.anomalies.extend(self.anomalies.iter().cloned());
        world
            .extraction_zones
            .extend(self.extractions.iter().cloned());
        world.seed_loot_ids();
    }

    /// Статические боксы карты для рейкаста.
    ///
    /// Поворот узла не учитывается: арена собрана из осей-выровненных блоков,
    /// а для повёрнутых боксов нужен полноценный ООББ-тест, которого пока нет.
    pub fn obstacles(&self) -> Vec<Aabb> {
        self.nodes
            .iter()
            .map(|n| Aabb::new(n.transform.position, n.transform.scale.abs() * 0.5))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"(
        nodes: [],
    )"#;

    const FULL: &str = r#"(
        nodes: [],
        bounds: (min: (-10.0, 0.0, -10.0), max: (10.0, 0.0, 10.0)),
        players: [(position: (0.0, 0.8, 0.0), yaw: 0.0)],
        creature_spawns: [
            (id: 3000000, position: (5.0, 0.8, 5.0), kind: "snork", health: 50.0, damage: 5.0, respawn_secs: 30.0),
        ],
        loot: [
            (id: 1000000, position: (2.0, 0.5, 2.0), item: (id: 1, name: "Medkit", value: 50, consumable: true)),
        ],
        anomalies: [
            (id: 1, position: (3.0, 0.0, 3.0), radius: 2.0, damage: 5.0, radiation: 1.0),
        ],
        extractions: [
            (id: 1, position: (8.0, 0.0, 8.0), radius: 2.0),
        ],
    )"#;

    #[test]
    fn minimal_map_uses_defaults() {
        let zone: ZoneMap = ron::from_str(MINIMAL).unwrap();
        assert!(zone.nodes.is_empty());
        assert!(zone.creature_spawns.is_empty());
        assert_eq!(zone.bounds.min, Bounds::default().min);
        assert_eq!(zone.bounds.max, Bounds::default().max);
    }

    #[test]
    fn full_map_parses_all_sections() {
        let zone: ZoneMap = ron::from_str(FULL).unwrap();
        assert_eq!(zone.players.len(), 1);
        assert_eq!(zone.creature_spawns.len(), 1);
        assert_eq!(zone.creature_spawns[0].kind, "snork");
        assert_eq!(zone.loot.len(), 1);
        assert_eq!(zone.anomalies.len(), 1);
        assert_eq!(zone.extractions.len(), 1);
        assert!(zone.validate().is_empty(), "{:?}", zone.validate());
    }

    #[test]
    fn validate_flags_spawn_outside_bounds() {
        let mut zone: ZoneMap = ron::from_str(FULL).unwrap();
        zone.creature_spawns[0].position = glam::Vec3::new(50.0, 0.8, 0.0);
        let problems = zone.validate();
        assert!(
            problems.iter().any(|p| matches!(
                p,
                Problem::OutsideBounds {
                    what: "спавн существа",
                    ..
                }
            )),
            "{problems:?}"
        );
    }

    #[test]
    fn validate_flags_duplicate_loot_id() {
        let mut zone: ZoneMap = ron::from_str(FULL).unwrap();
        let dup = zone.loot[0].clone();
        zone.loot.push(dup);
        let problems = zone.validate();
        assert!(
            problems
                .iter()
                .any(|p| matches!(p, Problem::DuplicateId { what: "лут", .. })),
            "{problems:?}"
        );
    }

    #[test]
    fn validate_flags_creature_id_outside_range() {
        let mut zone: ZoneMap = ron::from_str(FULL).unwrap();
        zone.creature_spawns[0].id = 1;
        let problems = zone.validate();
        assert!(
            problems.iter().any(|p| matches!(
                p,
                Problem::WrongIdRange {
                    what: "существо",
                    id: 1
                }
            )),
            "{problems:?}"
        );
    }

    #[test]
    fn apply_to_fills_world_and_seeds_loot_ids() {
        let zone: ZoneMap = ron::from_str(FULL).unwrap();
        let mut world = SimWorld::new();
        zone.apply_to(&mut world);

        assert_eq!(world.creatures.len(), 1);
        assert_eq!(world.creatures[0].kind, "snork");
        assert_eq!(world.loot.len(), 1);
        assert_eq!(world.anomalies.len(), 1);
        assert_eq!(world.extraction_zones.len(), 1);
        assert_eq!(world.bounds.min, zone.bounds.min);
        assert_eq!(world.player_spawn, glam::Vec3::new(0.0, 0.8, 0.0));
        // Счётчик выпавшего лута должен быть выше всего, что уже лежит на карте.
        assert!(world.next_loot_id > zone.loot[0].id);
        // Шаблоны нужны, чтобы существо вернулось в мир после смерти.
        assert_eq!(world.creature_spawns.len(), 1);
    }

    /// Все отгружаемые карты обязаны читаться и проходить валидацию: иначе
    /// сервер упадёт на старте, а правка RON руками останется незамеченной.
    ///
    /// Обход каталога, а не список файлов: новая карта попадает под проверку
    /// сама, забыть её невозможно, и CI (`cargo test --all`) ловит поломку.
    #[test]
    fn every_shipped_map_is_valid() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../maps");
        let mut maps = 0;
        for entry in std::fs::read_dir(&dir).expect("каталог maps должен существовать")
        {
            let path = entry.expect("запись каталога").path();
            if path.extension().and_then(|e| e.to_str()) != Some("ron") {
                continue;
            }
            let zone = ZoneMap::load(&path).unwrap_or_else(|e| panic!("{e}"));
            assert!(
                !zone.nodes.is_empty(),
                "{}: потеряна геометрия",
                path.display()
            );
            assert!(
                !zone.players.is_empty(),
                "{}: потеряны точки спавна игрока",
                path.display()
            );
            assert!(
                !zone.extractions.is_empty(),
                "{}: потеряна зона эвакуации",
                path.display()
            );
            maps += 1;
        }
        assert!(maps >= 2, "ожидались хотя бы две карты, найдено {maps}");
    }
}
