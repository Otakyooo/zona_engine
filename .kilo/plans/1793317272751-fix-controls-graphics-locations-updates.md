# План: Исправление технического состояния (управление, графика, локации, обновления)

## Цель

Устранить четыре критические проблемы, из-за которых проект неиграбелен: сломанное управление, отсутствие базовой графики, отсутствие игровых локаций, ненадёжная доставка обновлений. Результат: играбельный рейд с рабочим управлением, базовой графикой и одной картой.

## Контекст

Текущее состояние:
- Управление: WASD + мышь + геймпад реализованы в `zn-input`, но интеграция в клиенте имеет проблемы (cursor capture, mouse delta timing, movement не применяется к камере корректно при сетевой игре).
- Графика: Lambert-шейдер с одной directional light, текстуры поддерживаются но не применяются (все мешы используют default white texture). Нет skybox, fog, теней.
- Локации: только процедурный пол 40×40 м с кубами. Нет реальной карты.
- Обновления: launcher fetches manifest from GitHub; version comparison mismatched (manifest "v0.1.3" vs local "0.1.3" → always needsUpdate). Если fetch падает, fallback на пустой stub → ничего не скачивается.

## Зафиксированные решения

- Управление: фикс cursor capture (winit 0.30 `CursorGrabMode::Locked`), mouse delta через `DeviceEvent::MouseMotion`, ESC toggle, применение движения к камере (не к физике) для предсказания.
- Графика: применять текстуры из сцены (если есть), добавить skybox (куб), fog (линейный), улучшить освещение (ambient + directional, без теней на старте).
- Локации: создать `maps/arena.ron` — арена 100×100 м с укрытиями, зоной экстракции, спавном. Загружать по умолчанию если `--scene` не указан.
- Обновления: нормализовать версии (strip 'v' prefix), кэшировать последний успешный манифест в userData, показывать ошибки явно, не молчать при сбое fetch.

## Затронутые границы

- `crates/zn-client`: input integration, rendering (textures, skybox, fog), default map loading.
- `crates/zn-render`: shader (fog, skybox), texture binding per mesh.
- `crates/zn-input`: cursor capture helper, input state cleanup.
- `launcher/main.js`: version normalization, manifest caching, error surfacing.
- `launcher/renderer.js`: error display, download button (уже есть).
- `assets/` или `maps/`: новый файл `arena.ron`.
- `crates/zn-server`: спавн сущностей на арене (уже частично).

## Порядок работ (фазы)

### Phase A — Управление (client)
1. Фикс cursor capture: при старте захватывать курсор (`CursorGrabMode::Locked`), при ESC — освобождать, при клике в окно — захватывать снова.
2. Mouse look: использовать `DeviceEvent::MouseMotion` (уже есть), применять sensitivity 0.002, clamp pitch ±1.4.
3. Движение: WASD → velocity в camera space, применять к `local_pos` (предсказание). Синхронизировать с сервером через reconciliation (уже есть, но порог 0.5 м слишком большой → уменьшить до 0.1 м).
4. Стрельба: ЛКМ → raycast на клиенте (визуал) + отправка `Shot` на сервер.
5. Подбор: E → отправка `Pickup { entity_id }` на сервер.
6. Еда/вода: 1/2 → отправка `Consume { item_id }`.

### Phase B — Графика (render)
1. Применять текстуры: если меш имеет текстуру (из сцены или glTF), использовать её; иначе default white.
2. Skybox: рендер куба с текстурой неба (или градиент) в отдельном render pass (depth = far).
3. Fog: линейный fog в фрагментном шейдере (на основе расстояния от камеры).
4. Улучшить освещение: ambient 0.3 + directional (0.5, 1.0, 0.3) с Lambert. Добавить вторую точечную лампу (опционально).

### Phase C — Локация (content)
1. Создать `maps/arena.ron`: арена 100×100 м, пол, стены, укрытия (кубы), зона экстракции, спавн существ, аномалии, лут.
2. Загружать `maps/arena.ron` по умолчанию если `--scene` не указан.
3. Сервер: спавн сущностей из `maps/arena.ron` (или hardcoded для MVP).

### Phase D — Автообновление (launcher)
1. Нормализовать версии: strip 'v' prefix в `check-update` и `fetchManifest`.
2. Кэшировать последний успешный манифест в `userData/manifest.cache.json`.
3. При сбое fetch — использовать кэш; если кэша нет — показать ошибку, не молчать.
4. `needsUpdate`: true если manifest.version > local.version ИЛИ exe отсутствует.
5. Показывать статус явно (уже есть, но улучшить).

## Валидация

- `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings`.
- Ручной smoke: клиент запускается, управление работает (WASD+мышь), графика показывает освещённую сцену с текстурами, арена загружается, лаунчер скачивает обновления.

## Риски

- Skybox + fog увеличивают сложность шейдера; начать с fog, skybox — backlog.
- Асинхронная загрузка карты может вызвать фризы; для MVP — синхронная загрузка при старте.
- Version normalization: нужно обновить и клиент (если он проверяет версию) — но клиент не проверяет версию, только лаунчер.

## Вне области

- Анимация (скелетная).
- Тени (shadow maps).
- Несколько карт.
- Полноценный редактор viewport.

## Следующий шаг

Реализация Phase A (управление) агентом с правами на запись/сборку, начиная с `zn-client` и `zn-input`.
