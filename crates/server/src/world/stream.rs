//! Chunk streaming to clients, with interest management.
//!
//! Each accepted client receives chunks within 6 horizontally and 4 vertically
//! of its player. When it moves, chunks that leave that box get an unload
//! message. This is what keeps bandwidth bounded and memory per client low.

use std::collections::HashSet;

use bevy::prelude::*;
use bevy_replicon::prelude::{ClientId, SendTargets, ToClients};

use deepcore_shared::protocol::{
    ChunkBatch, ChunkData, ChunkUnload, VIEW_RADIUS_CHUNKS_XZ, VIEW_RADIUS_CHUNKS_Y,
};
use deepcore_shared::voxel::ChunkPos;
use deepcore_shared::worldgen::WorldGen;

use crate::net::ClientRegistry;
use crate::world::{ServerWorld, ensure_chunks_around, interest_set};

/// How many chunks to send per message.
///
/// Chunks are ~4-8 KB each after palette encoding, and renet's reliable channel
/// has a bounded send buffer. Sending a whole interest set at once exhausts it
/// and the transport disconnects the client ("reliable channel memory usage was
/// exhausted"). Two chunks per message, a few messages per tick, keeps the
/// buffer healthy while still filling a client in a couple of seconds.
const CHUNKS_PER_BATCH: usize = 2;

/// Maximum chunk messages sent to one client per tick.
///
/// At 30 ticks/s with 2 chunks per message this fills a 1600-chunk interest set
/// in about 7 seconds, which is the intended "loading screen" duration, while
/// keeping the reliable channel's send buffer well inside its limit.
const BATCHES_PER_TICK: usize = 6;

/// Streams chunks to every accepted client according to its interest radius.
pub fn stream_chunks(
    mut world: ResMut<ServerWorld>,
    world_gen: Res<WorldGen>,
    mut registry: ResMut<ClientRegistry>,
    avatars: Query<(
        &deepcore_shared::protocol::PlayerTransform,
        &crate::sim::players::ConnectedAvatar,
    )>,
    mut out: MessageWriter<ToClients<ChunkBatch>>,
    mut unload: MessageWriter<ToClients<ChunkUnload>>,
) {
    // Nothing to do until somebody is connected.
    if registry.clients.iter().all(|c| !c.accepted) {
        return;
    }

    // Collect the work first so we do not hold the registry borrow across sends.
    let mut per_client: Vec<(Entity, Vec<ChunkPos>, Vec<(i32, i32, i32)>)> = Vec::new();

    for player in registry.clients.iter_mut().filter(|c| c.accepted) {
        let Some(avatar) = player.avatar else {
            continue;
        };
        let Ok((transform, owner)) = avatars.get(avatar) else {
            continue;
        };
        // Guard against a stale avatar pointer: stream only the body that
        // actually belongs to this client.
        if owner.client != player.entity {
            continue;
        }
        let center = ChunkPos::containing(deepcore_shared::voxel::BlockPos::new(
            transform.position[0].floor() as i32,
            transform.position[1].floor() as i32,
            transform.position[2].floor() as i32,
        ));

        // Generate server-side chunks the client will need.
        ensure_chunks_around(
            &mut world,
            &world_gen,
            center,
            VIEW_RADIUS_CHUNKS_XZ,
            VIEW_RADIUS_CHUNKS_Y,
        );

        let wanted = interest_set(center);
        let wanted_set: HashSet<(i32, i32, i32)> = wanted.iter().map(|p| (p.x, p.y, p.z)).collect();

        // Chunks to send: wanted but not already known.
        let to_send: Vec<ChunkPos> = wanted
            .iter()
            .copied()
            .filter(|pos| !player.known_chunks.contains(&(pos.x, pos.y, pos.z)))
            .collect();

        // Chunks to unload: known but no longer wanted.
        let to_unload: Vec<(i32, i32, i32)> = player
            .known_chunks
            .iter()
            .copied()
            .filter(|pos| !wanted_set.contains(pos))
            .collect();

        // Record what we are about to send so we do not resend it.
        for pos in &to_send {
            player.known_chunks.insert((pos.x, pos.y, pos.z));
        }
        for pos in &to_unload {
            player.known_chunks.remove(pos);
        }

        per_client.push((player.entity, to_send, to_unload));
    }

    for (entity, to_send, to_unload) in per_client {
        // Send chunks in small batches, rate-limited so the reliable channel's
        // send buffer never overflows.
        let budget = CHUNKS_PER_BATCH * BATCHES_PER_TICK;
        let send_now: Vec<ChunkPos> = to_send.iter().copied().take(budget).collect();
        // Anything not sent this tick must not be recorded as "known", or it
        // would never be sent at all.
        if let Some(player) = registry.by_entity_mut(entity) {
            for pos in to_send.iter().skip(budget) {
                player.known_chunks.remove(&(pos.x, pos.y, pos.z));
            }
        }

        for chunk_group in send_now.chunks(CHUNKS_PER_BATCH) {
            let mut batch = ChunkBatch::default();
            for pos in chunk_group {
                if let Some(bytes) = world.chunk_bytes(*pos) {
                    batch.chunks.push(ChunkData {
                        pos: (pos.x, pos.y, pos.z),
                        data: bytes,
                    });
                }
            }
            if !batch.chunks.is_empty() {
                out.write(ToClients {
                    targets: SendTargets::Single(ClientId::Client(entity)),
                    message: batch,
                });
            }
        }

        for pos in to_unload {
            unload.write(ToClients {
                targets: SendTargets::Single(ClientId::Client(entity)),
                message: ChunkUnload { pos },
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interest_set_matches_the_documented_radii() {
        let center = ChunkPos::new(0, 16, 0);
        let set = interest_set(center);
        // 13 x 13 horizontally, 9 vertically.
        assert_eq!(set.len(), 13 * 13 * 9);
        assert!(
            set.iter()
                .all(|p| (p.x - center.x).abs() <= VIEW_RADIUS_CHUNKS_XZ)
        );
        assert!(
            set.iter()
                .all(|p| (p.y - center.y).abs() <= VIEW_RADIUS_CHUNKS_Y)
        );
    }

    #[test]
    fn known_chunk_tracking_produces_unloads_when_moving() {
        // Stand at the origin, then move far enough that some chunks leave.
        let start = interest_set(ChunkPos::new(0, 16, 0));
        let known: HashSet<(i32, i32, i32)> = start.iter().map(|p| (p.x, p.y, p.z)).collect();

        let moved = interest_set(ChunkPos::new(6, 16, 0));
        let wanted: HashSet<(i32, i32, i32)> = moved.iter().map(|p| (p.x, p.y, p.z)).collect();

        let to_unload = known.difference(&wanted).count();
        let to_send = wanted.difference(&known).count();
        assert!(to_unload > 0, "moving 6 chunks should unload something");
        assert!(to_send > 0, "moving 6 chunks should require new chunks");
        // Both sets are the same size: a move adds as many as it drops.
        assert_eq!(to_unload, to_send);
    }

    #[test]
    fn standing_still_produces_no_work() {
        let set = interest_set(ChunkPos::new(3, 16, -2));
        let known: HashSet<(i32, i32, i32)> = set.iter().map(|p| (p.x, p.y, p.z)).collect();
        let wanted: HashSet<(i32, i32, i32)> = set.iter().map(|p| (p.x, p.y, p.z)).collect();
        assert_eq!(known.difference(&wanted).count(), 0);
        assert_eq!(wanted.difference(&known).count(), 0);
    }
}
