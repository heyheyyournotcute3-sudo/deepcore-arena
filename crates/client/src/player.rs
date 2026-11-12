//! First-person player controller: input, camera, block targeting, mining and
//! placing.
//!
//! Movement is predicted locally with the shared [`step_player`] so the controls
//! feel instant. In single-player the same code is authoritative; once
//! multiplayer lands (M2) the server runs the identical function and the client
//! reconciles against it.

use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use deepcore_shared::content::{ContentRegistry, ItemId};
use deepcore_shared::mining::raycast::{RayFilter, RayHit, raycast};
use deepcore_shared::mining::{MineDenied, MiningTool, area_offsets, check_mine};
use deepcore_shared::movement::{BlockSolid, PlayerInput, PlayerPhysics, PlayerState, step_player};
use deepcore_shared::voxel::{BlockId, BlockPos};

use crate::render::chunks::ClientChunks;

/// The local player entity.
#[derive(Component)]
pub struct LocalPlayer;

/// Player vitals and interaction state, mirrored from the shared rules.
#[derive(Component)]
pub struct PlayerRuntime {
    /// Authoritative-ish physics state, predicted locally.
    pub state: PlayerState,
    /// Mouse sensitivity multiplier.
    pub sensitivity: f32,
    /// Blocks currently being mined and their progress in ticks.
    pub mining: Option<MiningProgress>,
    /// Hotbar contents as `(item, count)` pairs; `None` is an empty slot.
    pub hotbar: [Option<(ItemId, u32)>; 9],
    /// Selected hotbar slot, 0..8.
    pub selected_slot: usize,
    /// The item in hand, resolved for convenience.
    pub held: Option<ItemId>,
}

/// An in-progress mining operation.
#[derive(Clone, Copy, Debug)]
pub struct MiningProgress {
    /// The block being mined.
    pub block: BlockPos,
    /// Total ticks this block needs.
    pub total_ticks: u32,
    /// Ticks elapsed.
    pub elapsed_ticks: u32,
}

/// The block the crosshair is currently over, refreshed every frame.
#[derive(Resource, Default)]
pub struct TargetedBlock(pub Option<RayHit>);

/// Feedback shown on the HUD when mining is refused.
#[derive(Resource, Default)]
pub struct MiningFeedback {
    /// Message to display, e.g. "Too hard — needs Tier 2".
    pub message: String,
    /// How long the message should remain visible.
    pub timer: f32,
}

/// Fixed simulation rate used for mining timers, matching the server.
pub const TICK_HZ: u32 = 30;
/// Reach in blocks with a normal tool.
pub const REACH: f32 = 5.0;
/// Reach in blocks with an area drill.
pub const REACH_DRILL: f32 = 6.0;

/// The first-person player plugin.
pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TargetedBlock>()
            .init_resource::<MiningFeedback>()
            .insert_resource(PlayerPhysics::default())
            .add_systems(Startup, spawn_player)
            .add_systems(Update, (grab_cursor, look, select_hotbar, decay_feedback))
            .add_systems(
                Update,
                (move_player, update_target, handle_mining, handle_placing).chain(),
            );
    }
}

/// Spawns the camera and the player state at the surface camp.
fn spawn_player(
    mut commands: Commands,
    registry: Res<ContentRegistry>,
    world_gen: Res<deepcore_shared::worldgen::WorldGen>,
) {
    // Stand on the highest solid block at the camp, so the player never spawns
    // inside terrain or floating above it.
    let spawn = find_spawn_point(&world_gen);

    let mut hotbar: [Option<(ItemId, u32)>; 9] = [None; 9];
    // Everyone starts with a steel pickaxe and nothing else.
    if let Some(pick) = registry.0.item("steel_pickaxe") {
        hotbar[0] = Some((pick, 1));
    }

    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(spawn).looking_at(spawn + Vec3::NEG_Z, Vec3::Y),
        LocalPlayer,
        PlayerRuntime {
            state: PlayerState {
                position: spawn,
                ..default()
            },
            sensitivity: 0.0022,
            mining: None,
            hotbar,
            selected_slot: 0,
            held: hotbar[0].map(|(item, _)| item),
        },
        bevy::camera::Projection::Perspective(PerspectiveProjection {
            fov: 75.0_f32.to_radians(),
            near: 0.05,
            far: 1000.0,
            ..default()
        }),
    ));
}

/// Finds a safe standing position at the camp centre.
///
/// Scans downward from above the surface for the first solid block, then places
/// the feet one block above it.
fn find_spawn_point(world_gen: &deepcore_shared::worldgen::WorldGen) -> Vec3 {
    let x = 0;
    let z = 0;
    let top = deepcore_shared::worldgen::WorldGen::surface_y() + 4;
    for y in (0..=top).rev() {
        let block = world_gen.block_at(x, y, z);
        if !block.is_air() {
            return Vec3::new(x as f32 + 0.5, y as f32 + 1.0, z as f32 + 0.5);
        }
    }
    // Fallback: the nominal surface.
    Vec3::new(
        0.5,
        deepcore_shared::worldgen::WorldGen::surface_y() as f32 + 1.0,
        0.5,
    )
}

/// Captures the mouse when the window is clicked, releases it on Escape.
fn grab_cursor(
    mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    let Ok(mut cursor) = windows.single_mut() else {
        return;
    };
    if mouse.just_pressed(MouseButton::Left) && cursor.grab_mode == CursorGrabMode::None {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

/// Mouse look.
fn look(
    mut mouse_motion: MessageReader<MouseMotion>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    mut player: Query<(&mut PlayerRuntime, &mut Transform), With<LocalPlayer>>,
) {
    let Ok(cursor) = cursor.single() else {
        return;
    };
    if cursor.grab_mode == CursorGrabMode::None {
        return;
    }
    let Ok((mut runtime, mut transform)) = player.single_mut() else {
        return;
    };

    let mut delta = Vec2::ZERO;
    for motion in mouse_motion.read() {
        delta += motion.delta;
    }
    if delta == Vec2::ZERO {
        return;
    }

    let limit = std::f32::consts::FRAC_PI_2 - 0.001;
    runtime.state.yaw -= delta.x * runtime.sensitivity;
    runtime.state.pitch =
        (runtime.state.pitch - delta.y * runtime.sensitivity).clamp(-limit, limit);

    let (sin_yaw, cos_yaw) = runtime.state.yaw.sin_cos();
    let (sin_pitch, cos_pitch) = runtime.state.pitch.sin_cos();
    let forward = Vec3::new(-sin_yaw * cos_pitch, sin_pitch, -cos_yaw * cos_pitch);
    transform.translation = runtime.state.eye_position();
    transform.look_to(forward, Vec3::Y);
}

/// Reads movement keys and advances the player with the shared physics.
fn move_player(
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    time: Res<Time>,
    physics: Res<PlayerPhysics>,
    mut player: Query<(&mut PlayerRuntime, &mut Transform), With<LocalPlayer>>,
    chunks: Res<ClientChunks>,
    solid: Option<Res<crate::render::chunks::RenderAssets>>,
) {
    let Ok((mut runtime, mut transform)) = player.single_mut() else {
        return;
    };
    let Ok(cursor) = cursor.single() else {
        return;
    };
    let active = cursor.grab_mode != CursorGrabMode::None;

    let mut dir = [0.0f32, 0.0f32];
    if active {
        if keys.pressed(KeyCode::KeyW) {
            dir[1] += 1.0;
        }
        if keys.pressed(KeyCode::KeyS) {
            dir[1] -= 1.0;
        }
        if keys.pressed(KeyCode::KeyD) {
            dir[0] += 1.0;
        }
        if keys.pressed(KeyCode::KeyA) {
            dir[0] -= 1.0;
        }
    }

    let input = PlayerInput {
        seq: 0,
        dir,
        jump: active && keys.pressed(KeyCode::Space),
        sprint: active && (keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight)),
        crouch: active && (keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::KeyC)),
        yaw: runtime.state.yaw,
        pitch: runtime.state.pitch,
    };

    let world = ChunkWorld {
        chunks: &chunks,
        solid: solid.as_deref(),
    };

    // Step at a fixed rate by subdividing the frame time, so movement matches
    // the server's fixed timestep regardless of frame rate.
    let dt = 1.0 / TICK_HZ as f32;
    let mut remaining = time.delta_secs().min(0.25);
    while remaining > 0.0 {
        let step = remaining.min(dt);
        step_player(&mut runtime.state, &input, &physics, step, &world);
        remaining -= step;
    }

    transform.translation = runtime.state.eye_position();
    let dir_vec = runtime.state.look_direction();
    transform.look_to(dir_vec, Vec3::Y);
}

/// A [`BlockSolid`] implementation backed by the client's loaded chunks.
struct ChunkWorld<'a> {
    chunks: &'a ClientChunks,
    solid: Option<&'a crate::render::chunks::RenderAssets>,
}

impl BlockSolid for ChunkWorld<'_> {
    fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        let pos = BlockPos::new(x, y, z);
        let Some(chunk) = self.chunks.chunks.get(&pos.chunk()) else {
            // Unloaded chunks are solid so the player cannot fall out of the
            // world while streaming in.
            return true;
        };
        let block = chunk.get(pos);
        match self.solid {
            Some(assets) => assets.solid.is_opaque(block),
            None => !block.is_air(),
        }
    }
}

/// Raycasts from the eye to find the targeted block.
fn update_target(
    mut target: ResMut<TargetedBlock>,
    player: Query<&PlayerRuntime, With<LocalPlayer>>,
    chunks: Res<ClientChunks>,
    solid: Option<Res<crate::render::chunks::RenderAssets>>,
) {
    let Ok(runtime) = player.single() else {
        return;
    };
    let eye = runtime.state.eye_position();
    let direction = runtime.state.look_direction();
    let tool = MiningTool::HANDS;
    let reach = if tool.area.0 > 1 { REACH_DRILL } else { REACH };

    struct ChunkFilter<'a> {
        chunks: &'a ClientChunks,
        solid: Option<&'a crate::render::chunks::RenderAssets>,
    }
    impl RayFilter for ChunkFilter<'_> {
        fn blocks(&self, x: i32, y: i32, z: i32) -> bool {
            let pos = BlockPos::new(x, y, z);
            let Some(chunk) = self.chunks.chunks.get(&pos.chunk()) else {
                return false;
            };
            let block = chunk.get(pos);
            if block.is_air() {
                return false;
            }
            match self.solid {
                Some(assets) => assets.solid.is_opaque(block),
                None => true,
            }
        }
    }

    let filter = ChunkFilter {
        chunks: &chunks,
        solid: solid.as_deref(),
    };
    target.0 = raycast(eye, direction, reach, &filter);
}

/// Handles the mining button: starts, progresses and completes a block break.
fn handle_mining(
    mouse: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    target: Res<TargetedBlock>,
    registry: Res<ContentRegistry>,
    mut feedback: ResMut<MiningFeedback>,
    mut player: Query<&mut PlayerRuntime, With<LocalPlayer>>,
    mut chunks: ResMut<ClientChunks>,
) {
    let Ok(mut runtime) = player.single_mut() else {
        return;
    };

    // Resolve the held tool from the registry.
    let tool = runtime
        .held
        .and_then(|item| MiningTool::from_item(registry.0.item_info(item)))
        .unwrap_or(MiningTool::HANDS);

    if !mouse.pressed(MouseButton::Left) {
        runtime.mining = None;
        return;
    }

    let Some(hit) = target.0 else {
        runtime.mining = None;
        return;
    };

    // (Re)start mining if the target changed.
    let start_new = match runtime.mining {
        Some(progress) => progress.block != hit.block,
        None => true,
    };
    if start_new {
        let info = registry
            .0
            .block_info(chunks.block_at(hit.block).unwrap_or(BlockId::AIR));
        match check_mine(
            chunks.block_at(hit.block).unwrap_or(BlockId::AIR),
            info,
            tool,
            1.0,
            TICK_HZ,
        ) {
            Ok(check) => {
                runtime.mining = Some(MiningProgress {
                    block: hit.block,
                    total_ticks: check.ticks,
                    elapsed_ticks: 0,
                });
                feedback.message.clear();
            }
            Err(denied) => {
                runtime.mining = None;
                if denied != MineDenied::Air {
                    feedback.message = denied.message();
                    feedback.timer = 1.5;
                }
                return;
            }
        }
    }

    // Advance the timer. This is the same arithmetic the server will run.
    let Some(mut progress) = runtime.mining else {
        return;
    };
    progress.elapsed_ticks += (time.delta_secs() * TICK_HZ as f32).round().max(1.0) as u32;

    if progress.elapsed_ticks >= progress.total_ticks {
        // Break the block (and the full face for area tools).
        let offsets = area_offsets(tool, hit.face);
        for (dx, dy, dz) in offsets {
            let pos = hit.block.offset(dx, dy, dz);
            if chunks.block_at(pos).map(|b| !b.is_air()).unwrap_or(false) {
                chunks.set_block(pos, BlockId::AIR);
            }
        }
        runtime.mining = None;
    } else {
        runtime.mining = Some(progress);
    }
}

/// Right-click places the held block against the targeted face.
fn handle_placing(
    mouse: Res<ButtonInput<MouseButton>>,
    target: Res<TargetedBlock>,
    registry: Res<ContentRegistry>,
    mut player: Query<&mut PlayerRuntime, With<LocalPlayer>>,
    mut chunks: ResMut<ClientChunks>,
) {
    if !mouse.just_pressed(MouseButton::Right) {
        return;
    }
    let Ok(mut runtime) = player.single_mut() else {
        return;
    };
    let Some(hit) = target.0 else {
        return;
    };
    let Some((item, count)) = runtime.hotbar[runtime.selected_slot] else {
        return;
    };
    let Some(block) = crate::render::chunks::item_to_block(&registry, item) else {
        return;
    };
    if !registry.0.block_info(block).placeable {
        return;
    }

    let (nx, ny, nz) = hit.face.normal();
    let place_at = hit.block.offset(nx, ny, nz);

    // Do not place a block inside the player.
    let aabb = runtime.state.aabb();
    if aabb.intersects_block(place_at.x, place_at.y, place_at.z) {
        return;
    }
    if chunks
        .block_at(place_at)
        .map(|b| !b.is_air())
        .unwrap_or(false)
    {
        return;
    }
    if chunks.set_block(place_at, block) {
        let remaining = count.saturating_sub(1);
        let slot = runtime.selected_slot;
        runtime.hotbar[slot] = if remaining == 0 {
            None
        } else {
            Some((item, remaining))
        };
        if remaining == 0 {
            runtime.held = None;
        }
    }
}

/// Number keys select a hotbar slot.
fn select_hotbar(
    keys: Res<ButtonInput<KeyCode>>,
    mut player: Query<&mut PlayerRuntime, With<LocalPlayer>>,
) {
    let Ok(mut runtime) = player.single_mut() else {
        return;
    };
    const NUMBERS: [KeyCode; 9] = [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    for (index, key) in NUMBERS.iter().enumerate() {
        if keys.just_pressed(*key) {
            runtime.selected_slot = index;
            runtime.held = runtime.hotbar[index].map(|(item, _)| item);
            runtime.mining = None;
        }
    }
}

/// Fades the mining feedback message out.
fn decay_feedback(time: Res<Time>, mut feedback: ResMut<MiningFeedback>) {
    if feedback.timer > 0.0 {
        feedback.timer -= time.delta_secs();
        if feedback.timer <= 0.0 {
            feedback.message.clear();
        }
    }
}
