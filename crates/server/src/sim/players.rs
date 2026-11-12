//! Spawning and driving player avatars on the server.
//!
//! When a client is accepted the server creates an avatar entity for it and
//! starts replicating the transform. Movement inputs and mining intents are
//! applied here, with every intent validated first.

use bevy::prelude::*;
use bevy_replicon::prelude::{ClientId, FromClient, Replicated, SendTargets, ToClients};

use deepcore_shared::content::ContentRegistry;
use deepcore_shared::movement::{PlayerInput, PlayerPhysics, PlayerState, step_player};
use deepcore_shared::protocol::{
    MiningRefused, MoveCorrection, MoveInput, PlaceBlock, PlayerInfo, PlayerTransform, StartMining,
    StopMining, Vitals,
};
use deepcore_shared::voxel::BlockPos;
use deepcore_shared::worldgen::WorldGen;

use crate::PendingBlockChanges;
use crate::net::ClientRegistry;
use crate::sim::mining::{MiningTimers, WorldBlocks, try_start_mining};
use crate::sim::validate::{PlayerView, validate_placing};
use crate::world::ServerWorld;

/// Links a server-side avatar entity back to its client.
///
/// The client entity is what chunk streaming and per-client messages address, so
/// this is the join between "who is connected" and "where their body is".
#[derive(Component, Debug, Clone, Copy)]
pub struct ConnectedAvatar {
    /// The client entity that owns this avatar.
    pub client: Entity,
}

/// The player's last received input sequence, for reconciliation.
#[derive(Component, Debug, Clone, Copy, Default)]
pub struct InputSequence(pub u32);

/// The player's authoritative physics state on the server.
#[derive(Component, Debug, Clone, Copy)]
pub struct ServerPlayerState(pub PlayerState);

/// Spawns an avatar for each newly accepted client.
pub fn spawn_avatars(
    mut commands: Commands,
    mut registry: ResMut<ClientRegistry>,
    world_gen: Res<WorldGen>,
    mut world: ResMut<ServerWorld>,
    mut physics: ResMut<PlayerPhysics>,
) {
    for player in registry.clients.iter_mut() {
        if !player.accepted || player.avatar.is_some() {
            continue;
        }
        let spawn = find_spawn(&world_gen, player.player_id);
        // Make sure the ground the avatar stands on exists.
        crate::world::ensure_chunks_around(
            &mut world,
            &world_gen,
            deepcore_shared::voxel::ChunkPos::containing(BlockPos::new(
                spawn.x as i32,
                spawn.y as i32,
                spawn.z as i32,
            )),
            1,
            1,
        );

        let state = PlayerState {
            position: spawn,
            ..default()
        };
        let entity = commands
            .spawn((
                ConnectedAvatar {
                    client: player.entity,
                },
                PlayerInfo {
                    id: player.player_id,
                    name: player.name.clone(),
                },
                PlayerTransform {
                    position: spawn.to_array(),
                    yaw: 0.0,
                    pitch: 0.0,
                },
                Vitals::default(),
                InputSequence::default(),
                ServerPlayerState(state),
                Replicated,
            ))
            .id();

        player.avatar = Some(entity);
        info!(
            player_id = player.player_id,
            name = %player.name,
            spawn = ?spawn,
            "avatar spawned"
        );
        let _ = &mut physics;
    }
}

/// Finds a standing position at the camp, offset per player so they do not stack.
fn find_spawn(world_gen: &WorldGen, player_id: u64) -> Vec3 {
    let surface = WorldGen::surface_y();
    let angle = (player_id as f32) * 0.9;
    let radius = 1.5 + (player_id % 5) as f32 * 1.2;
    let x = (angle.cos() * radius).round() as i32;
    let z = (angle.sin() * radius).round() as i32;
    for y in (0..=surface + 2).rev() {
        if !world_gen.block_at(x, y, z).is_air() {
            return Vec3::new(x as f32 + 0.5, y as f32 + 1.0, z as f32 + 0.5);
        }
    }
    Vec3::new(x as f32 + 0.5, surface as f32 + 1.0, z as f32 + 0.5)
}

/// Applies movement inputs from clients, authoritatively.
pub fn apply_movement(
    mut inputs: MessageReader<FromClient<MoveInput>>,
    mut registry: ResMut<ClientRegistry>,
    mut avatars: Query<(
        &mut ServerPlayerState,
        &mut PlayerTransform,
        &mut InputSequence,
        &ConnectedAvatar,
    )>,
    world: Res<ServerWorld>,
    physics: Res<PlayerPhysics>,
    mut corrections: MessageWriter<ToClients<MoveCorrection>>,
) {
    let blocks = WorldBlocks { world: &world };

    // Index avatars by client entity for quick lookup.
    for FromClient { client_id, message } in inputs.read() {
        let Some(client_entity) = client_id.entity() else {
            continue;
        };
        let Some(player) = registry.by_entity_mut(client_entity) else {
            continue;
        };
        let Some(avatar) = player.avatar else {
            continue;
        };
        let Ok((mut state, mut transform, mut seq, _owner)) = avatars.get_mut(avatar) else {
            continue;
        };

        // Apply exactly the same deterministic step the client predicted.
        let input = PlayerInput {
            seq: message.seq,
            dir: message.dir,
            jump: message.jump,
            sprint: message.sprint,
            crouch: message.crouch,
            yaw: message.yaw,
            pitch: message.pitch,
        };
        step_player(
            &mut state.0,
            &input,
            &physics,
            1.0 / deepcore_shared::protocol::TICK_HZ as f32,
            &blocks,
        );

        seq.0 = message.seq;
        transform.position = state.0.position.to_array();
        transform.yaw = state.0.yaw;
        transform.pitch = state.0.pitch;

        // Send the authoritative result back for reconciliation.
        corrections.write(ToClients {
            targets: SendTargets::Single(ClientId::Client(client_entity)),
            message: MoveCorrection {
                last_processed_seq: message.seq,
                position: state.0.position.to_array(),
                velocity: state.0.velocity.to_array(),
                on_ground: state.0.on_ground,
            },
        });
    }
}

/// Handles mining start intents with full validation.
pub fn handle_start_mining(
    mut intents: MessageReader<FromClient<StartMining>>,
    registry: Res<ClientRegistry>,
    avatars: Query<(&ServerPlayerState, &ConnectedAvatar)>,
    world: Res<ServerWorld>,
    registry_content: Res<ContentRegistry>,
    mut timers: ResMut<MiningTimers>,
    mut refused: MessageWriter<ToClients<MiningRefused>>,
) {
    for FromClient { client_id, message } in intents.read() {
        let Some(client_entity) = client_id.entity() else {
            continue;
        };
        let Some(player) = registry.by_entity(client_entity) else {
            continue;
        };
        let Some(avatar) = player.avatar else {
            continue;
        };
        let Ok((state, _)) = avatars.get(avatar) else {
            continue;
        };

        let aabb = state.0.aabb();
        let view = PlayerView {
            eye: state.0.eye_position(),
            look: state.0.look_direction(),
            tool: deepcore_shared::mining::MiningTool::HANDS,
            hotbar: [None; 9],
            body_min: aabb.min(),
            body_max: aabb.max(),
        };

        if let Err(reason) = try_start_mining(
            player.player_id,
            &view,
            message,
            &registry_content,
            &world,
            &mut timers,
        ) && let Some(text) = reason.client_message()
        {
            refused.write(ToClients {
                targets: SendTargets::Single(ClientId::Client(client_entity)),
                message: MiningRefused {
                    block: message.block,
                    reason: text,
                },
            });
        }
    }
}

/// Handles stop-mining intents.
pub fn handle_stop_mining(
    mut intents: MessageReader<FromClient<StopMining>>,
    registry: Res<ClientRegistry>,
    mut timers: ResMut<MiningTimers>,
) {
    for FromClient { client_id, .. } in intents.read() {
        let Some(client_entity) = client_id.entity() else {
            continue;
        };
        if let Some(player) = registry.by_entity(client_entity) {
            timers.stop(player.player_id);
        }
    }
}

/// Handles place-block intents with full validation.
pub fn handle_place_block(
    mut intents: MessageReader<FromClient<PlaceBlock>>,
    registry: Res<ClientRegistry>,
    avatars: Query<(&ServerPlayerState, &ConnectedAvatar)>,
    mut world: ResMut<ServerWorld>,
    content: Res<ContentRegistry>,
    mut pending: ResMut<PendingBlockChanges>,
) {
    for FromClient { client_id, message } in intents.read() {
        let Some(client_entity) = client_id.entity() else {
            continue;
        };
        let Some(player) = registry.by_entity(client_entity) else {
            continue;
        };
        let Some(avatar) = player.avatar else {
            continue;
        };
        let Ok((state, _)) = avatars.get(avatar) else {
            continue;
        };

        let aabb = state.0.aabb();
        let view = PlayerView {
            eye: state.0.eye_position(),
            look: state.0.look_direction(),
            tool: deepcore_shared::mining::MiningTool::HANDS,
            hotbar: [None; 9],
            body_min: aabb.min(),
            body_max: aabb.max(),
        };

        let blocks = WorldBlocks { world: &world };
        match validate_placing(
            &view,
            message.target,
            message.face,
            message.item_slot,
            &content,
            &blocks,
        ) {
            Ok(destination) => {
                // The item in the claimed slot determines the block placed.
                let item_name = view
                    .hotbar
                    .get(message.item_slot as usize)
                    .and_then(|slot| *slot)
                    .and_then(|(item, _)| content.0.items.get(item as usize))
                    .map(|info| info.name.clone());
                let block = item_name.and_then(|name| content.0.block(&name));
                if let Some(block) = block
                    && world.set_block(destination, block)
                {
                    pending
                        .0
                        .changes
                        .push(deepcore_shared::protocol::BlockChange {
                            pos: destination,
                            block: block.0,
                        });
                }
            }
            Err(reason) => {
                warn!(
                    player_id = player.player_id,
                    tag = reason.tag(),
                    "rejected place intent"
                );
            }
        }
    }
}
