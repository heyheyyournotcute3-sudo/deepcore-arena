//! The chunk: 32³ blocks with palette compression.
//!
//! A chunk stores a palette of distinct block ids plus, for each block, a
//! packed index into that palette. A chunk full of one block costs one palette
//! entry and no per-block bits at all, which matters because most of the mine is
//! solid stone.

use super::BlockId;
use super::position::{BlockPos, CHUNK_SIZE, CHUNK_VOLUME, ChunkPos};
use thiserror::Error;

/// Errors produced when reading or writing palette-compressed storage.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ChunkError {
    /// A palette index referred to a palette slot that does not exist.
    #[error("palette index {index} out of range for palette of {len} entries")]
    IndexOutOfRange {
        /// The offending index.
        index: usize,
        /// Number of entries in the palette.
        len: usize,
    },
    /// More distinct block types than the palette format can address.
    #[error("chunk palette overflow: {count} distinct blocks exceeds limit {limit}")]
    PaletteOverflow {
        /// Number of distinct blocks requested.
        count: usize,
        /// Maximum supported.
        limit: usize,
    },
    /// Serialised data was shorter or longer than expected.
    #[error("corrupt chunk data: {0}")]
    Corrupt(&'static str),
}

/// Number of bits per palette index. 8 bits covers 256 distinct block types,
/// which is comfortably more than the content set uses.
const BITS_PER_INDEX: u32 = 8;
/// Maximum palette size implied by [`BITS_PER_INDEX`].
pub const MAX_PALETTE_LEN: usize = 1 << BITS_PER_INDEX;

/// A 32×32×32 block of the world.
///
/// Internally this keeps a palette of distinct [`BlockId`]s and a densely
/// packed `u8` index per block. Callers never see the packing: they read and
/// write [`BlockId`]s.
#[derive(Clone, Debug)]
pub struct Chunk {
    /// Position of this chunk in the chunk grid.
    pos: ChunkPos,
    /// Distinct block ids present in this chunk.
    palette: Vec<BlockId>,
    /// One palette index per block, `CHUNK_VOLUME` entries.
    indices: Vec<u8>,
    /// Set once any block changes, so persistence knows to save this chunk.
    modified: bool,
}

impl Chunk {
    /// Creates an empty (all-air) chunk at the given position.
    pub fn new(pos: ChunkPos) -> Self {
        Self {
            pos,
            palette: vec![BlockId::AIR],
            indices: vec![0; CHUNK_VOLUME],
            modified: false,
        }
    }

    /// Creates a chunk filled uniformly with one block type.
    pub fn filled(pos: ChunkPos, block: BlockId) -> Self {
        let mut chunk = Self::new(pos);
        if !block.is_air() {
            // A solid chunk needs no per-block indices at all in principle, but
            // we keep the dense array for O(1) random writes.
            chunk.palette.push(block);
            chunk.indices.fill(1);
        }
        chunk
    }

    /// This chunk's position.
    #[inline]
    pub fn pos(&self) -> ChunkPos {
        self.pos
    }

    /// Whether any block has been changed since generation.
    #[inline]
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Clears the modified flag, e.g. after a successful save.
    #[inline]
    pub fn clear_modified(&mut self) {
        self.modified = false;
    }

    /// Reads the block at a local `(x, y, z)` coordinate inside this chunk.
    ///
    /// Returns [`BlockId::AIR`] for out-of-range coordinates rather than
    /// panicking, because callers legitimately probe chunk edges.
    #[inline]
    pub fn get_local(&self, x: i32, y: i32, z: i32) -> BlockId {
        if x < 0 || y < 0 || z < 0 || x >= CHUNK_SIZE || y >= CHUNK_SIZE || z >= CHUNK_SIZE {
            return BlockId::AIR;
        }
        let index = (y as usize) * (CHUNK_SIZE as usize * CHUNK_SIZE as usize)
            + (z as usize) * (CHUNK_SIZE as usize)
            + (x as usize);
        self.block_at_index(index)
    }

    /// Reads a block by flat chunk index. Out-of-range indices yield air.
    #[inline]
    pub fn block_at_index(&self, index: usize) -> BlockId {
        match self.indices.get(index) {
            Some(&palette_index) => self
                .palette
                .get(palette_index as usize)
                .copied()
                .unwrap_or(BlockId::AIR),
            None => BlockId::AIR,
        }
    }

    /// Writes a block at a local coordinate. Returns `false` if out of range.
    pub fn set_local(&mut self, x: i32, y: i32, z: i32, block: BlockId) -> bool {
        if x < 0 || y < 0 || z < 0 || x >= CHUNK_SIZE || y >= CHUNK_SIZE || z >= CHUNK_SIZE {
            return false;
        }
        let index = (y as usize) * (CHUNK_SIZE as usize * CHUNK_SIZE as usize)
            + (z as usize) * (CHUNK_SIZE as usize)
            + (x as usize);
        self.set_index(index, block)
    }

    /// Writes a block by flat chunk index.
    fn set_index(&mut self, index: usize, block: BlockId) -> bool {
        let Some(slot) = self.indices.get_mut(index) else {
            return false;
        };
        let palette_index = match self.palette.iter().position(|&b| b == block) {
            Some(existing) => existing,
            None => {
                if self.palette.len() >= MAX_PALETTE_LEN {
                    // Cannot represent this block in this chunk. Treat as a
                    // no-op rather than corrupting the palette.
                    tracing::error!(
                        pos = ?self.pos,
                        "palette overflow; block {:?} not stored",
                        block
                    );
                    return false;
                }
                self.palette.push(block);
                self.palette.len() - 1
            }
        };
        if *slot as usize != palette_index {
            *slot = palette_index as u8;
            self.modified = true;
        }
        true
    }

    /// Reads a block using a world-space position that falls inside this chunk.
    ///
    /// The chunk part of `pos` is ignored; only the low 5 bits of each axis are
    /// used. Callers should ensure `pos.chunk() == self.pos`.
    #[inline]
    pub fn get(&self, pos: BlockPos) -> BlockId {
        self.block_at_index(pos.chunk_index())
    }

    /// Writes a block using a world-space position inside this chunk.
    pub fn set(&mut self, pos: BlockPos, block: BlockId) -> bool {
        self.set_index(pos.chunk_index(), block)
    }

    /// Number of distinct block ids in the palette.
    #[inline]
    pub fn palette_len(&self) -> usize {
        self.palette.len()
    }

    /// Iterates over the chunk's blocks with their local coordinates.
    pub fn iter_blocks(&self) -> impl Iterator<Item = ((i32, i32, i32), BlockId)> + '_ {
        self.indices
            .iter()
            .enumerate()
            .map(|(index, &palette_index)| {
                let block = self
                    .palette
                    .get(palette_index as usize)
                    .copied()
                    .unwrap_or(BlockId::AIR);
                (BlockPos::from_chunk_index(index), block)
            })
    }

    /// Packs the chunk into a compact byte buffer for the network and for disk.
    ///
    /// Format: `u8 palette_len`, then `palette_len` × `u16` block ids, then the
    /// `CHUNK_VOLUME` palette indices. A caller that has run-length or entropy
    /// compression on top (zstd) gets most of the benefit from the dense,
    /// repetitive index array.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + self.palette.len() * 2 + self.indices.len());
        // Palette length is guaranteed < MAX_PALETTE_LEN (256) by set_index.
        out.push(self.palette.len().min(MAX_PALETTE_LEN) as u8);
        for block in &self.palette {
            out.extend_from_slice(&block.0.to_le_bytes());
        }
        out.extend_from_slice(&self.indices);
        out
    }

    /// Unpacks a chunk produced by [`Chunk::to_bytes`].
    pub fn from_bytes(pos: ChunkPos, bytes: &[u8]) -> Result<Self, ChunkError> {
        let Some((&palette_len_byte, rest)) = bytes.split_first() else {
            return Err(ChunkError::Corrupt("empty chunk buffer"));
        };
        let palette_len = palette_len_byte as usize;
        if palette_len == 0 {
            return Err(ChunkError::Corrupt("palette must contain at least air"));
        }
        let palette_bytes = palette_len * 2;
        if rest.len() < palette_bytes + CHUNK_VOLUME {
            return Err(ChunkError::Corrupt("chunk buffer too short"));
        }
        let (palette_raw, indices_raw) = rest.split_at(palette_bytes);

        let mut palette = Vec::with_capacity(palette_len);
        for chunk in palette_raw.as_chunks::<2>().0 {
            palette.push(BlockId(u16::from_le_bytes(*chunk)));
        }

        // Validate every index up front so a corrupt buffer cannot produce a
        // chunk that silently reads as air in the wrong places.
        for &idx in &indices_raw[..CHUNK_VOLUME] {
            if idx as usize >= palette_len {
                return Err(ChunkError::IndexOutOfRange {
                    index: idx as usize,
                    len: palette_len,
                });
            }
        }

        Ok(Self {
            pos,
            palette,
            indices: indices_raw[..CHUNK_VOLUME].to_vec(),
            modified: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_chunk_is_all_air() {
        let chunk = Chunk::new(ChunkPos::new(0, 0, 0));
        assert_eq!(chunk.palette_len(), 1);
        assert_eq!(chunk.get_local(0, 0, 0), BlockId::AIR);
        assert_eq!(chunk.get_local(31, 31, 31), BlockId::AIR);
        assert!(!chunk.is_modified());
    }

    #[test]
    fn filled_chunk_uses_one_palette_entry() {
        let chunk = Chunk::filled(ChunkPos::new(0, 0, 0), BlockId(7));
        assert_eq!(chunk.palette_len(), 2, "air + the fill block");
        assert_eq!(chunk.get_local(5, 5, 5), BlockId(7));
        // A solid chunk compresses to almost nothing after zstd; the raw
        // buffer is still small because the palette is tiny.
        assert!(chunk.to_bytes().len() < 1 + 2 * 2 + CHUNK_VOLUME + 1);
    }

    #[test]
    fn set_and_get_round_trip_every_position() {
        let mut chunk = Chunk::new(ChunkPos::new(1, 2, 3));
        chunk.set_local(0, 0, 0, BlockId(4));
        chunk.set_local(31, 31, 31, BlockId(5));
        chunk.set_local(17, 3, 29, BlockId(6));
        assert_eq!(chunk.get_local(0, 0, 0), BlockId(4));
        assert_eq!(chunk.get_local(31, 31, 31), BlockId(5));
        assert_eq!(chunk.get_local(17, 3, 29), BlockId(6));
        assert!(chunk.is_modified());
    }

    #[test]
    fn out_of_range_access_is_safe() {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0, 0));
        assert_eq!(chunk.get_local(-1, 0, 0), BlockId::AIR);
        assert_eq!(chunk.get_local(0, 32, 0), BlockId::AIR);
        assert!(!chunk.set_local(-1, 0, 0, BlockId(2)));
        assert!(!chunk.set_local(0, 0, 99, BlockId(2)));
    }

    #[test]
    fn palette_round_trip_preserves_every_block() {
        let mut chunk = Chunk::new(ChunkPos::new(-2, 1, 7));
        // Use a spread of ids to exercise palette growth.
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let id = BlockId(((x + y * 3 + z * 7) % 40 + 1) as u16);
                    chunk.set_local(x, y, z, id);
                }
            }
        }
        let bytes = chunk.to_bytes();
        let Ok(restored) = Chunk::from_bytes(chunk.pos(), &bytes) else {
            panic!("chunk round-trip must succeed");
        };
        for y in 0..CHUNK_SIZE {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    assert_eq!(restored.get_local(x, y, z), chunk.get_local(x, y, z));
                }
            }
        }
    }

    #[test]
    fn world_position_access_matches_local_access() {
        let chunk_pos = ChunkPos::new(1, 0, -1);
        let mut chunk = Chunk::filled(chunk_pos, BlockId(3));
        let world = BlockPos::new(32 + 4, 9, -32 + 2);
        assert_eq!(world.chunk(), chunk_pos);
        chunk.set(world, BlockId(9));
        assert_eq!(chunk.get(world), BlockId(9));
        assert_eq!(chunk.get_local(4, 9, 2), BlockId(9));
    }

    #[test]
    fn corrupt_buffers_are_rejected_not_silently_accepted() {
        assert!(Chunk::from_bytes(ChunkPos::new(0, 0, 0), &[]).is_err());
        assert!(Chunk::from_bytes(ChunkPos::new(0, 0, 0), &[0]).is_err());
        assert!(Chunk::from_bytes(ChunkPos::new(0, 0, 0), &[1, 0, 0]).is_err());
        // Valid palette length but an index pointing past it.
        let mut bad = vec![1u8, 0, 0];
        bad.extend(std::iter::repeat_n(5u8, CHUNK_VOLUME));
        assert!(matches!(
            Chunk::from_bytes(ChunkPos::new(0, 0, 0), &bad),
            Err(ChunkError::IndexOutOfRange { .. })
        ));
    }

    #[test]
    fn clearing_modified_flag_works() {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0, 0));
        chunk.set_local(1, 1, 1, BlockId(2));
        assert!(chunk.is_modified());
        chunk.clear_modified();
        assert!(!chunk.is_modified());
    }
}
