//! Axis-aligned bounding boxes and swept collision against the voxel grid.

use bevy::math::Vec3;

/// An axis-aligned box in world space, stored as a centre and half-extents.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// Centre of the box.
    pub center: Vec3,
    /// Half size on each axis.
    pub half: Vec3,
}

impl Aabb {
    /// Builds a box from a centre and half-extents.
    pub fn new(center: Vec3, half: Vec3) -> Self {
        Self { center, half }
    }

    /// Builds a box standing on `feet` with the given width and height.
    pub fn from_feet(feet: Vec3, width: f32, height: f32) -> Self {
        Self {
            center: Vec3::new(feet.x, feet.y + height * 0.5, feet.z),
            half: Vec3::new(width * 0.5, height * 0.5, width * 0.5),
        }
    }

    /// Minimum corner.
    pub fn min(&self) -> Vec3 {
        self.center - self.half
    }

    /// Maximum corner.
    pub fn max(&self) -> Vec3 {
        self.center + self.half
    }

    /// Whether this box overlaps the unit block at `(x, y, z)`.
    pub fn intersects_block(&self, x: i32, y: i32, z: i32) -> bool {
        let bmin = Vec3::new(x as f32, y as f32, z as f32);
        let bmax = bmin + Vec3::ONE;
        let min = self.min();
        let max = self.max();
        min.x < bmax.x
            && max.x > bmin.x
            && min.y < bmax.y
            && max.y > bmin.y
            && min.z < bmax.z
            && max.z > bmin.z
    }
}

/// Result of a swept move.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SweepResult {
    /// The translation that was actually applied.
    pub translation: Vec3,
    /// Whether the move hit something on the X axis.
    pub hit_x: bool,
    /// Whether the move hit something on the Y axis.
    pub hit_y: bool,
    /// Whether the move hit something on the Z axis.
    pub hit_z: bool,
}

impl SweepResult {
    /// Whether the player is standing on ground (blocked moving downward).
    pub fn on_ground(&self) -> bool {
        self.hit_y && self.translation.y >= 0.0
    }
}

/// Read-only access to block solidity, so collision can be tested without a
/// full world.
pub trait BlockSolid {
    /// Whether the block at a world position stops movement.
    fn is_solid(&self, x: i32, y: i32, z: i32) -> bool;
}

/// Moves an AABB by `delta`, resolving collisions one axis at a time.
///
/// Horizontal axes are each swept from the original position and then combined;
/// vertical is swept last from the combined horizontal result. This gives clean
/// wall sliding, correct landing on ledges, and prevents diagonal corner slips.
pub fn sweep_aabb(start: Aabb, delta: Vec3, world: &impl BlockSolid) -> SweepResult {
    // X from the start position.
    let dx = if delta.x == 0.0 {
        0.0
    } else {
        move_axis(start, Vec3::ZERO, 0, delta.x, world)
    };
    // Z from the start position too, so a corner cannot be slipped diagonally.
    let dz = if delta.z == 0.0 {
        0.0
    } else {
        move_axis(start, Vec3::ZERO, 2, delta.z, world)
    };
    // Y from the combined horizontal offset, so we land on ledges correctly.
    let horizontal = Vec3::new(dx, 0.0, dz);
    let dy = if delta.y == 0.0 {
        0.0
    } else {
        move_axis(start, horizontal, 1, delta.y, world)
    };

    SweepResult {
        translation: Vec3::new(dx, dy, dz),
        hit_x: (dx - delta.x).abs() > f32::EPSILON,
        hit_y: (dy - delta.y).abs() > f32::EPSILON,
        hit_z: (dz - delta.z).abs() > f32::EPSILON,
    }
}

/// Attempts to move along one component, returning how far the box actually moved.
///
/// The test is *continuous*: it samples the swept volume rather than only the
/// destination, so a fast-moving box cannot pass through a thin wall.
fn move_axis(
    start: Aabb,
    already: Vec3,
    component: usize,
    amount: f32,
    world: &impl BlockSolid,
) -> f32 {
    let steps = swept_steps(amount);
    let step = amount / steps as f32;
    let mut applied = 0.0f32;
    for _ in 0..steps {
        let next = applied + step;
        if collides_at(start, already, component, next, world) {
            // Binary-search the exact contact point within this substep so the
            // box ends flush against the surface instead of short of it.
            let mut lo = applied;
            let mut hi = next;
            for _ in 0..12 {
                let mid = (lo + hi) * 0.5;
                if collides_at(start, already, component, mid, world) {
                    hi = mid;
                } else {
                    lo = mid;
                }
            }
            return lo;
        }
        applied = next;
    }
    amount
}

/// How many samples to take across a sweep. One sample per half block keeps a
/// 0.6-wide player box from skipping a one-block wall.
fn swept_steps(amount: f32) -> usize {
    ((amount.abs() / 0.25).ceil() as usize).clamp(1, 4096)
}

/// Tests whether moving `amount` along `component` from the already-applied
/// offset causes a collision.
fn collides_at(
    start: Aabb,
    already: Vec3,
    component: usize,
    amount: f32,
    world: &impl BlockSolid,
) -> bool {
    let mut offset = already;
    offset[component] = amount;
    let candidate = Aabb::new(start.center + offset, start.half);

    // Small epsilon so resting exactly on a surface is not a collision.
    let eps = 1.0e-4;
    let min = candidate.min() + Vec3::splat(eps);
    let max = candidate.max() - Vec3::splat(eps);

    let x0 = min.x.floor() as i32;
    let x1 = max.x.floor() as i32;
    let y0 = min.y.floor() as i32;
    let y1 = max.y.floor() as i32;
    let z0 = min.z.floor() as i32;
    let z1 = max.z.floor() as i32;

    for y in y0..=y1 {
        for z in z0..=z1 {
            for x in x0..=x1 {
                if world.is_solid(x, y, z) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voxel::{BlockId, Chunk, ChunkPos};
    use std::collections::HashMap;

    /// A simple solidity source backed by a map of loaded chunks.
    ///
    /// Used by tests and by the server's authoritative collision queries.
    pub struct ChunkSolidity<'a> {
        /// Loaded chunks keyed by position.
        chunks: &'a HashMap<ChunkPos, Chunk>,
        /// Which block ids are solid.
        solid_blocks: &'a [bool],
    }

    impl BlockSolid for ChunkSolidity<'_> {
        fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
            // Outside the loaded world is treated as solid so players cannot fall
            // out of the map.
            let pos = crate::voxel::BlockPos::new(x, y, z);
            let Some(chunk) = self.chunks.get(&pos.chunk()) else {
                return true;
            };
            let block: BlockId = chunk.get(pos);
            self.solid_blocks
                .get(block.0 as usize)
                .copied()
                .unwrap_or(true)
        }
    }

    #[test]
    fn chunk_solidity_reads_blocks_from_loaded_chunks() {
        let mut chunks = HashMap::new();
        let mut chunk = Chunk::new(ChunkPos::new(0, 0, 0));
        chunk.set_local(1, 2, 3, BlockId(1));
        chunks.insert(ChunkPos::new(0, 0, 0), chunk);
        let solid = [false, true];
        let source = ChunkSolidity {
            chunks: &chunks,
            solid_blocks: &solid,
        };
        assert!(source.is_solid(1, 2, 3), "the placed block should be solid");
        assert!(!source.is_solid(2, 2, 3), "air should not be solid");
        // Unloaded chunks are treated as solid so players cannot fall out.
        assert!(source.is_solid(9999, 2, 3));
    }

    struct SolidAt(Vec<(i32, i32, i32)>);

    impl BlockSolid for SolidAt {
        fn is_solid(&self, x: i32, y: i32, z: i32) -> bool {
            self.0.contains(&(x, y, z))
        }
    }

    fn player_box(feet: Vec3) -> Aabb {
        Aabb::from_feet(feet, 0.6, 1.8)
    }

    #[test]
    fn free_move_is_not_blocked() {
        let world = SolidAt(vec![]);
        let start = player_box(Vec3::new(0.5, 10.0, 0.5));
        let result = sweep_aabb(start, Vec3::new(1.0, 0.0, 0.0), &world);
        assert!((result.translation.x - 1.0).abs() < 1e-3);
        assert!(!result.hit_x && !result.hit_y && !result.hit_z);
    }

    #[test]
    fn wall_stops_horizontal_movement() {
        // Wall at x = 2, spanning the player's full body height.
        let world = SolidAt(vec![(2, 10, 0), (2, 11, 0)]);
        let start = player_box(Vec3::new(0.5, 10.0, 0.5));
        let result = sweep_aabb(start, Vec3::new(3.0, 0.0, 0.0), &world);
        assert!(result.hit_x, "should have hit the wall");
        // Player half-width is 0.3, so it stops with its right edge at x=2.
        assert!(
            result.translation.x <= 2.0 - 0.3 - 0.5 + 1e-2,
            "moved too far: {}",
            result.translation.x
        );
    }

    #[test]
    fn floor_stops_falling_and_reports_ground() {
        // Floor at y = 9, player standing at y = 10.
        let world = SolidAt(vec![
            (0, 9, 0),
            (1, 9, 0),
            (-1, 9, 0),
            (0, 9, 1),
            (0, 9, -1),
        ]);
        let start = player_box(Vec3::new(0.5, 10.0, 0.5));
        let result = sweep_aabb(start, Vec3::new(0.0, -2.0, 0.0), &world);
        assert!(result.hit_y, "should have landed");
        assert!(result.translation.y > -1.0, "fell through the floor");
    }

    #[test]
    fn sliding_along_a_wall_keeps_forward_speed() {
        // Wall along z = 2, moving diagonally into it.
        let mut blocks = Vec::new();
        for x in -8..9 {
            blocks.push((x, 10, 1));
            blocks.push((x, 11, 1));
        }
        let world = SolidAt(blocks);
        let start = player_box(Vec3::new(0.5, 10.0, 0.5));
        let result = sweep_aabb(start, Vec3::new(1.0, 0.0, 1.0), &world);
        // Blocked in Z, but X should still make progress.
        assert!(result.hit_z, "z should be blocked");
        assert!(result.translation.x > 0.5, "should slide along the wall");
    }

    #[test]
    fn standing_exactly_on_a_surface_is_not_a_collision() {
        let world = SolidAt(vec![(0, 9, 0)]);
        // Feet exactly at y=10, floor block occupies 9..10.
        let start = player_box(Vec3::new(0.5, 10.0, 0.5));
        let result = sweep_aabb(start, Vec3::ZERO, &world);
        assert!(!result.hit_y);
    }

    #[test]
    fn box_intersection_is_exclusive_at_touching_faces() {
        let a = Aabb::new(Vec3::new(0.5, 0.5, 0.5), Vec3::splat(0.5));
        // Touching but not overlapping the block at x=1.
        assert!(!a.intersects_block(1, 0, 0));
        assert!(a.intersects_block(0, 0, 0));
    }

    #[test]
    fn movement_never_tunnels_through_a_thin_wall() {
        // A single-block wall with a large delta; the sweep must stop.
        let world = SolidAt(vec![(1, 10, 0), (1, 11, 0)]);
        let start = player_box(Vec3::new(0.5, 10.0, 0.5));
        let result = sweep_aabb(start, Vec3::new(10.0, 0.0, 0.0), &world);
        assert!(result.hit_x);
        assert!(result.translation.x < 1.0, "tunnelled through the wall");
    }
}
