//! Chunk and block coordinate types and the conversions between them.

/// Side length of a chunk, in blocks. Chunks are cubes.
pub const CHUNK_SIZE: i32 = 32;

/// Number of blocks in a chunk.
pub const CHUNK_VOLUME: usize = (CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE) as usize;

/// Position of a chunk in the chunk grid. One unit is one whole chunk.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct ChunkPos {
    /// Chunk coordinate along X.
    pub x: i32,
    /// Chunk coordinate along Y (vertical).
    pub y: i32,
    /// Chunk coordinate along Z.
    pub z: i32,
}

impl ChunkPos {
    /// Creates a chunk position.
    #[inline]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// The chunk containing a world block position.
    ///
    /// Uses Euclidean division so that negative coordinates floor correctly
    /// instead of truncating toward zero.
    #[inline]
    pub const fn containing(block: BlockPos) -> Self {
        Self {
            x: block.x.div_euclid(CHUNK_SIZE),
            y: block.y.div_euclid(CHUNK_SIZE),
            z: block.z.div_euclid(CHUNK_SIZE),
        }
    }

    /// The lowest block position inside this chunk.
    #[inline]
    pub const fn origin(self) -> BlockPos {
        BlockPos::new(
            self.x * CHUNK_SIZE,
            self.y * CHUNK_SIZE,
            self.z * CHUNK_SIZE,
        )
    }

    /// Squared horizontal distance to another chunk, ignoring Y.
    #[inline]
    pub fn horizontal_distance_squared(self, other: ChunkPos) -> i32 {
        let dx = self.x - other.x;
        let dz = self.z - other.z;
        dx * dx + dz * dz
    }
}

/// Position of a single block in world space.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct BlockPos {
    /// Block coordinate along X.
    pub x: i32,
    /// Block coordinate along Y (vertical, up is positive).
    pub y: i32,
    /// Block coordinate along Z.
    pub z: i32,
}

impl BlockPos {
    /// Creates a block position.
    #[inline]
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Self { x, y, z }
    }

    /// Offsets this position by the given deltas.
    #[inline]
    pub const fn offset(self, dx: i32, dy: i32, dz: i32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.z + dz)
    }

    /// The chunk that contains this block.
    #[inline]
    pub const fn chunk(self) -> ChunkPos {
        ChunkPos::containing(self)
    }

    /// This block's index within its chunk's flat storage array.
    ///
    /// Layout is X fastest, then Z, then Y, which keeps vertical scans
    /// (common when meshing columns) reasonably cache friendly.
    #[inline]
    pub const fn chunk_index(self) -> usize {
        let lx = self.x.rem_euclid(CHUNK_SIZE) as usize;
        let ly = self.y.rem_euclid(CHUNK_SIZE) as usize;
        let lz = self.z.rem_euclid(CHUNK_SIZE) as usize;
        ly * (CHUNK_SIZE as usize * CHUNK_SIZE as usize) + lz * (CHUNK_SIZE as usize) + lx
    }

    /// Converts a flat chunk index back into a local `(x, y, z)` triple.
    #[inline]
    pub const fn from_chunk_index(index: usize) -> (i32, i32, i32) {
        let size = CHUNK_SIZE as usize;
        let x = (index % size) as i32;
        let z = ((index / size) % size) as i32;
        let y = (index / (size * size)) as i32;
        (x, y, z)
    }

    /// Squared distance to another block position.
    #[inline]
    pub fn distance_squared(self, other: BlockPos) -> i32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        let dz = self.z - other.z;
        dx * dx + dy * dy + dz * dz
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_coordinates_floor_into_the_right_chunk() {
        // -1 is in chunk -1, not chunk 0: truncating division would be a bug.
        assert_eq!(
            ChunkPos::containing(BlockPos::new(-1, -1, -1)),
            ChunkPos::new(-1, -1, -1)
        );
        assert_eq!(
            ChunkPos::containing(BlockPos::new(-33, 0, 0)),
            ChunkPos::new(-2, 0, 0)
        );
        assert_eq!(
            ChunkPos::containing(BlockPos::new(31, 31, 31)),
            ChunkPos::new(0, 0, 0)
        );
        assert_eq!(
            ChunkPos::containing(BlockPos::new(32, 32, 32)),
            ChunkPos::new(1, 1, 1)
        );
    }

    #[test]
    fn chunk_origin_is_lowest_corner() {
        assert_eq!(ChunkPos::new(2, 3, 4).origin(), BlockPos::new(64, 96, 128));
        assert_eq!(
            ChunkPos::new(-1, -1, -1).origin(),
            BlockPos::new(-32, -32, -32)
        );
    }

    #[test]
    fn chunk_index_round_trips_for_every_block() {
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let pos = BlockPos::new(x, y, z);
                    let idx = pos.chunk_index();
                    assert!(idx < CHUNK_VOLUME);
                    assert_eq!(BlockPos::from_chunk_index(idx), (x, y, z));
                }
            }
        }
    }

    #[test]
    fn chunk_index_is_unique() {
        // A bijection over the chunk volume: no two local positions collide.
        let mut seen = vec![false; CHUNK_VOLUME];
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let idx = BlockPos::new(x, y, z).chunk_index();
                    assert!(!seen[idx], "index {idx} used twice");
                    seen[idx] = true;
                }
            }
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn chunk_index_matches_chunk_for_world_coords() {
        let pos = BlockPos::new(-5, 70, 33);
        assert_eq!(pos.chunk(), ChunkPos::new(-1, 2, 1));
        // Local position inside that chunk is (27, 6, 1).
        assert_eq!(BlockPos::from_chunk_index(pos.chunk_index()), (27, 6, 1));
    }

    #[test]
    fn distance_squared_is_symmetric() {
        let a = BlockPos::new(1, 2, 3);
        let b = BlockPos::new(4, 6, 3);
        assert_eq!(a.distance_squared(b), 25);
        assert_eq!(a.distance_squared(b), b.distance_squared(a));
    }
}
