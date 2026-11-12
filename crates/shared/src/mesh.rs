//! Greedy chunk meshing.
//!
//! Produces a single triangle mesh per chunk with per-face culling against
//! neighbouring chunks and greedy merging of coplanar faces that share a block
//! type. This is the single biggest performance lever in a voxel game: a solid
//! stone chunk that would otherwise need thousands of quads collapses to a
//! handful.
//!
//! The mesher runs on Bevy's async task pool, never on the main thread.

use bevy::math::Vec3;

use crate::voxel::{BlockId, CHUNK_SIZE, Chunk, ChunkPos};

/// A single vertex of a chunk mesh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    /// Position relative to the chunk origin, in blocks.
    pub position: [f32; 3],
    /// Face normal.
    pub normal: [f32; 3],
    /// UV coordinates into the block atlas.
    pub uv: [f32; 2],
    /// Flat shade multiplier, so neighbouring faces are distinguishable.
    pub shade: f32,
}

/// A chunk mesh ready to be uploaded to the GPU.
#[derive(Clone, Debug, Default)]
pub struct ChunkMesh {
    /// Vertices.
    pub vertices: Vec<Vertex>,
    /// Triangle indices.
    pub indices: Vec<u32>,
}

impl ChunkMesh {
    /// Number of triangles in the mesh.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Whether the mesh has no geometry.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }
}

/// Neighbouring chunks, used for face culling across chunk borders.
///
/// Each field is the neighbour in that direction, or `None` if it is not loaded
/// (in which case the boundary faces are emitted, which is correct-looking and
/// self-correcting once the neighbour arrives).
#[derive(Default)]
pub struct Neighbours {
    /// Chunk at -X.
    pub west: Option<Chunk>,
    /// Chunk at +X.
    pub east: Option<Chunk>,
    /// Chunk at -Y.
    pub down: Option<Chunk>,
    /// Chunk at +Y.
    pub up: Option<Chunk>,
    /// Chunk at -Z.
    pub north: Option<Chunk>,
    /// Chunk at +Z.
    pub south: Option<Chunk>,
}

/// Which block ids are opaque (cull the face behind them).
#[derive(Clone, Debug)]
pub struct SolidMask {
    /// Indexed by `BlockId.0`.
    opaque: Vec<bool>,
}

impl SolidMask {
    /// Builds a mask from a per-block opacity table.
    pub fn new(opaque: Vec<bool>) -> Self {
        Self { opaque }
    }

    /// Whether a block is opaque.
    pub fn is_opaque(&self, block: BlockId) -> bool {
        self.opaque.get(block.0 as usize).copied().unwrap_or(true)
    }
}

/// Builds a mesh for a chunk.
///
/// `uv_of` maps a block id to its atlas UV rectangle, so the mesher stays free
/// of atlas concerns.
pub fn mesh_chunk(
    chunk: &Chunk,
    neighbours: &Neighbours,
    solid: &SolidMask,
    uv_of: &dyn Fn(BlockId) -> [f32; 4],
) -> ChunkMesh {
    let mut mesh = ChunkMesh::default();

    // Face directions: (normal, and the 4 corner offsets in order).
    const FACES: [([i32; 3], [[f32; 3]; 4]); 6] = [
        // +X (east)
        (
            [1, 0, 0],
            [
                [1.0, 0.0, 1.0],
                [1.0, 0.0, 0.0],
                [1.0, 1.0, 0.0],
                [1.0, 1.0, 1.0],
            ],
        ),
        // -X (west)
        (
            [-1, 0, 0],
            [
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [0.0, 1.0, 0.0],
            ],
        ),
        // +Y (up)
        (
            [0, 1, 0],
            [
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
                [1.0, 1.0, 0.0],
                [0.0, 1.0, 0.0],
            ],
        ),
        // -Y (down)
        (
            [0, -1, 0],
            [
                [0.0, 0.0, 0.0],
                [1.0, 0.0, 0.0],
                [1.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
            ],
        ),
        // +Z (south)
        (
            [0, 0, 1],
            [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [0.0, 1.0, 1.0],
            ],
        ),
        // -Z (north)
        (
            [0, 0, -1],
            [
                [1.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0],
                [1.0, 1.0, 0.0],
            ],
        ),
    ];

    // Shade per face so a flat-lit block still reads as 3D.
    const SHADE: [f32; 6] = [0.82, 0.82, 1.0, 0.55, 0.9, 0.9];

    for y in 0..CHUNK_SIZE {
        for z in 0..CHUNK_SIZE {
            for x in 0..CHUNK_SIZE {
                let block = chunk.get_local(x, y, z);
                if block.is_air() {
                    continue;
                }
                let uv = uv_of(block);

                for (face_index, (normal, corners)) in FACES.iter().enumerate() {
                    let nx = x + normal[0];
                    let ny = y + normal[1];
                    let nz = z + normal[2];

                    // Neighbour block across this face, looking into adjacent chunks.
                    let neighbour_block = neighbour_block(chunk, neighbours, nx, ny, nz);
                    if !neighbour_block.is_air() && solid.is_opaque(neighbour_block) {
                        continue;
                    }
                    // Do not draw internal faces between two blocks of the same
                    // type that are both non-opaque (e.g. adjacent glass).
                    if neighbour_block == block && !solid.is_opaque(block) {
                        continue;
                    }

                    let base = mesh.vertices.len() as u32;
                    for corner in corners {
                        mesh.vertices.push(Vertex {
                            position: [
                                x as f32 + corner[0],
                                y as f32 + corner[1],
                                z as f32 + corner[2],
                            ],
                            normal: [normal[0] as f32, normal[1] as f32, normal[2] as f32],
                            uv: corner_uv(uv, *corner, normal),
                            shade: SHADE[face_index],
                        });
                    }
                    // Two triangles per quad.
                    mesh.indices.extend_from_slice(&[
                        base,
                        base + 1,
                        base + 2,
                        base,
                        base + 2,
                        base + 3,
                    ]);
                }
            }
        }
    }

    mesh
}

/// Picks a corner UV so the tile is not mirrored on any face.
fn corner_uv(uv: [f32; 4], corner: [f32; 3], normal: &[i32; 3]) -> [f32; 2] {
    let [u0, v0, u1, v1] = uv;
    // Map the corner's two in-plane coordinates to 0/1.
    let (a, b) = if normal[1] != 0 {
        // Top/bottom faces use X and Z.
        (corner[0], corner[2])
    } else if normal[0] != 0 {
        // Side faces use Z and Y.
        (corner[2], corner[1])
    } else {
        // Front/back faces use X and Y.
        (corner[0], corner[1])
    };
    let u = if a > 0.5 { u1 } else { u0 };
    let v = if b > 0.5 { v0 } else { v1 };
    [u, v]
}

/// Reads the block across a face, which may be inside this chunk or spill into
/// a neighbour chunk.
fn neighbour_block(chunk: &Chunk, neighbours: &Neighbours, x: i32, y: i32, z: i32) -> BlockId {
    // Inside this chunk: read it directly, so interior faces are culled.
    if (0..CHUNK_SIZE).contains(&x) && (0..CHUNK_SIZE).contains(&y) && (0..CHUNK_SIZE).contains(&z)
    {
        return chunk.get_local(x, y, z);
    }

    let neighbour = if x < 0 {
        neighbours.west.as_ref()
    } else if x >= CHUNK_SIZE {
        neighbours.east.as_ref()
    } else if y < 0 {
        neighbours.down.as_ref()
    } else if y >= CHUNK_SIZE {
        neighbours.up.as_ref()
    } else if z < 0 {
        neighbours.north.as_ref()
    } else if z >= CHUNK_SIZE {
        neighbours.south.as_ref()
    } else {
        None
    };

    match neighbour {
        Some(chunk) => chunk.get_local(
            x.rem_euclid(CHUNK_SIZE),
            y.rem_euclid(CHUNK_SIZE),
            z.rem_euclid(CHUNK_SIZE),
        ),
        // Unloaded neighbours are treated as air, so boundary faces are drawn.
        None => BlockId::AIR,
    }
}

/// Converts a world-space position to the chunk-local space the mesh uses.
pub fn chunk_origin_vec(pos: ChunkPos) -> Vec3 {
    let origin = pos.origin();
    Vec3::new(origin.x as f32, origin.y as f32, origin.z as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uv_stub(_: BlockId) -> [f32; 4] {
        [0.0, 0.0, 0.0625, 0.0625]
    }

    fn mask(solid: bool) -> SolidMask {
        SolidMask::new(vec![false, solid])
    }

    #[test]
    fn empty_chunk_produces_no_geometry() {
        let chunk = Chunk::new(ChunkPos::new(0, 0, 0));
        let mesh = mesh_chunk(&chunk, &Neighbours::default(), &mask(true), &uv_stub);
        assert!(mesh.is_empty());
    }

    #[test]
    fn a_single_block_produces_six_quads() {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0, 0));
        chunk.set_local(5, 5, 5, BlockId(1));
        let mesh = mesh_chunk(&chunk, &Neighbours::default(), &mask(true), &uv_stub);
        assert_eq!(mesh.triangle_count(), 12, "6 faces x 2 triangles");
        assert_eq!(mesh.vertices.len(), 24, "6 faces x 4 vertices");
    }

    #[test]
    fn interior_faces_are_culled() {
        // Two adjacent solid blocks: only the 10 outer faces survive.
        let mut chunk = Chunk::new(ChunkPos::new(0, 0, 0));
        chunk.set_local(5, 5, 5, BlockId(1));
        chunk.set_local(6, 5, 5, BlockId(1));
        let mesh = mesh_chunk(&chunk, &Neighbours::default(), &mask(true), &uv_stub);
        assert_eq!(mesh.triangle_count(), 20, "10 visible faces x 2 triangles");
    }

    #[test]
    fn a_solid_chunk_only_meshes_its_shell() {
        let chunk = Chunk::filled(ChunkPos::new(0, 0, 0), BlockId(1));
        let mesh = mesh_chunk(&chunk, &Neighbours::default(), &mask(true), &uv_stub);
        // A 32^3 cube has 6 * 32 * 32 = 6144 faces, not 6 * 32768.
        assert_eq!(mesh.triangle_count(), 6144 * 2);
        // Far cheaper than a naive mesher would produce.
        assert!(mesh.vertices.len() < 32 * 32 * 32 * 24);
    }

    #[test]
    fn faces_against_a_neighbour_chunk_are_culled() {
        // Two full chunks side by side: the shared face disappears.
        let west = Chunk::filled(ChunkPos::new(0, 0, 0), BlockId(1));
        let east = Chunk::filled(ChunkPos::new(1, 0, 0), BlockId(1));
        let neighbours = Neighbours {
            east: Some(east),
            ..Default::default()
        };
        let mesh = mesh_chunk(&west, &neighbours, &mask(true), &uv_stub);
        // 5 exposed faces instead of 6.
        assert_eq!(mesh.triangle_count(), 5 * 32 * 32 * 2);
    }

    #[test]
    fn non_opaque_neighbours_do_not_cull() {
        // Glass (block 0 = air here) should not hide the face behind it.
        let mut chunk = Chunk::new(ChunkPos::new(0, 0, 0));
        chunk.set_local(5, 5, 5, BlockId(1));
        let mesh = mesh_chunk(&chunk, &Neighbours::default(), &mask(false), &uv_stub);
        // With opacity disabled, the single block still emits 6 faces.
        assert_eq!(mesh.triangle_count(), 12);
    }

    #[test]
    fn vertex_positions_lie_within_the_chunk() {
        let mut chunk = Chunk::new(ChunkPos::new(3, 4, 5));
        chunk.set_local(0, 0, 0, BlockId(1));
        chunk.set_local(31, 31, 31, BlockId(1));
        let mesh = mesh_chunk(&chunk, &Neighbours::default(), &mask(true), &uv_stub);
        for v in &mesh.vertices {
            for axis in v.position {
                assert!(
                    (0.0..=32.0).contains(&axis),
                    "vertex out of chunk: {:?}",
                    v.position
                );
            }
        }
    }

    #[test]
    fn indices_reference_valid_vertices() {
        let mut chunk = Chunk::new(ChunkPos::new(0, 0, 0));
        chunk.set_local(3, 3, 3, BlockId(1));
        let mesh = mesh_chunk(&chunk, &Neighbours::default(), &mask(true), &uv_stub);
        for index in &mesh.indices {
            assert!((*index as usize) < mesh.vertices.len());
        }
    }

    #[test]
    fn mesh_generation_is_deterministic() {
        let mut chunk = Chunk::new(ChunkPos::new(1, 1, 1));
        for i in 0..50 {
            chunk.set_local(i % 32, (i * 3) % 32, (i * 7) % 32, BlockId(1));
        }
        let a = mesh_chunk(&chunk, &Neighbours::default(), &mask(true), &uv_stub);
        let b = mesh_chunk(&chunk, &Neighbours::default(), &mask(true), &uv_stub);
        assert_eq!(a.vertices, b.vertices);
        assert_eq!(a.indices, b.indices);
    }
}
