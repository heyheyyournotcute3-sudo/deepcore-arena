# Deepcore: Build Prompt for a Rust Multiplayer Mining Game

> This file is the project specification, committed so that a session resuming
> from `PROGRESS.md` has the full requirements available in the repo.
> `PROGRESS.md` records what has actually been built and measured against it.

## 1. Role, mission and operating rules

You are a senior Rust game engineer and technical game designer. Your job is to
build **Deepcore**, a complete, playable, online co-op 3D voxel mining game, from
an empty folder, entirely in Rust. You own the code, the architecture, the game
balance and the tests. Work autonomously. Ask the human only when a decision is
irreversible or truly ambiguous.

### The game in one paragraph

Players spawn at a surface camp above a huge procedurally generated mine.
Everyone starts with a **steel pickaxe** and nothing else. They dig down through
block layers, collect ores, and haul them back to build machines: furnaces,
crushers, boilers, generators, refineries. Each machine unlocks better tools and
new materials, and those open deeper, harder, more dangerous layers. The final
goal is **nuclear power**: find uranium in the deepest layer, then build the full
refining chain (crush, leach, convert, enrich, fabricate fuel). That chain powers
a **Nuclear Drill** that mines whole tunnels in seconds, plus a reactor that
powers the base. Every step on that ladder must be a structure the players
physically build and operate in the world.

### How you work

1. **Read the whole prompt before writing any code.** Then write `docs/PLAN.md`:
   your understanding of the game, the crate layout, and the milestone list from
   Section 8, in your own words.
2. **Keep a `PROGRESS.md`.** After every milestone, record what was built, what
   was deferred, known bugs, and the exact commands to run the server and
   client. A future session, possibly a different model, must be able to continue
   from this file alone.
3. **Work milestone by milestone, in order.** Do not start a milestone until the
   previous one meets its acceptance criteria. The game must compile and run at
   the end of every milestone.
4. **Compile constantly.** Run `cargo check --workspace` after every meaningful
   change. Run `cargo clippy --workspace -- -D warnings` and
   `cargo test --workspace` before closing a milestone. Fix errors immediately
   and never leave the tree broken.
5. **Verify library APIs, never guess them.** The Bevy ecosystem breaks its API
   every release. Before using a crate, check its newest version on crates.io and
   read its docs.rs page and examples for that exact version. If your memory of
   an API disagrees with the docs, the docs win. Pin exact versions in the
   workspace `Cargo.toml` once chosen.
6. **No stubs, no fake features.** Do not write `todo!()`, empty systems, or
   "implement later" comments in code you call finished. If something must be
   deferred, leave it out entirely and list it in `PROGRESS.md`.
7. **Data over code.** Blocks, items, recipes, machines and the tech tree live in
   RON data files (Section 6). Adding a new ore or recipe must never require
   touching Rust code.
8. **The server is the source of truth.** Every gameplay rule runs on the server:
   mining, crafting, inventories, machines, power and hazards. The client
   predicts only its own movement and sends intents.
9. **Keep files small and named by purpose.** Aim for under 400 lines per file
   and one Bevy plugin per feature (`MiningPlugin`, `InventoryPlugin`,
   `PowerGridPlugin`, ...).
10. **No external art required.** Generate placeholder textures procedurally
    (Section 3) so the game runs from a fresh clone with zero downloaded assets.

### What not to do

- Do not use a different language, a scripting layer, or a non-Rust engine.
- Do not use real-world chemistry or engineering detail beyond what is written
  here. Refining is a game abstraction: machines take items in, wait, and put
  items out.
- Do not add features outside this prompt (combat, PvP, monsters,
  microtransactions) unless a milestone asks for them.
- Do not rewrite working systems for style. Refactor only when a milestone needs
  it.

---

## 2. Tech stack

Use **Bevy** for both client and server. One engine and one ECS on both sides
means the same components, systems and data types run everywhere. That is the
single biggest simplification for a multiplayer game. Bevy is code-first with no
editor, which suits an AI agent: everything is plain Rust you can read, write and
test.

At the time of writing, the latest stable Bevy is 0.19.x, and `bevy_replicon`
0.44.x targets it. Treat these as a starting point. Check crates.io, pick the
newest Bevy release that **all** the crates below support, and pin that set.

| Concern | Choice | Why |
|---|---|---|
| Engine, ECS, rendering | `bevy` (latest stable that all deps support) | One ECS for client and server; 3D with PBR and lighting out of the box |
| Headless server | `bevy` with `MinimalPlugins` (no window, no renderer) | Same game code, no GPU needed on the server |
| Replication | `bevy_replicon` | Server-authoritative and ECS-native; stable, well documented, simpler than full prediction frameworks |
| Transport | `bevy_replicon_renet` (renet, UDP) | First-party transport for replicon. Fallback: `bevy_replicon_quinnet` (QUIC) if renet lags behind Bevy |
| Voxel storage and meshing | Your own code in `shared`, optionally the `block-mesh` crate for greedy meshing | Cube voxels are simple; owning the code avoids fighting a framework |
| World generation noise | `noise` or `fastnoise-lite` | Deterministic, seedable 3D noise for caves and ore veins |
| Player physics | Your own AABB-vs-voxel collision | Deterministic and identical on client and server; far simpler than a physics engine for block worlds |
| Item physics (optional) | `avian3d` | Only if dropped items need to tumble; otherwise items snap to the ground |
| UI (menus, inventory, crafting, machines) | `bevy_egui` | Immediate-mode UI is the fastest way to build dense inventory and machine screens |
| HUD (crosshair, hotbar, vitals) | Bevy UI | Lightweight, always on screen |
| Content data | `serde` + `ron` files in `assets/data/` | Human-editable, hot-reloadable recipes and balance |
| Network messages | `serde` + `postcard` (replicon's default) | Compact binary |
| Save files | Region files with `postcard` + `zstd`; player and base data in SQLite via `rusqlite` | Chunks are bulk binary; players and structures are queryable records |
| Accounts and server list (M10) | `axum` + `sqlx` (SQLite, Postgres-ready) | Small web service for login tokens and a server browser |
| Logging | `tracing` (Bevy's log plugin) | Structured logs on both sides |
| Randomness | `rand` + `rand_chacha`, seeded per chunk | Same seed gives the same world on every machine |
| Testing | `cargo test`, headless Bevy `App` tests, `proptest` | Logic is tested without opening a window |

**Setup notes:**

- Enable Bevy's `dynamic_linking` feature for dev builds only. Set
  `[profile.dev.package."*"] opt-level = 3` so dependencies run fast while your
  own code compiles quickly.
- Use Rust edition 2024 and the current stable toolchain.
- If a crate in the table has no release for your chosen Bevy version, use the
  alternative named in the table and note the change in `PROGRESS.md`. Do not
  downgrade Bevy for one optional crate.

---

## 3. Workspace architecture

Build a Cargo workspace with four crates. The rule that keeps it sane:
**`shared` holds every rule of the game, `server` decides, `client` shows.**

```text
deepcore/
├── Cargo.toml # [workspace], pinned versions, profiles
├── PROMPT.md PROGRESS.md docs/PLAN.md
├── assets/
│ ├── data/ # blocks.ron items.ron recipes.ron machines.ron
│ │ # tools.ron tech_tree.ron layers.ron hazards.ron
│ └── shaders/ # voxel.wgsl (optional custom material)
├── crates/
│ ├── shared/ # pure game logic, no rendering, no sockets
│ │ ├── content/ # RON loaders, registries (BlockId, ItemId, RecipeId)
│ │ ├── voxel/ # Chunk, ChunkPos, BlockPos, palette compression
│ │ ├── worldgen/ # layers, caves, ore veins, deterministic by seed
│ │ ├── movement/ # AABB collision, player controller (both sides)
│ │ ├── inventory/ # stacks, slots, transfer rules
│ │ ├── crafting/ # recipe matching, machine processing state machine
│ │ ├── power/ # power networks, producers, consumers
│ │ ├── hazards/ # heat, gas, water, radiation, cave-ins
│ │ ├── protocol/ # replicated components, client→server intents,
│ │ │ # server→client events, channel setup
│ │ └── lib.rs # SharedPlugin
│ ├── server/ # headless authoritative server binary
│ │ ├── net/ # connections, auth handshake, visibility
│ │ ├── world/ # chunk loading, generation workers, persistence
│ │ ├── sim/ # mining validation, machine ticks, power solve, hazards
│ │ └── main.rs
│ ├── client/ # windowed game binary
│ │ ├── render/ # async chunk meshing, texture atlas, lighting, effects
│ │ ├── input/ # first-person controls, block targeting raycast
│ │ ├── prediction/ # local movement prediction and reconciliation
│ │ ├── ui/ # egui screens: inventory, crafting, machines, tech tree
│ │ ├── hud/ # hotbar, crosshair, vitals, depth meter, Geiger counter
│ │ ├── audio/ # dig, place, machine hum
│ │ └── main.rs
│ └── lobby/ # axum service: accounts, tokens, server list (M10)
└── tools/
└── atlas_gen/ # generates the block texture atlas PNG
```

### Runtime model

- **Server tick:** a fixed 30 Hz simulation in Bevy's `FixedUpdate`. Machines,
  power and hazards tick here.
- **Client frame:** uncapped rendering. The local player's movement is predicted
  every frame with the shared movement code.
- **Hosting from the client:** add a "Host game" button that runs the server
  plugins inside the client process, on a background `App` or as a child process.
  This is how most players will play with friends.
- **Chunks:** 32×32×32 blocks. The server generates and stores them. The client
  receives compressed chunk data on a reliable channel and meshes it on Bevy's
  `AsyncComputeTaskPool`.
- **Block storage:** `u16` block IDs, palette-compressed per chunk. Block
  entities (machines, chests) are ECS entities with a `BlockPos`, not voxel data.

### Placeholder art (no downloads)

`tools/atlas_gen` builds a texture atlas with 16×16-pixel tiles from each block's
`color` and `pattern` fields in `blocks.ron`. Patterns cover noise speckle for
stone, bright flecks for ore, and glow for uranium. Machines are built from
simple Bevy primitive meshes (cuboids, cylinders) with emissive parts when
running. The game stays fully playable and readable with zero art assets.

---

## 4. Game design: the world and its ores

Each server has one shared, persistent mine, **1024 × 1024 blocks wide and 512
blocks deep**, with an unbreakable bedrock floor and walls. The surface is a
flat-ish camp area (the **Base**) where players build machines. Depth is the
progression axis: every layer down is harder to dig, richer and more dangerous,
and needs a better tool to enter.

### Core rules

- Every block has a **hardness** (seconds to break with a tier-1 tool) and a
  **minimum tool tier**. A tool below the minimum tier cannot break the block.
  The client then shows a red crosshair and the message "Too hard — needs Tier N".
- Break time = `hardness / (tool_speed × power_multiplier)`, rounded to server
  ticks.
- Mined blocks drop items straight into the miner's inventory when there is room.
  Otherwise they drop as a pickup in the world.
- Ore veins come from 3D noise, clustered in blobs of 4–40 blocks. Rarer ores
  form smaller, sparser veins. Generation is fully deterministic from the world
  seed and chunk position.
- Noise-carved caves (tunnels and caverns) appear in every layer, so players can
  explore, not only dig.
- Hauling matters. Ore must be carried or transported (minecarts, lifts) to the
  Base. This is what makes logistics structures worth building.

### Layers

| # | Layer | Depth below surface | Base block | Min tool tier | Ores found | New hazard |
|---|---|---|---|---|---|---|
| 1 | Topsoil | 0–16 | Dirt, gravel, clay, sand | 1 | Coal, clay, sand | None |
| 2 | Shale Beds | 16–64 | Stone, limestone, shale | 1 | Coal, copper, tin, iron | Darkness |
| 3 | Granite Depths | 64–160 | Granite | 2 | Iron, lead, zinc, sulfur, quartz, bauxite, salt | Flooding |
| 4 | Basalt Furnace | 160–288 | Basalt | 3 | Nickel, tungsten, chromite, graphite, fluorite, zircon, gold | Heat, gas pockets |
| 5 | Deep Crust | 288–448 | Gneiss, obsidian | 4 | Uranium (pitchblende), monazite, rare earths | Radiation, cave-ins |
| 6 | Mantle Edge | 448–512 | Mantle rock | 5 | Dense uranium veins, core crystals (cosmetic trophy) | Extreme heat and radiation |

### Ores and what they are for

| Ore block | Drops | Layer | Rarity | Main use |
|---|---|---|---|---|
| Coal seam | Coal | 1–3 | Common | Furnace and boiler fuel, steel making |
| Clay | Clay | 1 | Common | Bricks for furnaces and kilns |
| Sand | Sand | 1 | Common | Glass, casting molds |
| Malachite | Raw copper | 2 | Common | Wire, pipes, bronze, motors |
| Cassiterite | Raw tin | 2 | Uncommon | Bronze (with copper) |
| Hematite | Raw iron | 2–3 | Common | Iron, steel, almost every machine |
| Galena | Raw lead | 3 | Uncommon | Radiation shielding, batteries |
| Sphalerite | Raw zinc | 3 | Uncommon | Brass; refining byproduct **cadmium** (reactor control rods) |
| Sulfur deposit | Sulfur | 3–4 | Uncommon | Sulfuric acid for leaching |
| Quartz | Quartz | 3 | Uncommon | Silicon, circuits, glass lenses |
| Bauxite | Raw bauxite | 3 | Uncommon | Aluminium (needs electricity) |
| Rock salt | Salt | 3 | Common | Chlorine and lye for the chemical plant |
| Pentlandite | Raw nickel | 4 | Uncommon | Heat-resistant alloys, stainless steel |
| Wolframite | Raw tungsten | 4 | Rare | Tungsten-carbide drill bits, centrifuge parts |
| Chromite | Raw chromium | 4 | Uncommon | Stainless steel |
| Graphite vein | Graphite | 4 | Uncommon | Reactor moderator blocks, electrodes |
| Fluorite | Fluorite | 4 | Rare | Fluorine compounds for uranium conversion |
| Zircon | Raw zircon | 4 | Rare | Zirconium fuel cladding |
| Gold vein | Raw gold | 4 | Rare | High-tier circuits |
| Pitchblende | Uranium ore | 5–6 | Very rare | The nuclear chain |
| Monazite | Monazite sand | 5 | Rare | Rare earths for magnets in high-tier motors and centrifuges |

Uranium is **radioactive in the world**. Standing within 4 blocks of exposed
pitchblende slowly raises the player's radiation dose (see Hazards in Section 5).
The Geiger counter clicks faster as uranium gets closer, which is how players
find it. Every ore must have at least one use in the progression, with no
dead-end items.

### Player basics

- First-person controls: WASD, jump, sprint, crouch. Left mouse mines; right
  mouse places or interacts. 9-slot hotbar plus a 36-slot backpack.
- **Vitals:** health, stamina (sprinting, climbing), oxygen (gas and water), body
  heat and radiation dose. Death drops the backpack, not the hotbar, as a
  recoverable crate at the death spot. The player respawns at the Base.
- **Light:** the mine is dark below layer 1. Players carry a headlamp (oil or
  battery) and can place lamps.

---

## 5. Progression: from steel pickaxe to nuclear drill

Progression runs on one loop: **dig deeper → bring ore home → build a structure
→ it unlocks a better tool or refining step → dig deeper.** It is organised in
six Eras. Each Era is gated by materials only reachable with the previous Era's
tool, so players cannot skip ahead.

### The ladder

| Era | Name | Materials from layers | Key new structures | Tool unlocked | Tier | Speed | Reaches layer |
|---|---|---|---|---|---|---|---|
| 0 | Hand Tools | 1–2 | Workbench, Storage Crate, Ladder, Timber Support, Oil Lamp | Steel Pickaxe (start) | 1 | 1.0× | 1–2 |
| 1 | Smelting | 1–2 | Brick Kiln, Stone Furnace, Anvil, Blast Furnace | Hardened Steel Pickaxe | 2 | 1.6× | 3 |
| 2 | Steam | 1–3 | Hand Pump, Steam Boiler, Steam Engine, Crusher, Ore Washer, Rails and Minecart, Mine Lift | Steam Drill | 3 | 3.0× | 4 |
| 3 | Electric | 3 | Steam Turbine, Generator, Cables and Poles, Battery Bank, Arc Furnace, Electrolysis Cell, Circuit Assembler, Charging Station, Vent Fan, Electric Pump | Ore Scanner, Headlamp Mk2 | — | — | 4 (safely) |
| 4 | Chemical | 4 | Chemical Plant, Alloy Furnace, Machine Shop, Rare-Earth Separator, Decon Shower | Tungsten-Carbide Drill | 4 | 5.0× | 5 |
| 5 | Nuclear | 5 | Leaching Tank, Precipitator, Conversion Plant, Centrifuge Cascade, Fuel Fabricator, Reactor, Cooling Pool, Waste Vault | **Nuclear Drill** | 5 | 12.0×, 3×3 area | 6 |

Speed is the multiplier in the break-time formula from Section 4. The Nuclear
Drill also mines a 3×3 face per action, so its effective rate is about 100× the
starting pickaxe. That leap is the reward the whole game builds toward.

### Era by era

**Era 0: Hand Tools.** Trees ring the surface Base and can be chopped with the
pickaxe for timber. The Workbench crafts ladders, timber supports, crates, oil
lamps and clay bricks by hand. Goal: find coal, copper and iron in the Shale
Beds.

**Era 1: Smelting.** The Brick Kiln fires clay into bricks. The Stone Furnace
burns coal to smelt raw copper, tin, iron, lead and zinc into ingots. It also
alloys copper and tin into bronze. The Anvil turns ingots into plates, rods,
gears and tool heads. The Blast Furnace turns iron, coal and limestone flux into
steel. Steel makes the Hardened Steel Pickaxe, which opens the Granite Depths.

**Era 2: Steam.**
- **Steam power:** a Hand Pump (later steam-driven) supplies water to the Steam
  Boiler, which burns coal into steam. Copper pipes carry the steam, and the
  Steam Engine turns it into shaft power.
- **Ore processing:** the Crusher and Ore Washer run on shaft power and raise ore
  yield from 1 ingot per ore to 1.5 (crushed) and 2.0 (crushed and washed). That
  yield bonus is the reason to build them.
- **Logistics:** rails, minecarts and the Mine Lift solve the hauling problem.
- **Tool:** the Steam Drill (steel, brass gears, copper pipes) runs on steam
  canisters refilled at the boiler and opens the Basalt Furnace.

**Era 3: Electric.**
- **Power:** the Steam Turbine and Generator turn steam into electricity,
  carried on copper cables between poles.
- **Processing:** the Arc Furnace smelts faster and makes silicon from quartz.
  The Electrolysis Cell turns bauxite into aluminium and salt into chlorine and
  lye. The Circuit Assembler makes circuits.
- **Storage:** lead and sulfuric acid make battery packs for the Battery Bank and
  for portable tools.
- **Safety and scouting:** Vent Fans and Electric Pumps make deep tunnels safe
  from gas and flooding. The Ore Scanner reveals ore within 16 blocks.

**Era 4: Chemical.**
- **Chemistry:** the Chemical Plant makes sulfuric acid (sulfur, water and power)
  and fluorine compounds (fluorite and acid).
- **Advanced materials:** the Alloy Furnace makes stainless steel (steel, nickel
  and chromium) and tungsten carbide (tungsten and graphite). The Machine Shop
  makes precision parts: bearings, turbine blades, centrifuge rotors. The
  Rare-Earth Separator turns monazite into rare-earth magnets.
- **Hazard gear:** Cooling Suit, Respirator, Lead-Lined Hazmat Suit, Geiger
  Counter.
- **Tool:** the battery-powered Tungsten-Carbide Drill opens the Deep Crust.

**Era 5: Nuclear.** See the chain below.

### The nuclear chain

Every step is a separate structure the players build, power and feed. All of it
is a game abstraction: items in, timer, items out.

1. **Crusher** (existing): uranium ore → crushed uranium ore.
2. **Leaching Tank**: crushed uranium ore + sulfuric acid → uranium solution +
   radioactive tailings.
3. **Precipitator**: uranium solution + lye → yellowcake.
4. **Conversion Plant**: yellowcake + fluorine compound → uranium hexafluoride
   canister.
5. **Centrifuge Cascade**: a line of 4–16 linked Centrifuge blocks (rare-earth
   magnets, stainless rotors, high power draw). Each canister fills an enrichment
   meter, and more centrifuges in the cascade means faster enrichment. Output: an
   enriched canister plus a depleted canister.
6. **Fuel Fabricator**: two recipes.
   - Enriched canister + zirconium tubing → fuel rod.
   - Enriched canister + stainless casing + lead shielding → **compact core
     cell** for the Nuclear Drill.
7. **Control rods**: cadmium + stainless steel → control rod. Cadmium is a
   byproduct of zinc refining, stockpiled since Eras 1–3.
8. **Reactor**: a multi-block structure that produces about 20× the power of the
   coal grid.
   - **Build:** a 5×5×5 shell of stainless reactor casing and lead shielding.
     Inside go graphite moderator blocks, fuel rods and control rods, plus a
     coolant pump and a heat exchanger feeding turbines.
   - **Operation:** players run it from a reactor console. Core temperature rises
     when rods are withdrawn and falls when rods are inserted or coolant flows.
   - **Failure:** above the warning line it sounds an alarm. Above the limit it
     scrams automatically if a Scram Panel is built. Without one, it overheats,
     destroys the reactor and leaves a radiation zone at the Base for 20 minutes.
9. **Nuclear Drill**: compact core cell + tungsten-carbide drill head + stainless
   frame + rare-earth motor + lead shielding. One core cell powers about 2,000
   block breaks. Spent cells must be returned.
10. **Waste handling**:
    - Spent fuel rods and core cells go to the Cooling Pool for a timed
      cool-down, then into Waste Casks stored in a Waste Vault.
    - Waste left loose at the Base emits radiation.
    - Depleted canisters can be cast into dense shielding plates, which shield
      better than lead.

### Power model

- **Fuel heat** (Eras 0–1): each furnace burns its own fuel slot. One coal burns
  for 80 seconds.
- **Steam** (Era 2): boilers produce steam units per second into a pipe network,
  and consumers draw from it. A pipe network is a set of connected pipe blocks.
  Solve total supply against demand each tick.
- **Electricity** (Era 3+): generators produce kW into a cable network. If demand
  exceeds supply, every consumer on that network runs at `supply / demand` speed.
  Battery Banks absorb surplus and cover deficits. Show a network overlay with
  supply, demand and stored energy.
- Recompute network topology only when a pipe, cable or machine is placed or
  removed. Each tick only sums numbers.

### Machines (common behaviour)

Every machine is a block entity with:
- input slots and output slots;
- an optional fuel or power requirement;
- a current recipe and a progress value.

A machine runs when inputs are present, the output has room and energy is
available. Otherwise it shows why it is idle: "No power", "Output full",
"Missing sulfuric acid".

From Era 3, machines accept up to two upgrade modules:
- **Speed:** +50% speed, +80% power use.
- **Efficiency:** −30% power use.

From Era 2, conveyors and item pipes can link machines, so players can build
automated production lines.

### Research

Recipes unlock through a team-shared **Tech Tree** at the Research Desk in the
Base. Each node requires its parent nodes plus a delivery of items. Example:
"Steam Power" costs 20 steel plates, 10 copper pipes and 5 bronze gears. This
gives every session a clear next goal and makes cooperation pay off. Show the
tree as a zoomable node graph with locked, available and unlocked states.

### Hazards

| Hazard | Where | Effect | Counter |
|---|---|---|---|
| Darkness | Layer 2+ | No visibility without light | Oil lamp, headlamp, placed lamps |
| Flooding | Layer 3+ water pockets | Breaking a pocket releases water that spreads by simple cellular flow (max 512 blocks per pocket); being underwater drains oxygen | Electric Pump, sealing blocks |
| Gas pockets | Layer 4+ | Releases methane: drains oxygen; an open flame ignites it for burst damage | Respirator, Vent Fan, electric lights |
| Heat | Layer 4+, rising with depth | Body heat climbs; above the limit, health drains | Cooling Suit, Vent Fan, coolant lines |
| Cave-ins | Layer 5+ | Unsupported ceiling spans wider than 7 blocks can collapse when mined beneath | Timber or steel supports |
| Radiation | Near uranium, waste, reactor | Dose accumulates: 100 = screen grain, 300 = no health regen, 600 = damage over time | Hazmat Suit (−80%), lead or shielding blocks, Decon Shower clears dose |

### Pacing targets

Target times for a group of 2–4 players (solo play takes about 1.5–2× longer):

| Milestone in play | Target time |
|---|---|
| Era 1 | about 30 minutes |
| Era 2 | by 1.5 hours |
| Era 3 | by 3 hours |
| Era 4 | by 5 hours |
| First Nuclear Drill | 8–12 hours |

Put every tunable number in the RON files so balance can change without code:
hardness, speeds, recipe times, yields, vein frequency, power values. Write a
small `cargo run -p shared --bin balance_report` tool that walks the tech tree
and prints the total raw ore needed per Era, so you can check pacing on paper.

---

## 6. Data-driven content

All content lives in RON files under `assets/data/` and loads at startup into
registries in the `shared` crate. String IDs in files (`"raw_copper"`) resolve to
compact numeric IDs (`ItemId(u16)`) at load time.

Validate everything on load and fail fast with a clear message. Catch at least:
- unknown item references;
- recipes with no machine that can make them;
- tech nodes with missing parents;
- ores with no use.

During the handshake the server sends a hash of the content files. A client with
a different hash is refused with "Content mismatch".

Use these shapes as a starting point. Extend fields as needed, but keep the files
the single source of truth.

```rust
// blocks.ron
[
(id: "granite", hardness: 2.5, min_tier: 2, drops: [("granite_cobble", 1)],
color: (0.55, 0.50, 0.48), pattern: Speckle, layer_hint: Some(3)),
(id: "pitchblende", hardness: 6.0, min_tier: 4, drops: [("uranium_ore", 1)],
color: (0.15, 0.18, 0.12), pattern: OreFleck((0.4, 0.9, 0.3)),
emissive: 0.3, radiation: Some((radius: 4, dose_per_sec: 0.5))),
]

// items.ron
[
(id: "steel_ingot", name: "Steel Ingot", stack: 64, category: Material),
(id: "steam_drill", name: "Steam Drill", stack: 1, category:
Tool((tier: 3, speed: 3.0, area: (1, 1), energy: Steam(capacity: 300)))),
(id: "nuclear_drill", name: "Nuclear Drill", stack: 1, category:
Tool((tier: 5, speed: 12.0, area: (3, 3), energy: CoreCell(breaks: 2000)))),
]

// recipes.ron
[
(id: "smelt_iron", machine: "stone_furnace", time: 6.0,
inputs: [("raw_iron", 1)], outputs: [("iron_ingot", 1)], tech: "smelting"),
(id: "leach_uranium", machine: "leaching_tank", time: 20.0, power_kw: 40,
inputs: [("crushed_uranium_ore", 2), ("sulfuric_acid", 1)],
outputs: [("uranium_solution", 1), ("radioactive_tailings", 1)], tech: "uranium_milling"),
]

// machines.ron
[
(id: "blast_furnace", name: "Blast Furnace", size: (2, 3, 2),
energy: Fuel(slots: 1), input_slots: 3, output_slots: 2,
build_cost: [("fire_brick", 40), ("iron_plate", 12)], tech: "steelmaking"),
(id: "centrifuge", name: "Centrifuge", size: (1, 3, 1),
energy: Electric(kw: 150), cascade: Some((min: 4, max: 16)),
build_cost: [("stainless_rotor", 2), ("rare_earth_magnet", 4), ("circuit_mk2", 2)],
tech: "enrichment"),
]

// tech_tree.ron
[
(id: "steam_power", name: "Steam Power", era: 2, parents: ["steelmaking"],
cost: [("steel_plate", 20), ("copper_pipe", 10), ("bronze_gear", 5)],
unlocks: ["steam_boiler", "steam_engine", "copper_pipe_block"]),
]

// layers.ron
[
(id: "granite_depths", depth: (64, 160), base_block: "granite",
ores: [(block: "galena", frequency: 0.012, vein_size: (4, 18)), ...],
hazards: [Flooding((pocket_chance: 0.002, max_volume: 512))],
ambient_temp: 18.0),
]
```

Write the complete content: all six layers, all ores in Section 4, every
structure and tool in Section 5, and every intermediate item needed to connect
them. Aim for roughly:

| Content type | Target count |
|---|---|
| Ores and raw materials | ~25 |
| Items | 80–120 |
| Recipes | 100–150 |
| Machines and blocks | 40–50 |
| Tech nodes | 30–40 |

Run the content validator and `balance_report` after every content change.

---

## 7. Multiplayer and networking

Deepcore is **co-op**: up to 16 players share one world, one Base, one tech tree
and one goal. Each player has a private inventory. The server is authoritative
for everything except the local player's predicted movement.

### Connection flow

1. The client connects by direct IP, through "Host game", or from the lobby
   server list (M10).
2. Handshake: the client sends its protocol version, content hash and a display
   name (or an auth token once the lobby exists). The server rejects mismatches
   with a readable reason.
3. The server spawns the player entity at the Base and streams chunks
   nearest-first.
4. When the chunks around the player have arrived and meshed, the client leaves
   the loading screen.

### What travels how

| Data | Mechanism | Channel | Notes |
|---|---|---|---|
| Player position, rotation, held item, animation | Replicated components | Unreliable, every tick | Other players interpolated with a ~100 ms buffer |
| Player vitals | Replicated, owner only | Reliable on change | Teammates see a health bar only |
| Inventory contents | Server → owner message on change | Reliable | Never replicated to other clients |
| Machines (type, facing, running state, coarse progress) | Replicated components | Reliable on change | Full slot contents only while a player has that machine's UI open |
| Chunk data | Server → client message | Reliable, ordered | Palette + zstd compressed; only chunks within view radius |
| Block changes | Server → client message, batched per tick | Reliable, ordered | Only to clients with that chunk loaded |
| Power and steam network stats | Server → client, once per second | Unreliable | Only to clients viewing the overlay |
| Tech tree state, chat, notifications | Server → all | Reliable | |
| Player intents | Client → server events | Reliable; movement unreliable and sequenced | See list below |

### Client intents

| Intent | Fields |
|---|---|
| `MoveInput` | `seq`, `dir`, `jump`, `sprint`, `crouch`, `yaw`, `pitch` |
| `StartMining` | `block`, `face` |
| `StopMining` | — |
| `PlaceBlock` | `pos`, `face`, `item_slot` |
| `Interact` | `target` |
| `MoveItems` | `from`, `to`, `count` |
| `Craft` | `recipe`, `count` |
| `SetMachineRecipe` | `machine`, `recipe` |
| `ResearchDeposit` | `node`, `items` |
| `Chat` | `text` |
| `Respawn` | — |

The server validates every intent:
- reach: 5 blocks, or 6 with the drill;
- line of sight, by voxel raycast;
- tool tier against the block;
- the item is actually in the slot;
- the recipe is unlocked;
- the machine is within reach.

Invalid intents are dropped and logged. They must never crash or desync the
server.

### Mining over the network

1. The client sends `StartMining` and immediately plays a predicted crack
   animation using the shared break-time formula.
2. The server runs the real timer. On completion it removes the block, grants the
   drops and broadcasts the block change.
3. If the player looks away or releases the button, the client sends
   `StopMining`.

Area tools like the Nuclear Drill break the full 3×3 face in one server action.

### Movement prediction

Movement code lives in `shared::movement` and is pure and deterministic: fixed
timestep, no randomness.

1. The client applies each `MoveInput` locally and keeps a buffer of
   unacknowledged inputs.
2. The server applies the same inputs and replicates the authoritative position
   with the last processed `seq`.
3. The client rewinds to that state and replays the unacknowledged inputs.

Corrections under 0.3 blocks are smoothed over 100 ms instead of snapping.

### Interest management

Each client receives chunks within 6 chunks horizontally and 4 vertically of its
player. Underground, vertical range matters as much as horizontal. Send unload
messages as the player moves away. Use replicon's per-client visibility so
replicated entities outside that range are not sent.

### Persistence

- Unmodified chunks are never saved, because they regenerate from the seed.
  Modified chunks go to region files of 8×8×8 chunks.
- SQLite stores players (position, vitals, inventory), the tech tree, machines
  with their contents, and the reactor state.
- Autosave every 5 minutes and on clean shutdown without stalling the tick.
  Serialise on the main thread and write on a background task.

### Budgets and test tools

- Keep steady-state bandwidth under 50 KB/s per client. Chunk-streaming bursts
  may go higher.
- Add `--simulate-latency <ms>` and `--simulate-loss <percent>` flags to both
  binaries.
- Add `server --bots <N>`: headless bot clients that walk, dig and craft
  randomly, for load and desync testing.
- Add a debug overlay on F3 showing:
  - FPS and server tick time;
  - ping and bandwidth in and out;
  - loaded chunks;
  - prediction corrections per second.

---

## 8. Milestones

Build in this order. Networking comes early (M2) on purpose: retrofitting
multiplayer onto a single-player game is the most common way projects like this
fail. Each milestone ends with a playable build, green tests and an updated
`PROGRESS.md`.

### M0: Workspace and skeleton
- Create the workspace and four crates, then choose and pin crate versions
  (Section 2).
- Set up build profiles, `rustfmt.toml`, the clippy config, and a
  `scripts/check.sh` that runs fmt, clippy and tests.
- `client` opens a window with a camera and a lit test cube. `server` runs
  headless at 30 Hz and logs its tick rate.
- **Done when:** `cargo run -p server` and `cargo run -p client` both start
  cleanly and `scripts/check.sh` passes.

### M1: Voxel world (local, single process)
- Chunk storage with palette compression, plus `ChunkPos`/`BlockPos` maths with
  unit tests.
- World generation from `layers.ron`: all six layers, caves, ore veins and
  bedrock bounds. It must be deterministic, so the same seed gives identical
  chunks; test this.
- Procedural texture atlas from `blocks.ron`.
- Greedy chunk meshing on the async task pool, with face culling between chunks.
- First-person controller with AABB collision in `shared::movement`, and a block
  targeting raycast with an outline.
- Mining with hardness and tier rules, block placing, a hotbar and a depth meter.
- **Done when:** a player can dig from the surface to the bottom of layer 2 and
  is blocked by layer 3 granite with the steel pickaxe. The game holds 60 FPS at
  view radius 6 on a mid-range GPU.

### M2: Multiplayer foundation
- Split into an authoritative server and a client, with a handshake that checks
  protocol version and content hash.
- Chunk streaming with interest management, and batched block-change messages.
- Replicated players with interpolation; movement prediction and reconciliation.
- Mining and placing through intents with full server validation.
- A simple main menu with "Host game" and "Join by IP".
- Latency and loss simulation flags, and bot clients.
- **Done when:**
  - a server plus two clients on one machine see each other move and dig in real
    time;
  - movement still feels responsive at 150 ms simulated latency;
  - a 10-minute run with 8 bots shows no desync and no server panic.

### M3: Content, inventory, crafting (Era 0)
- RON loaders, registries, the content validator and `balance_report`.
- Backpack and hotbar inventory in egui (drag, split, quick-move), item pickups,
  death crate and respawn.
- Workbench hand-crafting, trees and timber, ladders, supports, oil lamps and
  darkness.
- **Done when:** Era 0 is fully playable in multiplayer and the validator passes
  on all content files.

### M4: Machines, research and Era 1
- Block entities and multi-block placement with a ghost preview and rotation.
- Machine UI with slots and idle reasons.
- Fuel burning, plus the Brick Kiln, Stone Furnace, Anvil and Blast Furnace.
- Research Desk and the Tech Tree graph UI.
- **Done when:** a team can go from nothing to the Hardened Steel Pickaxe and dig
  into layer 3.

### M5: Era 2, steam and logistics
- Pipe networks and the steam solver.
- Hand Pump, Boiler, Steam Engine, Crusher and Ore Washer with yield bonuses.
- Conveyors and item pipes between machines; rails, minecarts and the Mine Lift.
- Steam Drill with refillable canisters.
- Flooding hazard and oxygen.
- **Done when:** a team can automate ore processing from minecart to ingot chest
  and reach layer 4 with the Steam Drill.

### M6: Era 3, electricity
- Cable networks and the electric solver, with batteries, brownout scaling and
  the network overlay.
- Machines: Turbine, Generator, Battery Bank, Arc Furnace, Electrolysis Cell,
  Circuit Assembler, Charging Station.
- Upgrade modules and the Ore Scanner.
- Vent Fan, Electric Pump and the gas hazard.
- **Done when:** a coal-and-steam power plant runs the Base, brownouts behave
  correctly under overload, and gas pockets can be cleared.

### M7: Era 4, chemistry and deep metallurgy
- Chemical Plant, Alloy Furnace, Machine Shop, Rare-Earth Separator and Decon
  Shower.
- Heat hazard and cave-ins.
- Cooling Suit, Respirator, Hazmat Suit and Geiger Counter.
- Tungsten-Carbide Drill.
- **Done when:** a team can reach layer 5, find uranium with the Geiger counter,
  and survive there with the right gear.

### M8: Era 5, nuclear
- **Processing:** Leaching Tank, Precipitator, Conversion Plant, and the
  Centrifuge Cascade as a multi-block that scales with its length.
- **Fuel:** Fuel Fabricator and control rods.
- **Reactor:** the multi-block with its console, scram, and overheat
  consequences.
- **Waste:** Cooling Pool and Waste Vault.
- **Tool:** the Nuclear Drill with 3×3 mining.
- **Hazard:** radiation everywhere it applies.
- **Done when:**
  - a headless integration test feeds raw materials through every nuclear
    structure and produces a working Nuclear Drill and a stable reactor;
  - in a real session, a player can bore a 3×3 tunnel into layer 6.

### M9: Persistence and polish
- Region-file and SQLite saving, autosave, and crash-safe writes (write to a temp
  file, then rename).
- Audio (dig, place, machine hum, reactor alarm, Geiger clicks), particles (dust,
  sparks, steam) and machine running animations.
- Main menu, pause menu, and settings: keybinds, FOV, mouse sensitivity, render
  distance, volume.
- A guided objective list for the first hour ("Smelt your first iron ingot",
  "Build a Blast Furnace"), so new players always know the next step.
- Balance pass using `balance_report` against the pacing targets.
- **Done when:** a server restart restores the world, machines, inventories and
  tech tree exactly, and a new player can reach Era 1 using only the objective
  list.

### M10: Full stack, accounts and server browser
- `lobby` crate built on `axum` + `sqlx` (SQLite, with Postgres-compatible
  queries).
- Accounts with argon2-hashed passwords, session tokens and rate-limited login.
- Dedicated servers register and send a heartbeat (name, player count, ping
  endpoint).
- The client shows a server browser and logs in before joining. Servers verify
  tokens with the lobby.
- Dockerfiles for the dedicated server and the lobby, release build profiles, and
  build instructions for Windows, Linux and macOS.
- **Done when:** a fresh machine can run the lobby and a dedicated server with
  Docker, and two clients can register, log in, find the server in the browser
  and play together.

---

## 9. Quality bar and definition of done

### Tests you must write
- **Shared logic unit tests:**
  - chunk indexing and palette round-trips;
  - world generation determinism;
  - the break-time formula;
  - inventory moves and stacking, with `proptest`;
  - recipe matching;
  - steam and power network solving, including brownouts;
  - radiation dose maths.
- **Headless app tests:** build a Bevy `App` with the server plugins and no
  window, script player intents, step the schedule, and assert outcomes (a block
  is mined, an item is crafted, a machine finishes a recipe).
- **Progression integration tests:** one test per Era that starts from that Era's
  raw inputs and drives every structure to the Era's unlock. The M8 nuclear test
  is the most important test in the project.
- **Network tests:** run server and client apps in one process over a local
  transport, apply simulated latency, and check that client state converges to
  server state.

### Performance budgets

| Metric | Target |
|---|---|
| Client frame rate, view radius 6, mid-range GPU | ≥ 60 FPS |
| Chunk mesh time (32³, async) | < 4 ms |
| Server tick, 16 players and 200 machines | < 10 ms (of a 33 ms budget) |
| Chunk generation | < 5 ms per chunk on a worker thread |
| Bandwidth per client, steady state | < 50 KB/s |
| Server memory, 16 players | < 2 GB |

Profile with Bevy's diagnostics and `tracing` spans before optimising. Never
block the main thread on generation, meshing, disk or network.

### Code standards
- `cargo clippy --workspace -- -D warnings` passes and `cargo fmt` is applied.
- No `unwrap()` on network input, file input or content lookups. Use `Result` and
  log errors. `unwrap()` is acceptable only for invariants you document in a
  comment.
- Every public type and plugin in `shared` has a one-line doc comment.
- Replicated components and protocol messages are versioned by a single
  `PROTOCOL_VERSION` constant. Bump it whenever their shape changes.

### Definition of done for the whole project
1. A fresh clone builds with `cargo build --release` and runs with no downloaded
   assets.
2. Two or more players can host or join, play together, and progress from the
   steel pickaxe through all six Eras to a working Nuclear Drill and a stable
   reactor.
3. Every ore has a use, every structure in Section 5 exists and works, and every
   Era's integration test passes.
4. The world, machines, inventories and tech tree survive a server restart.
5. Accounts and the server browser work through the lobby service, with
   Dockerfiles for deployment.
6. `README.md` explains how to build, host, join, and add new content through the
   RON files. `PROGRESS.md` lists any known issues.
