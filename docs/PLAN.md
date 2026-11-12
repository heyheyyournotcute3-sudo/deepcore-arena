# Deepcore — Plan

## What the game is

Deepcore is a co-op, first-person, voxel mining game. A group of up to 16 players
spawns at a surface **Base** above one large, persistent, procedurally generated
mine. Everyone starts with a steel pickaxe and nothing else. The loop is:

> dig deeper → haul ore home → build a structure → it unlocks a better tool or a
> refining step → dig deeper.

Six **Eras** gate each other by materials. The world is 1024 × 1024 × 512 blocks,
made of six depth layers. Each layer is harder to dig, richer, and more dangerous,
and needs a higher tool tier. The end of the ladder is **nuclear power**: uranium
in the deepest layer feeds a refining chain (crush → leach → precipitate →
convert → enrich → fabricate) that produces fuel rods, a reactor, and the
**Nuclear Drill** — a 3×3, 12× speed tool that bores tunnels in seconds.

Every step is a structure the players physically place and operate in the world.

## Architecture rule

> **`shared` holds every rule. `server` decides. `client` shows.**

Four crates in one Cargo workspace:

| Crate | Responsibility |
|---|---|
| `shared` | All game rules: content registries, voxel storage, worldgen, movement/collision, inventory, crafting, power/steam solving, hazards, protocol types. No rendering, no sockets. |
| `server` | Headless authoritative simulation. Owns the world, validates every intent, streams chunks, persists. |
| `client` | Windowed game. Prediction for its own movement only; renders chunks, HUD, egui screens. |
| `lobby` | (M10) axum + sqlx accounts, tokens, server browser. |

The single most important consequence: **movement, break-time, recipe matching and
power maths are the same functions on both sides.** The client can predict a
crack animation because it calls the exact code the server will call.

## Module layout inside `shared`

```text
shared/src/
  content/    RON loaders -> numeric registries (BlockId, ItemId, ...), validator
  voxel/      Chunk (palette-compressed), ChunkPos/BlockPos maths
  worldgen/   layers, caves, ore veins — deterministic from seed + chunk pos
  movement/   AABB-vs-voxel collision, player controller (pure, fixed step)
  inventory/  stacks, slots, transfer/stacking rules
  crafting/   recipe matching, machine processing state machine
  power/      steam and electric network solving, brownout scaling
  hazards/    heat, gas, water, radiation, cave-ins
  protocol/   replicated components, intents, events, channels, PROTOCOL_VERSION
```

Files stay small and named by purpose (target < 400 lines), one Bevy plugin per
feature: `MiningPlugin`, `InventoryPlugin`, `PowerGridPlugin`, …

## Key technical decisions

- **Bevy 0.20.1 on both sides.** Server uses `MinimalPlugins`; client uses
  `DefaultPlugins`. Same components and systems everywhere.
- **Versions pinned exactly** in the workspace `Cargo.toml`. Verified on
  crates.io on 2026-12-06: `bevy_replicon 0.45.5`, `bevy_replicon_renet 0.21.0`
  and `bevy_egui 0.43.1` all depend on `bevy ^0.20`.
- **Chunks are 32³.** Palette-compressed `u16` block ids. Unmodified chunks are
  never saved — they regenerate from the seed.
- **Own voxel meshing** (greedy, on `AsyncComputeTaskPool`), own AABB physics.
  No physics engine: determinism matters more than realism here.
- **Data over code.** Blocks, items, recipes, machines, tech tree, layers and
  hazards live in RON under `assets/data/`. Adding an ore never touches Rust.
  Content is validated at load and fails fast with a readable message.
- **Server tick** is a fixed 30 Hz in `FixedUpdate`. Client renders uncapped and
  predicts only its own movement.
- **No external art.** A texture atlas is generated procedurally from each
  block's `color` + `pattern`, so a fresh clone runs with zero downloads.

## Milestones

The order below is deliberate: **networking lands at M2**, because retrofitting
multiplayer onto a single-player game is the classic way this kind of project
fails. Each milestone must end compiling, with green tests and an updated
`PROGRESS.md`.

- **M0 — Workspace and skeleton.** Four crates, pinned versions, profiles,
  `rustfmt.toml`, clippy config, `scripts/check.sh`. Client opens a window with a
  camera and a lit cube; server runs headless at 30 Hz and logs its tick rate.
- **M1 — Voxel world (single process).** Palette chunks, position maths, worldgen
  for all six layers with caves and veins (deterministic), procedural atlas,
  greedy async meshing, first-person controller with AABB collision, block
  targeting, mining with hardness/tier, placing, hotbar, depth meter.
- **M2 — Multiplayer foundation.** Handshake (protocol version + content hash),
  chunk streaming with interest management, replicated players with
  interpolation, movement prediction/reconciliation, mining and placing through
  validated intents, main menu (Host / Join by IP), latency + loss simulation,
  bot clients.
- **M3 — Content, inventory, crafting (Era 0).** RON loaders, registries,
  validator, `balance_report`. Backpack + hotbar in egui, pickups, death crate,
  respawn. Workbench hand-crafting, trees/timber, ladders, supports, oil lamps.
- **M4 — Machines, research, Era 1.** Block entities, multi-block placement with
  ghost preview, machine UI with idle reasons, fuel burning, Brick Kiln, Stone
  Furnace, Anvil, Blast Furnace, Research Desk + Tech Tree graph UI.
- **M5 — Era 2: steam and logistics.** Pipe networks and steam solver, Hand Pump,
  Boiler, Steam Engine, Crusher, Ore Washer, conveyors/item pipes, rails,
  minecarts, Mine Lift, Steam Drill, flooding + oxygen.
- **M6 — Era 3: electricity.** Cable networks, electric solver with brownouts and
  batteries, network overlay, Turbine/Generator/Battery Bank/Arc Furnace/
  Electrolysis Cell/Circuit Assembler/Charging Station, upgrade modules, Ore
  Scanner, Vent Fan, Electric Pump, gas hazard.
- **M7 — Era 4: chemistry and deep metallurgy.** Chemical Plant, Alloy Furnace,
  Machine Shop, Rare-Earth Separator, Decon Shower; heat and cave-ins; Cooling
  Suit, Respirator, Hazmat Suit, Geiger Counter; Tungsten-Carbide Drill.
- **M8 — Era 5: nuclear.** Leaching Tank, Precipitator, Conversion Plant,
  Centrifuge Cascade (scales with length), Fuel Fabricator, control rods, the
  multi-block Reactor with console/scram/overheat, Cooling Pool, Waste Vault,
  Nuclear Drill (3×3), radiation.
- **M9 — Persistence and polish.** Region files + SQLite, autosave, crash-safe
  writes; audio, particles, animations; menus, settings, objective list;
  balance pass.
- **M10 — Full stack.** `lobby` on axum + sqlx, argon2 accounts, session tokens,
  rate limiting, server registration/heartbeat, server browser, Dockerfiles.

## Quality bar I am holding myself to

- `cargo clippy --workspace -- -D warnings` and `cargo fmt` clean.
- No `unwrap()` on network input, file input or content lookups.
- Every public type and plugin in `shared` has a doc comment.
- `PROTOCOL_VERSION` bumped whenever a replicated shape changes.
- Tests: shared-logic unit tests, headless Bevy `App` tests, one progression
  integration test per Era (the nuclear one matters most), and network
  convergence tests.
