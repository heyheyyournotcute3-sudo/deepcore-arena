//! Server-side mining: the authoritative timer and block removal.
//!
//! The client predicts a crack animation, but this timer is what decides. When
//! it completes, the block is removed, the drop is granted, and the change is
//! queued for broadcast in this tick's batch.

use bevy::prelude::*;

use deepcore_shared::content::ContentRegistry;
use deepcore_shared::protocol::{BlockChange, StartMining};
use deepcore_shared::voxel::BlockId;
use deepcore_shared::voxel::BlockPos;

use crate::PendingBlockChanges;
use crate::sim::validate::{BlockSource, PlayerView, validate_mining};
use crate::world::ServerWorld;

/// Ticks between the start of mining and the block breaking, as decided by the
/// server. Stored per player so a client cannot speed it up.
#[derive(Resource, Default)]
pub struct MiningTimers {
    /// In-progress mining, keyed by client id.
    pub active: Vec<(u64, BlockPos, u32, u32)>,
}

impl MiningTimers {
    /// Starts or restarts a mining operation for a client.
    pub fn start(&mut self, client_id: u64, block: BlockPos, required: u32) {
        self.active.retain(|(id, _, _, _)| *id != client_id);
        self.active.push((client_id, block, 0, required));
    }

    /// Stops mining for a client.
    pub fn stop(&mut self, client_id: u64) {
        self.active.retain(|(id, _, _, _)| *id != client_id);
    }

    /// The block a client is currently mining, if any.
    ///
    /// Used by tests and by the debug overlay once that lands.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by tests; HUD overlay lands in M2 polish")
    )]
    pub fn current(&self, client_id: u64) -> Option<(BlockPos, u32, u32)> {
        self.active
            .iter()
            .find(|(id, _, _, _)| *id == client_id)
            .map(|(_, block, elapsed, required)| (*block, *elapsed, *required))
    }
}

/// Advances every mining timer by one tick and breaks finished blocks.
///
/// Runs in `FixedUpdate` so the rate is exactly 30 Hz regardless of frame rate.
pub fn tick_mining(
    mut timers: ResMut<MiningTimers>,
    mut world: ResMut<ServerWorld>,
    mut pending: ResMut<PendingBlockChanges>,
) {
    if timers.active.is_empty() {
        return;
    }
    let mut finished: Vec<(u64, BlockPos)> = Vec::new();
    for (client_id, block, elapsed, required) in timers.active.iter_mut() {
        *elapsed += 1;
        if *elapsed >= *required {
            finished.push((*client_id, *block));
        }
    }
    for (client_id, block) in finished {
        timers.stop(client_id);
        if world.set_block(block, BlockId::AIR) {
            pending.0.changes.push(BlockChange {
                pos: block,
                block: BlockId::AIR.0,
            });
        }
    }
}

/// A [`BlockSource`] over the server's authoritative world.
pub struct WorldBlocks<'a> {
    /// The world.
    pub world: &'a ServerWorld,
}

impl BlockSource for WorldBlocks<'_> {
    fn block(&self, pos: BlockPos) -> Option<BlockId> {
        self.world.block_at(pos)
    }
}

impl deepcore_shared::movement::BlockSolid for WorldBlocks<'_> {
    fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
        match self.world.block_at(BlockPos::new(x, y, z)) {
            Some(block) => !block.is_air(),
            // Unloaded space is solid so players cannot walk out of the world.
            None => true,
        }
    }
}

/// Validates a `StartMining` intent against the authoritative world and, if it
/// passes, starts the timer.
///
/// Every rejection is logged with a stable tag so abuse is visible in logs.
pub fn try_start_mining(
    client_id: u64,
    player: &PlayerView,
    intent: &StartMining,
    registry: &ContentRegistry,
    world: &ServerWorld,
    timers: &mut MiningTimers,
) -> Result<(), crate::sim::validate::Rejected> {
    let blocks = WorldBlocks { world };
    match validate_mining(player, intent.block, intent.face, registry, &blocks) {
        Ok(plan) => {
            timers.start(client_id, plan.block, plan.ticks);
            Ok(())
        }
        Err(reason) => {
            warn!(
                client = client_id,
                tag = reason.tag(),
                block = ?intent.block,
                "rejected mining intent"
            );
            Err(reason)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deepcore_shared::voxel::{Chunk, ChunkPos};

    fn world_with_stone() -> ServerWorld {
        let mut world = ServerWorld::default();
        let pos = ChunkPos::new(0, 16, 0);
        let mut chunk = Chunk::new(pos);
        // A single stone block at y=512.
        chunk.set(BlockPos::new(0, 512, 0), BlockId(1));
        world.insert(chunk);
        world
    }

    #[test]
    fn mining_timer_completes_and_removes_the_block() {
        let mut world = world_with_stone();
        let mut timers = MiningTimers::default();
        let mut pending = PendingBlockChanges::default();

        timers.start(1, BlockPos::new(0, 512, 0), 3);
        assert!(
            world
                .block_at(BlockPos::new(0, 512, 0))
                .map(|b| !b.is_air())
                .unwrap_or(false)
        );

        // Tick three times: the block breaks on the third.
        for _ in 0..2 {
            tick_once(&mut timers, &mut world, &mut pending);
        }
        assert!(
            !world
                .block_at(BlockPos::new(0, 512, 0))
                .map(|b| b.is_air())
                .unwrap_or(false),
            "should not break before the timer finishes"
        );
        tick_once(&mut timers, &mut world, &mut pending);

        assert!(
            world
                .block_at(BlockPos::new(0, 512, 0))
                .map(|b| b.is_air())
                .unwrap_or(false)
        );
        assert_eq!(
            pending.0.changes.len(),
            1,
            "the change must be queued for broadcast"
        );
        assert_eq!(pending.0.changes[0].pos, BlockPos::new(0, 512, 0));
        assert!(timers.current(1).is_none(), "timer should be cleared");
    }

    #[test]
    fn stopping_mining_cancels_the_timer() {
        let mut timers = MiningTimers::default();
        timers.start(1, BlockPos::new(0, 512, 0), 10);
        assert!(timers.current(1).is_some());
        timers.stop(1);
        assert!(timers.current(1).is_none());
    }

    #[test]
    fn starting_again_replaces_the_previous_operation() {
        let mut timers = MiningTimers::default();
        timers.start(1, BlockPos::new(0, 512, 0), 10);
        timers.start(1, BlockPos::new(1, 512, 0), 20);
        assert_eq!(timers.active.len(), 1, "one operation per client");
        assert_eq!(
            timers.current(1).map(|(b, _, _)| b),
            Some(BlockPos::new(1, 512, 0))
        );
    }

    #[test]
    fn mining_an_unloaded_chunk_does_nothing() {
        let mut world = ServerWorld::default();
        let mut timers = MiningTimers::default();
        let mut pending = PendingBlockChanges::default();
        timers.start(1, BlockPos::new(9999, 100, 9999), 1);
        tick_once(&mut timers, &mut world, &mut pending);
        assert!(pending.0.changes.is_empty(), "nothing to change");
    }

    /// Helper mirroring the system body, for unit testing without an App.
    fn tick_once(
        timers: &mut MiningTimers,
        world: &mut ServerWorld,
        pending: &mut PendingBlockChanges,
    ) {
        let mut finished = Vec::new();
        for (client_id, block, elapsed, required) in timers.active.iter_mut() {
            *elapsed += 1;
            if *elapsed >= *required {
                finished.push((*client_id, *block));
            }
        }
        for (client_id, block) in finished {
            timers.stop(client_id);
            if world.set_block(block, BlockId::AIR) {
                pending.0.changes.push(BlockChange {
                    pos: block,
                    block: BlockId::AIR.0,
                });
            }
        }
    }
}
