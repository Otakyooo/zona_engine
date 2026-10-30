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
- M4: Zona gameplay (anomalies, artifacts, loot, extraction, progression) — planned

## Notes

- `ELECTRON_RUN_AS_NODE` must be unset to run the Electron launcher.
- Vulkan SDK recommended for Vulkan-primary development; wgpu falls back to DX12 without it.
- Database defaults to SQLite (`zona.db`); migrations run automatically on server start.
