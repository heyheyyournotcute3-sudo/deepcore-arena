# PROGRESS

Living status file. A future session should be able to resume from this alone.

## How to run

```bash
# Toolchain: Rust stable (edition 2024). If missing components:
rustup component add clippy rustfmt

# Server: headless, authoritative, fixed 30 Hz.
cargo run -p deepcore-server

# Client: windowed game.
cargo run -p deepcore-client

# Full quality gate (fmt + clippy -D warnings + tests).
./scripts/check.sh
```

Headless rendering for verification (no GPU present in the build sandbox):

```bash
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json ./target/debug/client
```

## Pinned dependency set

Verified against crates.io on **2026-12-06**. All four Bevy-facing crates agree
on `bevy ^0.20`:

| Crate | Version | Notes |
|---|---|---|
| `bevy` | `=0.20.1` | latest stable; `default-features = false` + explicit features |
| `bevy_replicon` | `=0.45.5` | depends on `bevy ^0.20` |
| `bevy_replicon_renet` | `=0.21.0` | depends on `bevy ^0.20`, `bevy_renet ^6.0` |
| `bevy_egui` | `=0.43.1` | depends on `bevy_* ^0.20.0`, `egui ^0.36` |

Other pins: `ron 0.12.3`, `postcard 1.1.3`, `noise 0.9.0`, `rand 0.9.2`,
`rand_chacha 0.10.0`, `zstd 0.14.3`, `tracing 0.1.41`, `thiserror 2.0.17`,
`proptest 1.9.0` (dev).

## Bevy 0.20 API notes (learned the hard way — do not regress)

These differ from older Bevy releases and are easy to get wrong from memory:

- `AmbientLight` is a **component**; the global default is the
  **`GlobalAmbientLight` resource**.
- `PointLight` / `DirectionalLight` use **`shadow_maps_enabled`** (not the old
  `shadows_enabled`). Lights also accept `contact_shadows_enabled`.
- Rendering uses the `Mesh3d(handle)` / `MeshMaterial3d(handle)` component pair.
- `Camera3d::default()` is the camera component.
- `MinimalPlugins` contains **no logger** — the server adds
  `bevy::log::LogPlugin` explicitly.
- Bevy 0.20's `3d_scene` example uses a `bsn!` scene macro, but the plain
  `commands.spawn((Mesh3d(..), MeshMaterial3d(..)))` API still works and is what
  this project uses.

## Milestone status

### M0 — Workspace and skeleton — DONE

Built:
- Cargo workspace, 4 members: `crates/shared`, `crates/server`, `crates/client`,
  `tools/atlas_gen`. (`lobby` is deferred to M10 and is not yet a member.)
- `shared` crate skeleton with `SharedPlugin` and `PROTOCOL_VERSION`.
- Exact-pinned dependency set (above).
- Build profiles: `[profile.dev.package."*"] opt-level = 3`, `release` with thin
  LTO + `codegen-units = 1`.
- `rustfmt.toml`, `clippy.toml`, workspace lints (`unsafe_code = "forbid"`,
  `unwrap_used`/`expect_used` warned).
- `scripts/check.sh`: fmt check → clippy `-D warnings` → tests.
- **Server**: `MinimalPlugins` + `ScheduleRunnerPlugin`, `Time<Fixed>` at 30 Hz,
  tick counter, heartbeat log. Verified output: steady
  `tick_rate_hz=30.0`.
- **Client**: window (1280×720), camera, directional sun with shadows, point fill
  light, ground plane and a rotating lit cube.

Verified:
- `cargo check --workspace --all-targets` clean.
- `cargo clippy --workspace --all-targets -- -D warnings` clean.
- `cargo fmt --all --check` clean.
- Server ran and logged 30.0 Hz over multiple seconds.
- Client rendered under Xvfb + lavapipe; screenshot confirms the lit cube, shadow
  and plane draw correctly.

Deferred / not yet built:
- `lobby` crate (M10).
- Real gameplay (M1+).

Known issues:
- Client requires a Vulkan/GPU backend; in this sandbox it falls back to software
  rendering (llvmpipe), which is slow but correct.
- `dynamic_linking` dev feature from the prompt is intentionally **not** enabled
  yet: it conflicts with `default-features = false` feature plumbing and adds
  no value until there is real code to hot-reload. Revisit in M9.

Next: **M1 — voxel world** (palette chunks, position maths, deterministic
worldgen for six layers, procedural atlas, greedy async meshing, first-person
controller with AABB collision, targeting raycast, mining/tier rules, hotbar,
depth meter).
