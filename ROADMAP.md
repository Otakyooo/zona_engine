# ZONA Engine — дорожная карта

> **Сгенерировано автоматически. Не редактировать вручную.**
> Источник: `ci/roadmap.yaml`, генератор: `ci/self-check.js`. Статусы и метрики обновляются на каждом прогоне CI.

**Ось (уроборос):** Уроборос: код -> тесты/метрики -> память/журнал -> автоматически выведенный следующий шаг -> код

| Показатель | Значение | Источник |
|---|---|---|
| Версия | 0.3.1 | `Cargo.toml` [workspace.package] |
| Протокол | 8 | `crates/engine/zn-net/src/lib.rs:17` |
| Тесты | 144 | `cargo test --workspace -- --list` |
| Baseline | 144 тестов, 0.3.1, протокол 8, 452b0bb | `ci/roadmap.yaml` |
| Лимит кадра | `64 * 1024` | `zn-net/src/lib.rs` |
| Последний тег | v0.3.1 | `git describe` |

## Фазы

| Фаза | Версия | Приоритет | Статус | Цель | Критерий готовности | Следующий шаг |
|---|---|---|---|---|---|---|
| F0 | без релиза | P0 | готово | Сделать петлю уробороса измеряемой до первого изменения геймплея. | node ci/self-check.js --verify --generate зелёный | — |
| F1 | v0.4.0 | P0 | запланировано | Убрать продовые падения, закрыть дыры входа и флуда, сделать сохранение подтверждаемым — без правок провода. | >=160 тестов | zn-client/src/main.rs:1059-1066: guard Action::StartRaid при !protocol_ok; ui.rs:227,813,870: ui.add_enabled(protocol_ok, ...) для «В рейд»; ui.rs:895-915: тест menu(..., false, ...); отказ пишется log::error! на клиенте (main.rs:510-518) |
| F2 | v0.5.0 | P0/P1 | запланировано | Легализовать ломающие изменения провода и закрыть открытый трафик. | check-versions.js ok 0.5.0 / protocol 9 | PROTOCOL_VERSION 8 -> 9; Handshake/HandshakeAck/Reload/Trade строго в конец; удалить легаси TakeFromStash |
| F3 | v0.6.0 | P1 | запланировано | Разбить монолиты, удалить мёртвый код, включить полноценную петлю уробороса. | >=230 тестов (>=3 интеграционных, >=1 нагрузочный, >=1 golden-wire) | zn-server/src/main.rs (2263 строки) -> src/{net,sim,persist,db}/mod.rs; zn-client/src/main.rs -> src/app/*; инвариант AGENTS.md:101 |
| F4 | v0.7.0 | P2 | запланировано | Задействовать объявленные, но пустые поля симуляции. | >=270 тестов | Торговец/магазин, трата currency (zn-sim/src/lib.rs:547,745); LIGHT_ARMOR и гашение урона (items.rs:75,154) |
| F5 | v0.8.0 | P2 | запланировано | Расширить контент и физику, не ломая авторитарность. | >=310 тестов | Skill + квесты; аномалии/артефакты через zn_sim::ids |
| F6 | v0.9.0 | P3 | запланировано | Устранить рассинхрон документации и процессов. | cargo run -p zn-tools --bin zn-editor резолвится | zn-tools/Cargo.toml: явный [[bin]] name = zn-editor, поправить ci.yml manifest |
| F7 | v1.0.0 | P3 | запланировано | Стабилизация и доказательство, что петля работает без человека. | >=360 тестов (ориентир) | Полный проход GUI_CHECKLIST.md владельцем: сетки, перенос/поворот, Settings, RTT, откат, смерть/эвакуация, три карты |

### F0 — без релиза «Нулевой цикл: измеримая петля» (P0, готово)

Сделать петлю уробороса измеряемой до первого изменения геймплея.

**Задачи**

- ci/roadmap.yaml + ci/self-check.js: генерация ROADMAP.md, CHANGELOG.md, next-step.md без внешних зависимостей
- ci.yml: clippy --workspace --all-targets, test --workspace, env CARGO_INCREMENTAL=0, job self-check с diff-guard и [skip ci]
- AGENTS.md: зафиксировать правило «ROADMAP.md генерируемый, править ci/roadmap.yaml»
- journal/<дата>.md: baseline 144 теста, 0.3.1, протокол 8, зелёные проверки

**Критерии готовности**

- node ci/self-check.js --verify --generate зелёный
- ROADMAP.md перегенерируется без diff при повторном запуске
- Два прогона CI подряд не создают бесконечную цепочку коммитов
- baseline 144 теста подтверждён cargo test --workspace -- --list и записан в ci/artifacts/tests.json

- Источник метрик: ci/roadmap.yaml, ci/check-versions.js, ci/artifacts/tests.json
- Зависимости: —
- Риски: Бесконечный CI-цикл; CARGO_INCREMENTAL=0 обязателен

### F1 — v0.4.0 «Безопасность и надёжность» (P0, запланировано)

Убрать продовые падения, закрыть дыры входа и флуда, сделать сохранение подтверждаемым — без правок провода.

**Задачи**

- zn-client/src/main.rs:1059-1066: guard Action::StartRaid при !protocol_ok; ui.rs:227,813,870: ui.add_enabled(protocol_ok, ...) для «В рейд»; ui.rs:895-915: тест menu(..., false, ...); отказ пишется log::error! на клиенте (main.rs:510-518)
- zn-net/src/lib.rs: ActionRateLimiter для Shot/Consume/Connect/Register/Pickup/MoveItem; общий per-IP бакет; проверка телепорта в zn-sim
- zn-client/src/main.rs:1479: parse().unwrap() -> обрабатываемая ошибка + тест (144 -> 145); launcher/renderer.js:96,98: удалить двойной init()
- zn-server/src/main.rs:112, PersistJob/spawn_persister: ctrl_c, drain с подтверждением записи, сохранение по смерти/эвакуации/выходу

**Критерии готовности**

- >=160 тестов
- 0 unwrap() в сетевых путях
- тест: при protocol_ok=false кнопка «В рейд» отключена и Action::StartRaid не отправляется; тест graceful shutdown с подтверждённой записью
- 4 проверки + check-versions.js зелёные

- Источник метрик: cargo test --workspace, log::error! клиента (метрика отказа сервера — F2)
- Зависимости: F0
- Риски: ClientPacket не меняется, протокол 8; подпись обновлений — F6. Сервер шлёт Welcome{protocol} только в ответ на Connect и не знает версию клиента, поэтому запрет Connect и серверная метрика без правок провода невозможны

### F2 — v0.5.0 «Протокол 9, шифрование, живая сеть» (P0/P1, запланировано)

Легализовать ломающие изменения провода и закрыть открытый трафик.

**Задачи**

- PROTOCOL_VERSION 8 -> 9; Handshake/HandshakeAck/Reload/Trade строго в конец; удалить легаси TakeFromStash
- TLS/rustls для TCP: tokio-rustls, ZONA_TLS_CERT/ZONA_TLS_KEY, --tls-ca у клиента, deploy/README.md
- AEAD ChaCha20-Poly1305 для UDP: session_key в LoginResult, миграция 0010_session_keys.sql, nonce из session_id+seq, anti-replay
- Dual-read 8/9 без downgrade: Handshake у клиента 9, метрика protocol_8_clients, отключение legacy в v0.6.0
- Серверная метрика отказа по версии протокола (клиент сообщает версию в Handshake): счётчик + logs/metrics.jsonl; новые варианты enum только в конец
- Jitter buffer ввода, keyframe при реконнекте, оживить/удалить ReliableChannel, zn-net/tests/wire_compat.rs
- Релизная цепочка: пересборка payload -> npm run stub -> protocol в manifest.stub.json и check-versions.js -> версия/тег (только по запросу владельца)

**Критерии готовности**

- check-versions.js ok 0.5.0 / protocol 9
- >=190 тестов, wire_compat зелёный на 8 и 9
- тесты: plaintext отклонён, повтор seq отклонён, TCP+UDP через шифрованный канал
- AGENTS.md:105 и PITFALLS.md:26 обновлены под ключи сессий (D32)

- Источник метрик: zn-net/src/lib.rs:17, manifest.stub.json, ci/artifacts/load.json
- Зависимости: F0, F1
- Риски: Рассинхрон офлайн-payload; bincode без схемы; ротация ключа и expiry сессии (D33)

### F3 — v0.6.0 «Архитектура, тесты, метрики» (P1, запланировано)

Разбить монолиты, удалить мёртвый код, включить полноценную петлю уробороса.

**Задачи**

- zn-server/src/main.rs (2263 строки) -> src/{net,sim,persist,db}/mod.rs; zn-client/src/main.rs -> src/app/*; инвариант AGENTS.md:101
- Удалить или реализовать ChunkManager/ChunkEvent/load_chunk_async/chunk_of; отключить legacy-протокол 8
- Новые тесты: zn-server/tests/integration.rs, migrations.rs, zn-tools/src/bin/loadbot.rs
- --telemetry из заглушки в logs/metrics.jsonl: tick, snapshot bytes, RTT EWMA, FPS, видимые объекты
- Инстансинг / трансформ в uniform за feature flag; честный бюджет MAX_RENDER_OBJECTS
- ci/self-check.js читает реальные метрики, OPEN.md и GUI_CHECKLIST.md и генерирует CHANGELOG.md, next-step.md, статусы ROADMAP.md

**Критерии готовности**

- >=230 тестов (>=3 интеграционных, >=1 нагрузочный, >=1 golden-wire)
- бюджет тика <=16.67 мс при 50 ботах
- 0 мёртвых pub из OPEN.md:51; CI gate зелёный

- Источник метрик: logs/metrics.jsonl, ci/artifacts/load.json
- Зависимости: F2
- Риски: Рефактор ядра сети дробить на коммиты с 4 проверками; GUI в песочнице недоступен

### F4 — v0.7.0 «Экономика и боёвка» (P2, запланировано)

Задействовать объявленные, но пустые поля симуляции.

**Задачи**

- Торговец/магазин, трата currency (zn-sim/src/lib.rs:547,745); LIGHT_ARMOR и гашение урона (items.rs:75,154)
- Перезарядка и пополнение патронов; урон по своим при ExtractionSuccess
- Отдача и разброс с детерминированным seed, согласованным клиент+сервер
- Настройки (чувствительность, громкость) и выбор карты в zn-client/src/settings.rs и ui.rs

**Критерии готовности**

- >=270 тестов
- тесты: currency не уходит в минус, броня гасит N урона, детерминизм разброса на 1000 выстрелов
- интеграционный тест F3 ловит рассинхрон клиент/сервер

- Источник метрик: cargo test --workspace, ci/artifacts/load.json
- Зависимости: F3
- Риски: Клиентский урон запрещён REJECTED.md:11 — вся математика в zn-sim

### F5 — v0.8.0 «Мир и содержание» (P2, запланировано)

Расширить контент и физику, не ломая авторитарность.

**Задачи**

- Skill + квесты; аномалии/артефакты через zn_sim::ids
- maps/third.ron через ZoneMap; мипмапы на втором пути загрузки; несколько примитивов glTF
- Музыка/3D-звук и анимации поверх 6 SFX; редактор: мышь, undo, превью (писать ZoneMap целиком)
- Серверный PhysicsWorld за feature flag + ООББ вместо AABB; изменение Welcome.obstacles -> протокол 10
- Матчмейкинг/группы без общего таймера

**Критерии готовности**

- >=310 тестов
- 3 карты проходят ZoneMap::validate(); >=10 тестов PhysicsWorld
- бюджет тика вырос не более чем на 10%

- Источник метрик: maps/, logs/metrics.jsonl
- Зависимости: F3, F4
- Риски: Полная физика каждой пули запрещена; bbox-правки требуют протокола 10

### F6 — v0.9.0 «Инструменты и гигиена» (P3, запланировано)

Устранить рассинхрон документации и процессов.

**Задачи**

- zn-tools/Cargo.toml: явный [[bin]] name = zn-editor, поправить ci.yml manifest
- rustfmt.toml, clippy.toml, deny.toml, [profile.release], Dockerfile, docker-compose, docs/
- deploy/watchdog.ps1 -> сервис + healthcheck; deploy/README.md под TLS
- Postgres-фича: db/dialect.rs либо явное удаление + D34
- launcher/main.js:199 verifyUpdateCodeSignature = false -> true — релизное решение владельца: package.json signAndEditExecutable: false и сертификата нет, включение отклонит неподписанные обновления; нужны сертификат и CSC_LINK в CI
- Синхронизировать память: DECISIONS.md D27 устарело, AGENTS.md/PITFALLS.md под ключи сессий, закрытые пункты OPEN.md в journal

**Критерии готовности**

- cargo run -p zn-tools --bin zn-editor резолвится
- 4 проверки + check-versions.js зелёные; ROADMAP.md покрывает P0-P3 без ручных правок
- MEMORY.md <=150 строк; подпись обновлений либо включена с сертификатом, либо явно отложена с записью в DECISIONS.md

- Источник метрик: AGENTS.md, .workbuddy-ai/memory/
- Зависимости: F5
- Риски: Миграции без rollback — только аддитивные SQL, тест на копии zona.db

### F7 — v1.0.0 «Замкнутая петля: приёмка» (P3, запланировано)

Стабилизация и доказательство, что петля работает без человека.

**Задачи**

- Полный проход GUI_CHECKLIST.md владельцем: сетки, перенос/поворот, Settings, RTT, откат, смерть/эвакуация, три карты
- Живая сеть 2+ клиентов: реальный RTT, дельты, реконнект, нагрузочный отчёт
- Итоговый self-check: ROADMAP.md со статусами/метриками, CHANGELOG.md, next-step.md, 0 регрессий
- journal/<дата>.md с числами и хешами; релиз v1.0.0 только по явному запросу владельца

**Критерии готовности**

- >=360 тестов (ориентир)
- 0 непройденных пунктов GUI-чека кроме явно принятых
- 0 регрессий метрик; зелёный CI без ручного вмешательства в ROADMAP.md

- Источник метрик: GUI_CHECKLIST.md, ci/artifacts/load.json
- Зависимости: F6
- Риски: GUI-проверку выполняет только владелец; релиз — только по явному запросу

## Как замыкается петля

`ci/self-check.js` читает `ci/roadmap.yaml`, `Cargo.toml`, `PROTOCOL_VERSION` и `MAX_FRAME_BYTES` из `crates/engine/zn-net/src/lib.rs`, число тестов из `ci/artifacts/tests.json` (или `ZONA_TEST_COUNT`/`--with-cargo`), а также `git log` и список тегов. Из этого он генерирует этот файл, `CHANGELOG.md` и неотслеживаемые черновики `ci/artifacts/next-step.md`, `journal-draft.md`, `CHANGELOG-unreleased.md`; при регрессии числа тестов и расхождении версий падает. В CI `ROADMAP.md` коммитится только при реальном diff и с `[skip ci]`, иначе возник бы бесконечный цикл. Чтение `logs/metrics.jsonl`, `ci/artifacts/load.json`, `OPEN.md` и `GUI_CHECKLIST.md` запланировано в Фазе 3. Живую GUI-проверку вносит только владелец.

## Сквозные правила

- CARGO_INCREMENTAL=0 всегда; глобальный CARGO_TARGET_DIR запрещён
- После любого изменения: cargo fmt --all -- --check, cargo build --workspace, cargo test --workspace, cargo clippy --workspace --all-targets -- -D warnings
- Версия — один источник [workspace.package] version; цепочка Cargo.toml <-> launcher/package.json <-> manifest.stub.json <-> тег v* через ci/check-versions.js; после замены payload — npm run stub
- Новые варианты ClientPacket/ServerPacket только в конец; при bump протокола — dual-read, пересборка клиент+сервер+stub, поле protocol в манифесте
- Состояние в zn-sim, БД только у сервера; кадр <= 64 КиБ; id через zn_sim::ids
- Conventional Commits; старт задачи — MEMORY.md + OPEN.md; финал — journal/<дата>.md с числами и хешами
- ROADMAP.md генерируется ci/self-check.js, редактировать только ci/roadmap.yaml
- Релиз и живая GUI-проверка — только владелец; агент проверяет node --check и логи

## Риски и обходы

| Риск | Обход |
|---|---|
| Бесконечный цикл CI: автокоммит ROADMAP.md -> новый CI | diff-guard + [skip ci] + отдельный job с permissions: contents: write |
| GUI в песочнице агента не поднимается (-1073741819) | агент: node --check, логи, метрики; чек-лист выполняет владелец |
| Холодная сборка 0xc0000005 | CARGO_INCREMENTAL=0 ... -j 2, не чистить target/ |
| Рассинхрон офлайн-payload и протокола | чек-лист Фазы 2 + поле protocol в check-versions.js |
| bincode без схемы, позиционные дискриминанты | варианты в конец, PROTOCOL_VERSION, wire_compat.rs, dual-read |
| Downgrade до plaintext при dual-read | legacy только для старых токенов, метрика, отключение в v0.6.0 |
| Ключи сессий против запрета кэша токенов | ключ в таблице sessions, кэш по addr на время соединения, явное D32 |
| Postgres-фича мертва, datetime('now') SQLite-only | db/dialect.rs либо удаление фичи + D34 |
| Нет rollback у миграций | только аддитивные SQL, тест на копии zona.db, бэкап перед bump БД |
| AEAD добавляет байты к кадру | тест на MAX_FRAME_BYTES с учётом служебных данных |
| Рефактор main.rs ломает сеть | дробить на коммиты, 4 проверки после каждого, интеграционный тест Фазы 3 |
| Непроверенная живая сеть | loadbot, GUI_CHECKLIST.md, метрики RTT/откат, приёмка владельцем |

## Что не делаем

- battle-royale и общий таймер рейда
- полный лут при смерти
- принудительная эвакуация и мгновенный выход в базу
- клиентский урон и клиентский хитскан
- pay-to-win, лутбоксы, battle pass
- полная серверная физика каждой пули
- бесшовный мир без стриминга, третье лицо и транспорт
- персистентность каждой гильзы и трупа

---

Сгенерировано из `ci/roadmap.yaml` · чтобы изменить статус или задачу, правьте источник, не этот файл.
