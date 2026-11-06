# CHANGELOG

> **Сгенерировано автоматически** `ci/self-check.js` по тегам и Conventional Commits. Не редактировать вручную.

## v0.3.1

### Новое

- `9045035` feat(inventory): grid raid/stash inventory, authoritative MoveItem, protocol 8

### Документация и память

- `23fcbbd` docs(memory): E:\zona_engine removed by owner; protocol 8 current
- `1e9735b` docs(memory): record the 0.3.0 release outcome

### Релизы и инфраструктура

- `a1780a8` chore(release): 0.3.1 — protocol-8 client, version bump

## v0.3.0

### Новое

- `323f098` feat: 0.3.0 — inventory, stash, raid flow, lag compensation

### Документация и память

- `5020bbf` docs(memory): record the 0.2.0 release outcome

### Релизы и инфраструктура

- `eb0e478` chore(release): point the offline payload at the 0.2.0 client

## v0.2.0

### Новое

- `0dc57ba` feat: 0.2.0 — hit zones, silhouettes, interpolation, session screens

## v0.1.9

### Новое

- `e5cffa4` feat(audio): synthesized sound effects on kira

## v0.1.8

### Новое

- `6fdb588` feat(client): status toasts and connection-loss detection
- `925d020` feat: send xp in snapshots and inventory updates over UDP
- `e967a7b` feat(client): HUD with crosshair, vitals, inventory and extraction bar
- `6917124` feat(render): draw egui over the scene in a second pass
- `7678582` feat(server): DB-backed sessions and per-IP rate limiting
- `e80086d` feat(server): id namespaces and input validation
- `88d1f03` feat(client): bind materials to scene and entities
- `977aad7` feat(render): make_box geometry with per-face UVs and real scale
- `3014dbd` feat(scene): material reference and texel density on nodes
- `53e1502` feat(tools): procedural seamless texture generator
- `81f5106` feat(assets): box-filter mipmap generation

### Исправления

- `5210caa` fix(net): UDP packets are raw bincode, not length-prefixed frames
- `105db46` fix: ship assets with releases and resolve them next to the exe
- `a6e499c` fix(client): require real credentials and stop clobbering view angles
- `abc4598` fix(net): cap frame size before allocating
- `85fd626` fix(sim): collect deaths after creature damage
- `77edb80` fix(sim): sim owns death loot, ids and position
- `8548186` fix(server): persist extracted inventory before clearing it
- `2f552eb` fix(server): hydrate level, xp and currency on connect
- `c619413` fix(render): sample textures with Repeat and linear mip filtering

### Документация и память

- `4a018f6` docs: describe the texture pipeline

### Релизы и инфраструктура

- `816a76e` chore: migrate workspace to WorkBuddy and fix repo defects

### Тесты

- `be89a50` test: stabilization coverage and session indexes

### Прочее

- `440f358` ci: make the release job independent of the launcher build
- `b70f494` build: add egui-wgpu for the HUD

## v0.1.6

### Исправления

- `5d1adef` fix: invert camera yaw (mouse/gamepad)

## v0.1.5

### Исправления

- `b0e1454` fix: align WASD movement with camera yaw

## v0.1.4

### Новое

- `721ffad` feat: full refactor (netcode identity, persistence, controls, render)
- `9289406` feat: controls (shoot/pickup/consume/cursor), fog, arena map
- `2e37b57` feat: M2.1 content pipeline (textures+mipmaps, baker, async streaming, LOD)

### Релизы и инфраструктура

- `f4cc65d` chore: remove dist_check artifacts
- `ec6f4d7` chore: remove dist_check artifacts

## v0.1.3

### Исправления

- `8ff2e6b` fix: update default server address to 192.168.0.47 (VPS)

### Релизы и инфраструктура

- `1cfe822` chore: bump launcher version to 0.1.3 (server 192.168.0.47)
- `5098d95` chore: remove dist_check artifacts

## v0.1.2

### Исправления

- `d9a93c1` fix: launcher buttons (syntax error, defaultGameDir, install-dir dialog)

### Релизы и инфраструктура

- `fc67ada` chore: bump launcher version to 0.1.2
- `b49ec73` chore: remove dist_check artifacts

## v0.1.1

### Исправления

- `7a5dfab` fix: launcher robustness (folder picker, launch check, error handling, logs)

### Релизы и инфраструктура

- `ca40966` chore: bump launcher version to 0.1.1
- `bb45d3d` chore: remove dist_check artifacts

## v0.1.0

### Новое

- `7e3e033` feat: launcher self-update (electron-updater + NSIS)
- `877bb8f` feat: user-selectable install directory with migration
- `60b44d8` feat: auto-update check on launcher startup and README for testers
- `10d56f5` feat: publish-ready launcher (GitHub manifest) and CI release
- `bdac60b` feat: configurable server address/ports and deploy scripts for dev server
- `1f75d9f` feat: M4.0 gameplay data model and sim (anomalies, artifacts, loot, creatures, extraction, survival, progression)
- `db9863d` feat: M3.1 UDP transport and interest management
- `88268b3` feat: M7 release hygiene (crash log, installer, CI release)

### Исправления

- `1939163` fix: move electron to devDependencies for builder
- `03f4ea4` fix: align wgpu uniform buffer size with shader (112 bytes)

### Документация и память

- `e709569` docs: add repository URL to AGENTS.md

### Релизы и инфраструктура

- `9284274` chore: remove dist_check artifacts from git
- `2a00f37` chore: add .gitattributes for LF line endings
- `1317a00` chore: initial commit

### Прочее

- `0a1879c` ci: avoid auto-publish in build-launcher, set GH_TOKEN
- `0cbb55c` ci: publish launcher via electron-builder for latest.yml
- `1f13575` ci: upload only top-level launcher files (no win-unpacked)
- `ba79834` ci: upload full launcher dist dir for latest.yml
- `8d06b99` ci: explicitly upload latest.yml for electron-updater
- `122adbc` ci: share launcher dist via artifacts
- `90980ee` ci: add contents:write permission for release job
- `0fe9768` ci: fix release job definition
- `27dc6a4` ci: sync ci/ci.yml with workflow
- `ece78bd` ci: add build-launcher job to workflow
- `95b483b` ci: add GitHub Actions workflow (triggers on tags)
- `ba3e664` ci: trigger release on version tags
- `6f83ae6` style: cargo fmt

---

Коммиты после последнего тега см. в `ci/artifacts/CHANGELOG-unreleased.md` (не отслеживается git).
