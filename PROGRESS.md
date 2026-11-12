# PROGRESS

Living status file. A future session should be able to resume from this alone.

## How to run

```bash
# Toolchain: Rust stable 1.99 (edition 2024). Install components once:
rustup component add clippy rustfmt

# Play the game (opens a window).
cargo run -p deepcore-client
cargo run -p deepcore-client -- --seed 1234     # different world

# Headless authoritative server (fixed 30 Hz).
cargo run -p deepcore-server

# Regenerate the block texture atlas (no art assets needed).
cargo run -p atlas_gen            # writes assets/atlas.png

# Pacing report: raw ore needed per Era, machine recipe coverage.
cargo run -p deepcore-shared --bin balance_report

# Full quality gate: fmt + clippy -D warnings + tests.
./scripts/check.sh
```

Headless rendering (no GPU in the build sandbox):

```bash
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json XDG_RUNTIME_DIR=/tmp/xdg \
  ./target/debug/client
```

## Controls

| Input | Action |
|---|---|
| Click window | capture mouse |
| WASD / Space / Shift / Ctrl | move / jump / sprint / crouch |
| Mouse | look |
| Left mouse (hold) | mine |
| Right mouse | place held block |
| 1–9 | select hotbar slot |
| Escape | release mouse |

## Pinned dependency set

Verified against crates.io on **2026-12-06**. All Bevy-facing crates agree on
`bevy ^0.20`:

| Crate | Version | Notes |
|---|---|---|
| `bevy` | `=0.20.1` | `default-features = false` + explicit features per crate |
| `bevy_replicon` | `=0.45.5` | depends on `bevy ^0.20` (not yet wired up — M2) |
| `bevy_replicon_renet` | `=0.21.0` | depends on `bevy ^0.20`, `bevy_renet ^6.0` |
| `bevy_egui` | `=0.43.1` | depends on `bevy_* ^0.20.0` (not yet wired up — M3) |

Others: `ron 0.12.3`, `postcard 1.1.3`, `noise 0.9.0`, `rand 0.9.2`,
`rand_chacha 0.10.0`, `zstd 0.14.3`, `tracing 0.1.41`, `thiserror 2.0.17`,
`proptest 1.9.0` (dev).

## API notes learned the hard way (do not regress)

### Bevy 0.20
- `AmbientLight` is a **component**; the global is the **`GlobalAmbientLight` resource**.
- Lights use **`shadow_maps_enabled`**, not `shadows_enabled`.
- Rendering: `Mesh3d(handle)` + `MeshMaterial3d(handle)`, `Camera3d::default()`.
- `MinimalPlugins` has **no logger** — the server adds `bevy::log::LogPlugin`.
- UI is **not** in `bevy`'s minimal feature set: the client enables `bevy_ui`,
  `bevy_ui_render`, `bevy_text`, `default_font`.
- `TextFont::font_size` is a **`FontSize` enum** (`FontSize::Px(20.0)`), not f32.
- Shaders: only `.wesl` files get import processing (`import a::b::{c};`).
  `.wgsl` files are passed through raw, so `#import` fails. **We use no custom
  shader** — chunks use `StandardMaterial` with the atlas as `base_color_texture`,
  which needs no shader plumbing and gets real lighting and shadows.
- Bevy looks for `assets/` relative to the executable; the client sets
  `AssetPlugin.file_path` to the workspace `assets/` so `cargo run` works anywhere.

### RON 0.12
- `Option` fields need an explicit `Some(..)`. Because that is noisy for content
  authors, optional *string* fields are modelled as `String` with
  `#[serde(default)]` (empty = absent). Tuple fields stay plain tuples.
- **Enum variants with fields must use named fields**, not tuples:
  `Flooding(pocket_chance: 0.002, max_volume: 512)`. `Flooding((0.002, 512))`
  is a parse error.
- Plain tuple *struct fields* are fine: `radiation: Some((4.0, 0.5))`.

### Edition 2024
- `gen` is a **reserved keyword** — cannot be used as a variable or function name.

### noise 0.9 / rand_chacha 0.10
- `noise::NoiseFn` must be imported for `.get([x, y, z])`.
- `ChaCha8Rng` seeds from a **`[u8; 32]`**, not `u64`; `seed_from_u64` is gone.

## Milestone status

### M0 — Workspace and skeleton — DONE
Workspace of 4 crates, pinned deps, profiles, `rustfmt.toml`, `clippy.toml`,
`scripts/check.sh`. Server runs headless at a verified steady **30.0 Hz** with a
heartbeat log. Client opens a window and renders a lit scene (verified by
screenshot under Xvfb + lavapipe).

**Gate status: `./scripts/check.sh` passes end to end** (fmt check → clippy
`--all-targets -D warnings` → tests), verified after touching source files to
force a rebuild.

> **Process note (do not repeat).** This gate was once reported green while it
> was actually failing: clippy had been run as `--workspace` without
> `--all-targets`, which skips integration tests, so an unused variable in
> `crates/shared/tests/mining_loop.rs` went unnoticed. `check.sh` now prints the
> exact command it runs, and the only acceptable evidence that a milestone is
> green is running `./scripts/check.sh` itself to completion.

### M1 — Voxel world — DONE (single process)

Built:
- **Voxel storage** (`shared/src/voxel/`): 32³ chunks, palette-compressed to `u8`
  indices, `BlockPos`/`ChunkPos` with Euclidean division so negative coordinates
  floor correctly, packed serialisation with strict validation of corrupt input.
- **Content system** (`shared/src/content/`): RON definitions, fail-fast loader
  with a full validator, numeric registries, stable content hash for the M2
  handshake.
- **Content**: 45 blocks, 174 items, 119 recipes, 43 machines, 30 tech nodes,
  6 contiguous layers (0–512).
- **Worldgen** (`shared/src/worldgen/`): six layers, noise-carved caves and
  caverns, per-ore weighted veins, undulating surface with a flat camp plateau,
  bedrock floor and walls, pure sky above the surface. Deterministic per seed.
- **Movement** (`shared/src/movement/`): pure fixed-step player controller and
  continuous swept AABB-vs-voxel collision with wall sliding and no tunnelling.
- **Mining** (`shared/src/mining/`): break-time formula, tool-tier gating,
  area-tool offsets, and an Amanatides–Woo voxel raycast.
- **Meshing** (`shared/src/mesh/`): face culling against neighbours, shade baked
  into vertex colour.
- **Atlas** (`shared/src/atlas.rs`): procedural 16×16 tiles from block colour +
  pattern, packed into one image, hand-written PNG encoder (no image crate).
- **Client**: async chunk meshing on the task pool, chunk streaming with
  distance-prioritised meshing, first-person controller, mining with progress,
  block placing, block highlight, HUD (crosshair that turns red on an
  unmineable block, hotbar, depth + layer meter, mining refusal messages).

Measured against the prompt's budgets:

| Metric | Budget | Measured | Status |
|---|---|---|---|
| Chunk generation | < 5 ms | **1.3 ms** (was 5.9 ms) | met |
| Chunk meshing (32³) | < 4 ms | **0.55 ms** | met |
| Solid chunk meshed to shell | — | 6144 faces, not 98k | — |
| **Client FPS, view radius 6** | **≥ 60 FPS on a mid-range GPU** | **1–4 FPS** | **NOT VERIFIED — see below** |

### Frame-rate instrumentation

The client now measures itself rather than having performance asserted about it.
`crates/client/src/diagnostics.rs` installs Bevy's `FrameTimeDiagnosticsPlugin`,
accumulates a rolling window, shows a live `FPS / ms / chunks / view radius`
readout top-right, and logs a summary every 5 s:

```
INFO client::diagnostics: client performance fps=1.0 avg_ms=963.78
  worst_ms=1011.36 best_ms=908.87 frames=20 chunks_loaded=440
  view_radius_chunks=6
```

**The 60 FPS target is unverified in this environment.** The build sandbox has no
GPU; the only available adapter is **llvmpipe** (Mesa software rasterisation,
reported by Bevy as *"a driver that only supports software rendering"*). Software
rendering of ~15k triangles per chunk at radius 6 gives the 1–4 FPS above, which
says nothing about real hardware. Frame rate must be re-measured on a machine
with a GPU before this criterion can be called met. The measurement harness is in
place; only the hardware is missing.

Tests: **85** passing, including chunk/palette round-trips, negative-coordinate
chunk maths, worldgen determinism, ore presence per layer, uranium-only-deep,
movement determinism, no-tunnelling, sliding, break-time maths, tier gating,
area tools, raycast accuracy, atlas determinism, and a mining-loop integration
test that digs layer 1 and is blocked by granite with a steel pickaxe.

**Bugs found and fixed during M1 (worth remembering):**
1. *Solid sky chunks.* Chunks at or above `surface_y` were generated as solid
   bedrock, burying the surface under an invisible shell and hiding the whole
   world from a player standing on it. Fixed; regression test added.
2. *Fill-block shares.* Fill shares were normalised, so layers with shares
   summing below 1.0 contained no base rock at all.
3. *Diagonal corner slip.* Sweeping X then Z sequentially let a diagonal move
   pass a corner. Each horizontal axis is now swept from the start position.
4. *Discrete collision.* Only testing the destination let a fast box pass
   through a thin wall. The sweep now samples the swept volume.
5. *Sphere chunk radius.* A spherical load radius left the diagonally adjacent
   chunk unloaded, so the ground under the player never meshed.
6. *Ore density.* The first vein algorithm produced ~0.0% copper. Rebalanced to
   0.5–2% per layer while cutting generation cost 4×.

Deferred / not yet built:
- `lobby` crate (M10). Not a workspace member yet.
- Chunk unloading is implemented but only runs on movement; no remesh pass yet
  for chunks whose neighbour arrives later (boundary faces are drawn, which is
  visually correct, just slightly more geometry).
- Emissive ore glow in-world (data exists; the visual overlay is M9 polish).
- `dynamic_linking` dev feature intentionally off: it conflicts with
  `default-features = false` plumbing and adds no value yet. Revisit in M9.

Known issues:
- Client needs a Vulkan/GL backend; in the sandbox it uses software rendering
  (llvmpipe), which is slow but correct.
- Chunk streaming is single-player only so far; the server does not yet send
  chunks (M2).

Also note: `PROMPT.md` at the repo root holds the full specification, so a session
resuming from this file has the requirements available in-repo.

### M2 — Multiplayer foundation — PARTIAL

Everything below was **verified by running it**, with logs quoted.

Built:
- **Protocol** (`shared/src/protocol.rs`): `ClientHello`/`ServerHello`, intents
  (`MoveInput`, `StartMining`, `StopMining`, `PlaceBlock`), server messages
  (batched `BlockChangeBatch`, `ChunkBatch`, `ChunkUnload`, `MoveCorrection`,
  `MiningRefused`, `ChatMessage`), replicated components. `PROTOCOL_VERSION = 2`.
- **Handshake** with protocol-version and content-hash checks, producing exactly
  `"Content mismatch"` when content differs.
- **Server authority**: `ServerWorld` chunk store, avatar spawning, authoritative
  movement using the *same* `step_player` the client predicts with, mining timers
  in `FixedUpdate`.
- **Intent validation** (`server/src/sim/validate.rs`): reach (5 / 6 with drill),
  line of sight by voxel raycast, tool tier, slot contents, place-target checks.
  20 tests, including unloaded-chunk and out-of-range-slot safety.
- **Chunk streaming** with interest management (6 chunks horizontal, 4 vertical),
  nearest-first, unload messages, rate-limited per tick.
- **Movement prediction and reconciliation**: client buffers unacked inputs,
  server echoes `last_processed_seq`, client rewinds/replays, corrections under
  0.3 blocks blend out linearly over 100 ms, larger ones snap.
- **Main menu**: Host game, Join by IP, Play alone, name and address fields,
  error display.
- **`--simulate-latency` / `--simulate-loss`** on both binaries
  (`shared/src/net_sim.rs`, 7 tests) — renet 2.0 has no built-in simulation.
- **`server --bots N`** headless bots going through real intent validation.
- **`--headless --join ADDR --run-secs N`** on the client, which is how the
  automated multiplayer tests run without a GPU.

**Verified by running:**

| Check | Result |
|---|---|
| Server + 2 clients, handshake | both `client accepted`, avatars spawned |
| Chunk streaming | 40–62 chunks received per client, both reach `world loaded` |
| Connection stability | both clients stay connected for the full run, no disconnects |
| Content mismatch | server logs `refusing client reason=Content mismatch`; client logs `server refused the connection reason=Content mismatch` |
| 8-bot soak, 10 minutes | **18,000 ticks, 596/599 heartbeats at exactly 30.0 Hz, 0 panics**, 1062 blocks mined, bots dug from y=512 to y=480 |

**Remote player rendering (added after the first pass):**
- `client/src/render/remote.rs` draws a capsule body, head and floating name for
  every replicated player, interpolated **100 ms in the past** between the two
  most recent snapshots (7 tests: interpolation midpoint, clamping before/after
  history, shortest-path yaw, exact decay window).
- Replication rules were missing entirely at first — `Replicated` alone does
  nothing. Both sides now call `.replicate::<PlayerTransform>()` and friends.
- A `VisibilityFilter` (`AvatarOwner`) hides each player's own avatar from
  themselves, so you do not see a duplicate of your own body.
- **Verified by running:** with two headless clients, Alice logs
  `remote players visible count=1 players=["Bob"]` and Bob logs
  `players=["Alice"]` — each sees exactly the other, never themselves.

**Not done / not verified (be honest about these):**
- **A screenshot of one player seeing another is still missing.** Remote body
  rendering is implemented and its interpolation is unit-tested, and replication
  is verified headlessly, but the rendering client in this sandbox (llvmpipe,
  ~2 FPS at the default view radius) is dropped by netcode's connection timeout
  before a second player joins. At `--view-radius 1` it reaches ~15 FPS but still
  times out after ~30 s. This is a software-rendering limitation, not a logic
  bug, but it means the on-screen two-player view is unproven here.
- **150 ms latency "feels responsive"** is not verified. The simulation and its
  tests exist; "feels responsive" needs a human on real hardware.
- Client-side prediction stores unacked inputs and replays after a snap, but the
  replay path is not exercised by an automated test.
- Placing blocks is validated server-side, but the client hotbar is still
  single-player-only (inventory is M3).

**Known issue worth fixing next:** a client whose frame rate is very low can be
dropped by netcode's connection timeout, because the client sends at most one
update per frame. A real fix is to run the transport update on a timer
independent of rendering; noted for M9.

Next: close the two-player *visual* proof on hardware that can render it, then
**M3 — content, inventory, crafting (Era 0)**.

Next: close the M2 rendering gap (remote player bodies + interpolation), then
**M3 — content, inventory, crafting (Era 0)**.
