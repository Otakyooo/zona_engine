# ZONA Full Refactor Plan

Goal: resolve critical netcode bugs, security gaps, persistence, simulation completeness, rendering/performance, and test coverage identified in the project audit.

## Phase 0 — Baseline
- Run `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --all-targets -- -D warnings` to capture current state.
- Add `launcher/dist_check*` to `.gitignore` and remove tracked artifacts.

## Phase 1 — Netcode identity & persistence (critical)
Files: `crates/zn-server/src/main.rs`, `crates/zn-net/src/lib.rs`, `crates/game/zn-sim/src/lib.rs`, migrations.

Tasks:
- Unify player identity across TCP/UDP:
  - Add `token_to_player: HashMap<String,u64>` and `player_to_user: HashMap<u64,i64>`.
  - TCP `Connect`: validate token; create player once (`next_id`), store mappings, `world.add_player`, send `Welcome{id}`.
  - UDP `Connect`: resolve `id` from `token_to_player`; bind `addr→id`; do not create duplicate player.
  - Disconnect cleanup removes all mappings.
- Extend `PlayerProfile` with `position: Vec3` (serde). Add migration `0004_profiles_position.sql`.
- Implement `save_profile(pool, uid, Option<&Player>)` to upsert level/xp/currency/position; add `save_inventory` upsert.
- Call save on disconnect and periodic save timer (already exists).
- Apply loaded profile/inventory/position in TCP `Connect`.
- Reconciliation: client interpolates server position (lerp) instead of hard snap when delta <= 2.0; snap only on larger divergence.

## Phase 2 — Security
Files: `crates/zn-server/src/main.rs`, `crates/zn-net/src/lib.rs`, `crates/zn-client/src/main.rs`.

Tasks:
- Split auth: `ClientPacket::Register` creates user; `Login` verifies only (remove auto-create).
- Password validation (min length 8).
- Sessions: store tokens in DB (`sessions` table, migration `0005_sessions.sql`) with expiry; periodic cleanup of expired.
- Rate limiting: per-IP failed login counter with lockout.
- Remove default `"guest"` password in client; require `--username/--password` or env vars; error if missing.
- (Optional, out-of-scope if too large) TLS for TCP via tokio-rustls.

## Phase 3 — Simulation completeness
Files: `crates/game/zn-sim/src/lib.rs`, `crates/zn-server/src/main.rs`.

Tasks:
- Add `inventory: Inventory` to `Player` (serde).
- Pickup: add item to player inventory, remove world entity.
- Consume: validate inventory, decrement quantity, apply effect by item_id (1=medkit +30 HP, 2=bandage +15 HP).
- Artifact pickup: apply effect (regen +20 HP) and store in inventory.
- Extraction: on timer complete, emit `ExtractionSuccess` with player inventory items; save inventory to DB; clear inventory; award XP.
- Death: drop inventory as `LootItem`s at player position; clear inventory; respawn at origin with full stats; save.
- Creature kill XP already; ensure removal and event emission.

## Phase 4 — Rendering & performance
Files: `crates/zn-client/src/main.rs`, `crates/engine/zn-render/src/lib.rs`, `crates/engine/zn-assets/src/lib.rs`.

Tasks:
- Mesh cache: reuse base meshes; build dynamic entities with instancing (instance buffer of model matrices) instead of recreating meshes per frame.
- Frustum/distance culling for remote entities.
- Wire `select_lod` into asset loading; generate and use mipmaps in renderer (`create_texture_from_loaded`).
- glTF: transform normals (inverse-transpose), support multiple primitives/meshes.
- Audio: implement `AudioManager` with kira; play shot/pickup sounds.
- (Large) Physics: integrate `PhysicsWorld` on server for movement/collisions replacing simplified logic.

## Phase 5 — Quality & tests
Files: all crates.

Tasks:
- Add unit tests: `SimWorld::tick`, damage, extraction; net encode/decode; scene roundtrip (exists).
- Remove dead code or gate behind features (ChunkManager, ReliableChannel, LOD helpers) or implement.
- Fix clippy warnings; replace `unwrap()` in server addr parse and client with proper errors.
- Structured logging to file (crash.log already; extend).

## Risks
- Identity refactor touches core net flow; requires careful migration of existing clients.
- Instancing refactor is large; can be split into separate PR.
- Physics integration changes movement feel; keep simplified as fallback behind feature flag.

## Validation
- `cargo build --workspace`
- `cargo test --workspace`
- `cargo clippy --all-targets -- -D warnings`
- Manual: two clients connect, see each other, pickup/consume, extract, die & respawn; profile persists after restart.
- Security: invalid login rejected, rate limit triggers.

## Open questions
- TLS scope: include now or defer? (Recommend defer to separate plan.)
- Should instancing be mandatory or optional phase? (Recommend optional, separate PR.)
