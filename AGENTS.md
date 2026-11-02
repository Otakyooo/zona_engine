# ZONA Engine — Agent Guide

## Overview

ZONA is a from-scratch 3D FPS extraction shooter (Stalker/Zone theme) with a custom Rust engine, dedicated MMO-style server, and an Electron launcher. Low poly realism art style. Windows x64 target.

## Workspace layout

```
crates/
  engine/   zn-core, zn-render, zn-physics, zn-audio, zn-assets, zn-input, zn-net, zn-scene
  game/     zn-sim, zn-content
  zn-client, zn-server, zn-tools (editor)
launcher/   Electron + Node.js launcher
ci/         GitHub Actions
migrations/ (in zn-server)
```

## Build & test

- Build all: `cargo build --workspace`
- Test all: `cargo test --workspace`
- Lint: `cargo clippy --all-targets -- -D warnings`
- Format: `cargo fmt --all`
- Client: `cargo run -p zn-client` (optional `--scene`, `--token`, `--username`, `--password`)
- Server: `cargo run -p zn-server` (env `DATABASE_URL` optional, defaults to sqlite)
- Editor: `cargo run -p zn-tools --bin zn-editor`
- Launcher: `cd launcher; npm start` (requires `ELECTRON_RUN_AS_NODE` unset)

## Conventions

- Rust edition 2024, resolver 2.
- Conventional Commits; branches `main`/`dev`.
- Shared simulation logic lives in `zn-sim` (used by both client and server). No render/platform code in `zn-sim`.
- Server is authoritative; client predicts local player.
- All text UTF-8.

## Milestones

- M0: workspace scaffold — done
- M1: vertical slice (window + wgpu + FPS camera + physics) — done
- M2: content pipeline + editor — done
- M3: netcode (TCP, prediction, reconciliation) — done
- M1.5: polish (lighting, textures, gamepad, tests) — done
- M5: accounts + DB (SQLite) — done
- M6: launcher + delivery — done
- M7: release hygiene (crash reporting, installer, CI release) — done

## Post-patch checklist (for agents)

After any code change, run:
- `cargo fmt --all`
- `cargo build --workspace`
- `cargo test --workspace`
- `cargo clippy --workspace --all-targets -- -D warnings`

For a release update (only when explicitly requested by user):
- `cargo build --workspace --release`
- update launcher version/manifest
- publish via `gh release` (requires user approval)

- `ELECTRON_RUN_AS_NODE` must be unset to run the Electron launcher.
- Vulkan SDK recommended for Vulkan-primary development; wgpu falls back to DX12 without it.
- Repository: https://github.com/Otakyooo/zona_engine

## Local workspace (WorkBuddy)

- Root: `C:\Users\russi\WorkBuddy AI\Zona_games` (path contains a space — quote it in shell commands).
- Migrated from `E:\zona_engine` on 2026-10-31 via local clone; the old copy is kept as a backup.
- Durable agent notes live in `.workbuddy-ai/memory/` (tracked in git).
- Old release artifacts and closed plans are archived under `.workbuddy-ai/archive/` and `.kilo/plans/archive/`.
- Do not set a global `CARGO_TARGET_DIR`: CI expects the relative `target/` directory.
- Product version has one source: `[workspace.package] version` in the root `Cargo.toml`. `node ci/check-versions.js` fails when `launcher/package.json`, `manifest.stub.json` or the release tag disagree; regenerate the offline manifest with `npm run stub` in `launcher/` after replacing the payload.
- Verify after every change: `cargo fmt --all -- --check`, `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
- Launcher check without a GUI session: `node --check launcher/main.js`, then `npm start` with `ELECTRON_RUN_AS_NODE` unset.
- `launcher/manifest.stub.json` points at the local `launcher/game/zn-client.exe` (offline fallback); keep its path and sha256 in sync with the payload.

## Memory (for agents)

Project memory lives in `.workbuddy-ai/memory/` and is tracked in git. It is the only durable context between sessions — read it before starting work, write to it before finishing.

- Start of a task: read `MEMORY.md` (index) and `OPEN.md` (priorities).
- An architectural decision that is not obvious from the code goes to `DECISIONS.md` (numbered, with context and consequences).
- A trap that cost time goes to `PITFALLS.md` as symptom → cause → fix. The cause is mandatory: without it the rule becomes superstition.
- End of a task: append to `journal/<YYYY-MM-DD>.md` — what changed, why, which checks ran, and concrete numbers (tests, FPS, sizes). Link commit hashes.
- `MEMORY.md` stays an index of at most ~150 lines. When it grows, push detail into `PITFALLS.md`, `DECISIONS.md` or the journal — do not let it become a wall of text.
- Do not duplicate what the code or `README.md` already says. Memory records the non-obvious.

## Textures

- Materials live in `assets/textures/<name>.png` — 512×512, seamless, generated (concrete, metal, rust, dirt, grass, wood).
- Regenerate with `cargo run -p zn-tools --bin zn-gentex`; output is deterministic (fixed per-material seeds).
- Scene nodes select a material with `texture: Some("concrete")` and set texel density with `uv_scale` (metres per tile, default 1.0).
- Build boxes with `zn_render::geom::make_box(&Transform, color, tile_m)` — 24 vertices, per-face normals, UVs measured in metres. Do not bake transforms into vertex data; pass the transform in.
- Textures are loaded once in `resumed()` into `App::textures`; never create a texture inside the per-entity mesh rebuild path.

## Server rules

- The simulation (`zn-sim`) owns world state: damage, death, loot drops, extraction timers and XP. The server owns persistence — it only reads/writes `profiles`, `inventory_items` and `sessions`.
- Death loot is created in `SimWorld::tick` before the respawn, with ids from `next_loot_id`. Never spawn loot from a `GameEvent` handler — by then the player has already respawned.
- Ids are namespaced via `zn_sim::ids`: items `1..=999`, loot `1_000_000+`, artifacts `2_000_000+`, creatures `3_000_000+`. Classify, don't guess; client and server must agree.
- Frames are capped at `zn_net::MAX_FRAME_BYTES` (64 KiB). Raise the constant, never allocate from the wire length directly.
- Sessions live in the `sessions` table; `Connect` validates the token with an expiry check, disconnect deletes it, and a sweep drops expired rows. There is no in-memory token cache.
- Login and register attempts are limited per peer IP (5 per 5 minutes, reset on success).

