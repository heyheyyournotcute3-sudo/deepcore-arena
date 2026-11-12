//! Voxel storage: block ids, chunk/block position maths, and the chunk itself.
//!
//! Everything here is pure data with no rendering or networking. The client and
//! the server share it so that a block address means the same thing on both
//! sides.

mod chunk;
mod position;

pub use chunk::{Chunk, ChunkError};
pub use position::{BlockPos, CHUNK_SIZE, CHUNK_VOLUME, ChunkPos};

/// Compact numeric id for a block type.
///
/// Ids are assigned at content-load time from the order of `blocks.ron`.
/// [`BlockId::AIR`] is always `0` so that zero-initialised storage is empty.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct BlockId(pub u16);

impl BlockId {
    /// The empty block. Always id `0`.
    pub const AIR: BlockId = BlockId(0);

    /// Returns `true` if this is the empty block.
    #[inline]
    pub const fn is_air(self) -> bool {
        self.0 == Self::AIR.0
    }
}
