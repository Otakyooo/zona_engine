# ZONA — индекс памяти

Точка входа для агента. Держать ≤150 строк: детали живут в файлах ниже, а не здесь.

| Файл | Что внутри |
|---|---|
| `PITFALLS.md` | Подводные камни: симптом → причина → что делать |
| `DECISIONS.md` | ADR-lite: принятые архитектурные решения и их следствия |
| `OPEN.md` | Открытые задачи по приоритету |
| `REJECTED.md` | Отбракованные идеи и чем заменены |
| `journal/YYYY-MM-DD.md` | Дневник: что сделано, зачем, проверки, числа |
| `../archive/` | Устаревшие артефакты (например `manifest-v0.1.6.json`) |

## Что это за проект

Extraction-shooter в стиле Stalker/Zone: собственный движок на Rust (wgpu 24, winit 0.30, egui, rapier3d), авторитарный сервер (TCP + UDP), SQLite/Postgres, Electron-лаунчер. 13 крейтов, Rust edition 2024, 128 тестов.

- `crates/engine/` — zn-core, zn-render, zn-physics, zn-audio, zn-assets, zn-input, zn-net, zn-scene
- `crates/game/` — zn-sim (общая симуляция), zn-content
- `crates/zn-client`, `crates/zn-server`, `crates/zn-tools`
- `launcher/` — Electron + Node, `deploy/`, `ci/`

## Расположение

- Рабочая папка сессии 2026-11-03: `E:\zona_engine2`.
- Каноническая: `C:\Users\russi\WorkBuddy AI\Zona_games` — путь с пробелом, квотировать в shell.
- Старый бэкап: `E:\zona_engine`; артефакты оттуда могут содержать зашитый старый `CARGO_MANIFEST_DIR` (см. `PITFALLS.md`).
- Remote: `https://github.com/Otakyooo/zona_engine`, ветка `main`. История перенесена локальным клоном 2026-10-31.

## Команды

```
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p zn-server          # TCP 7777 / UDP 7778, DATABASE_URL → SQLite
cargo run -p zn-client          # --scene --token --username --password
cargo run -p zn-tools --bin zn-editor
cargo run -p zn-tools --bin zn-gentex   # регенерация текстур
cd launcher && npm start        # требует unset ELECTRON_RUN_AS_NODE
node ci/check-versions.js       # версии: Cargo.toml ↔ launcher (шаг CI)
cd launcher && npm run stub     # пересобрать offline-манифест после замены payload
```

Версия продукта живёт только в `[workspace.package] version` (D22); тег `v*` без бампа валит CI.

**Всегда** собирать с `CARGO_INCREMENTAL=0` — иначе сборка виснет (см. `PITFALLS.md`).

## Как обновлять память

- Начало задачи: прочитать этот файл и `OPEN.md`.
- Решение, которое не очевидно из кода → `DECISIONS.md` (номер, контекст, следствия).
- Конец задачи → дописать в `journal/<сегодня>.md`: что изменено, почему, какие проверки прошли, числа.
- Этот файл не растёт: подробности вытеснять в `PITFALLS.md`, `DECISIONS.md` или журнал.
