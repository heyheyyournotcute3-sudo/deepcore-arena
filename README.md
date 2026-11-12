# Deepcore

A co-op, first-person, 3D voxel mining game written entirely in Rust with Bevy.

Players spawn at a surface camp above a huge procedural mine, start with a steel
pickaxe and nothing else, and dig down through six layers. Ore comes home to the
Base, where machines turn it into better tools, and better tools open deeper,
harder, more dangerous rock. The end of the ladder is **nuclear power**: uranium
from the deepest layer is crushed, leached, precipitated, converted, enriched and
fabricated into fuel rods and core cells that power a reactor and a 3×3 Nuclear
Drill.

Every step is a structure you physically build and feed.

## Status

Under active construction, milestone by milestone. See [`PROGRESS.md`](PROGRESS.md)
for exactly what works today and [`docs/PLAN.md`](docs/PLAN.md) for the design and
milestone list.

- **M0 Workspace and skeleton** — done
- **M1 Voxel world (single process)** — done: dig, place, stream chunks, six layers
- **M2 Multiplayer foundation** — next

## Build and run

Requires a stable Rust toolchain (edition 2024) and a Vulkan-capable GPU.

```bash
rustup component add clippy rustfmt     # once

cargo run -p deepcore-client            # play
cargo run -p deepcore-server            # headless 30 Hz server
./scripts/check.sh                      # fmt + clippy -D warnings + tests
```

The game ships with **no art assets**: block textures are generated procedurally
from each block's colour and pattern into a texture atlas at startup, so a fresh
clone runs with zero downloads.

### Other tools

```bash
cargo run -p atlas_gen                          # write assets/atlas.png for inspection
cargo run -p deepcore-shared --bin balance_report   # pacing report: ore needed per Era
```

## Controls

| Input | Action |
|---|---|
| Click the window | capture the mouse |
| WASD | move |
| Space / Shift / Ctrl | jump / sprint / crouch |
| Mouse | look |
| Left mouse (hold) | mine the targeted block |
| Right mouse | place the held block |
| 1–9 | select hotbar slot |
| Escape | release the mouse |

The crosshair turns red when the targeted block needs a higher tool tier, and the
HUD says which tier it needs.

## How the code is organised

> **`shared` holds every rule. `server` decides. `client` shows.**

```text
crates/
  shared/   all game logic: content, voxels, worldgen, movement, mining, meshing,
            inventory, crafting, power, hazards, protocol. No rendering, no sockets.
  server/   headless authoritative simulation (M2 onward).
  client/   windowed game: rendering, input, HUD, menus.
tools/
  atlas_gen/  generates the block texture atlas.
assets/
  data/     blocks.ron, items.ron, recipes.ron, machines.ron, tech_tree.ron, layers.ron
```

Because both the client and the server compile the same `shared` crate, movement,
break times, recipe matching and power maths are literally the same functions on
both sides. That is what makes prediction and reconciliation tractable.

## Adding content

All game content lives in RON files under `assets/data/` and is validated at
startup; a typo fails fast with a readable message rather than corrupting a
session. Adding an ore or a recipe never requires touching Rust.

```ron
// assets/data/blocks.ron — a new ore
(
    id: "cobaltite",
    hardness: 4.2,
    min_tier: 3,
    drops: [("raw_cobalt", 1)],
    color: (0.30, 0.34, 0.52),
    pattern: OreFleck((0.35, 0.45, 0.95)),
    layer_hint: Some(4),
),
```

```ron
// assets/data/recipes.ron — smelt it
(
    id: "smelt_cobalt", machine: "arc_furnace", time: 6.0,
    inputs: [("raw_cobalt", 2)], outputs: [("cobalt_ingot", 1)],
    tech: "electric_power", power_kw: 60.0,
),
```

Then add the item to `items.ron`, mention the block in a layer's `ores` list in
`layers.ron`, and run:

```bash
cargo run -p deepcore-shared --bin balance_report
cargo test -p deepcore-shared --test content
```

The validator rejects unknown references, recipes with no machine, tech nodes with
missing parents or cycles, layers that are not contiguous, and raw items that
nothing drops.

## Testing

```bash
cargo test --workspace
```

The suite covers chunk indexing and palette round-trips, worldgen determinism,
per-layer ore presence, movement determinism and collision (sliding, no
tunnelling), break-time maths and tier gating, voxel raycasting, atlas
determinism, and an end-to-end mining loop that digs layer 1 and is correctly
blocked by granite with a steel pickaxe.

## Licence

MIT OR Apache-2.0.
