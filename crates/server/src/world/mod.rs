//! Authoritative world state on the server.
//!
//! The server owns the only copy of the truth. This module holds the chunk map
//! and the block lookups that every validation path uses, so there is exactly
//! one place that can answer "what block is at this position".

use bevy::platform::collections::HashMap;
use bevy::prelude::*;

use deepcore_shared::voxel::{BlockId, BlockPos, Chunk, ChunkPos};
use deepcore_shared::worldgen::{WORLD_SIZE, WorldGen};

/// The server's authoritative chunk store.
#[derive(Resource)]
pub struct ServerWorld {
    /// Loaded chunks.
    chunks: HashMap<ChunkPos, Chunk>,
    /// Chunks that have been changed since generation and need saving.
    modified: std::collections::HashSet<ChunkPos>,
}

impl Default for ServerWorld {
    fn default() -> Self {
        Self {
            chunks: HashMap::new(),
            modified: std::collections::HashSet::new(),
        }
    }
}

impl ServerWorld {
    /// Inserts a chunk (generated or loaded from disk).
    pub fn insert(&mut self, chunk: Chunk) {
        self.chunks.insert(chunk.pos(), chunk);
    }

    /// Whether a chunk is loaded.
    pub fn contains(&self, pos: ChunkPos) -> bool {
        self.chunks.contains_key(&pos)
    }

    /// Reads a block. Returns `None` if the chunk is not loaded, which callers
    /// must treat as "unknown" rather than "air".
    pub fn block_at(&self, pos: BlockPos) -> Option<BlockId> {
        self.chunks.get(&pos.chunk()).map(|chunk| chunk.get(pos))
    }

    /// Writes a block, marking the chunk modified. Returns `false` if the chunk
    /// is not loaded.
    pub fn set_block(&mut self, pos: BlockPos, block: BlockId) -> bool {
        let Some(chunk) = self.chunks.get_mut(&pos.chunk()) else {
            return false;
        };
        let changed = chunk.set(pos, block);
        if changed {
            self.modified.insert(pos.chunk());
        }
        changed
    }

    /// Chunks that need saving.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "used by M9 region-file persistence")
    )]
    pub fn modified_chunks(&self) -> impl Iterator<Item = &ChunkPos> {
        self.modified.iter()
    }

    /// Number of loaded chunks.
    pub fn loaded_count(&self) -> usize {
        self.chunks.len()
    }

    /// A snapshot of a chunk for sending to a client.
    pub fn chunk_bytes(&self, pos: ChunkPos) -> Option<Vec<u8>> {
        self.chunks.get(&pos).map(|chunk| chunk.to_bytes())
    }
}

/// Generates the chunks a player needs, on the server.
///
/// Chunks are generated lazily as players join and move, so a server that nobody
/// has visited yet holds no chunks at all.
pub fn ensure_chunks_around(
    world: &mut ServerWorld,
    world_gen: &WorldGen,
    center: ChunkPos,
    radius_xz: i32,
    radius_y: i32,
) -> Vec<ChunkPos> {
    let mut generated = Vec::new();
    for dy in -radius_y..=radius_y {
        for dz in -radius_xz..=radius_xz {
            for dx in -radius_xz..=radius_xz {
                let pos = ChunkPos::new(center.x + dx, center.y + dy, center.z + dz);
                if !world.contains(pos) {
                    world.insert(world_gen.generate(pos));
                    generated.push(pos);
                }
            }
        }
    }
    generated
}

/// Which chunks a client at `center` should be sent, nearest first.
///
/// This is the interest-management rule: 6 chunks horizontally, 4 vertically.
pub fn interest_set(center: ChunkPos) -> Vec<ChunkPos> {
    let mut out = Vec::new();
    for dy in -deepcore_shared::protocol::VIEW_RADIUS_CHUNKS_Y
        ..=deepcore_shared::protocol::VIEW_RADIUS_CHUNKS_Y
    {
        for dz in -deepcore_shared::protocol::VIEW_RADIUS_CHUNKS_XZ
            ..=deepcore_shared::protocol::VIEW_RADIUS_CHUNKS_XZ
        {
            for dx in -deepcore_shared::protocol::VIEW_RADIUS_CHUNKS_XZ
                ..=deepcore_shared::protocol::VIEW_RADIUS_CHUNKS_XZ
            {
                out.push(ChunkPos::new(center.x + dx, center.y + dy, center.z + dz));
            }
        }
    }
    out.sort_by_key(|pos| {
        let dx = pos.x - center.x;
        let dy = pos.y - center.y;
        let dz = pos.z - center.z;
        dx * dx + dz * dz + dy * dy * 3
    });
    out
}

pub mod stream;

/// Whether a position is inside the mine's hard bounds.
///
/// Used by the intent validators once placing outside the world is a real case
/// (M3 adds building); it is exercised by tests today.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "exercised by tests; used by M3 building")
)]
pub fn in_world(pos: BlockPos) -> bool {
    let half = WORLD_SIZE / 2;
    pos.y >= 0
        && pos.y <= deepcore_shared::worldgen::WorldGen::surface_y()
        && pos.x > -half
        && pos.x < half
        && pos.z > -half
        && pos.z < half
}

#[cfg(test)]
mod tests {
    use super::*;
    use deepcore_shared::content::Registry;
    use std::path::Path;

    fn world_gen() -> WorldGen {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/data");
        let set = deepcore_shared::content::loader::load_from_dir(&dir).expect("content");
        let registry = Registry::from_content(&set).expect("resolves");
        WorldGen::new(2024, registry)
    }

    #[test]
    fn blocks_can_be_read_and_written() {
        let generator = world_gen();
        let mut world = ServerWorld::default();
        let pos = ChunkPos::new(0, 16, 0);
        world.insert(generator.generate(pos));

        let block_pos = BlockPos::new(3, 512, 4);
        let before = world.block_at(block_pos).expect("chunk loaded");
        assert!(!before.is_air());

        assert!(world.set_block(block_pos, BlockId::AIR));
        assert!(world.block_at(block_pos).expect("loaded").is_air());
        assert!(world.modified_chunks().any(|c| *c == pos));
    }

    #[test]
    fn unloaded_chunks_read_as_unknown_not_air() {
        let mut world = ServerWorld::default();
        assert_eq!(world.block_at(BlockPos::new(0, 100, 0)), None);
        assert!(!world.set_block(BlockPos::new(0, 100, 0), BlockId::AIR));
    }

    #[test]
    fn interest_set_is_bounded_and_nearest_first() {
        let center = ChunkPos::new(0, 16, 0);
        let set = interest_set(center);
        let expected = (6 * 2 + 1) * (6 * 2 + 1) * (4 * 2 + 1);
        assert_eq!(set.len(), expected);
        // The nearest chunk is the player's own.
        assert_eq!(set[0], center);
        // Horizontal extent is 6 chunks, vertical 4.
        assert!(set.iter().all(|p| (p.x - center.x).abs() <= 6));
        assert!(set.iter().all(|p| (p.y - center.y).abs() <= 4));
    }

    #[test]
    fn chunks_generate_lazily_around_a_centre() {
        let generator = world_gen();
        let mut world = ServerWorld::default();
        assert_eq!(world.loaded_count(), 0);
        let generated = ensure_chunks_around(&mut world, &generator, ChunkPos::new(0, 16, 0), 1, 1);
        assert_eq!(generated.len(), 27);
        assert_eq!(world.loaded_count(), 27);
        // Calling again generates nothing new.
        let again = ensure_chunks_around(&mut world, &generator, ChunkPos::new(0, 16, 0), 1, 1);
        assert!(again.is_empty());
    }

    #[test]
    fn world_bounds_reject_outside_positions() {
        assert!(in_world(BlockPos::new(0, 100, 0)));
        assert!(!in_world(BlockPos::new(0, -1, 0)));
        assert!(!in_world(BlockPos::new(0, 9999, 0)));
        assert!(!in_world(BlockPos::new(100_000, 100, 0)));
    }

    #[test]
    fn chunk_bytes_round_trip_through_the_wire_format() {
        let generator = world_gen();
        let mut world = ServerWorld::default();
        let pos = ChunkPos::new(0, 16, 0);
        world.insert(generator.generate(pos));
        let bytes = world.chunk_bytes(pos).expect("chunk exists");
        let restored = Chunk::from_bytes(pos, &bytes).expect("valid chunk bytes");
        assert_eq!(
            restored.get(BlockPos::new(0, 512, 0)),
            world.block_at(BlockPos::new(0, 512, 0)).expect("loaded")
        );
    }
}
