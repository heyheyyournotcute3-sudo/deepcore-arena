//! Voxel raycasting for block targeting and line-of-sight checks.
//!
//! Uses the Amanatides–Woo voxel traversal algorithm: an exact, branch-light
//! walk of the grid that visits every block a ray passes through. The client
//! uses it for the crosshair; the server uses the same code to validate reach
//! and line of sight, so the two cannot disagree about what is being aimed at.

use bevy::math::Vec3;

use super::Face;
use crate::voxel::BlockPos;

/// The block a ray hit, and which face it entered through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayHit {
    /// The block that was hit.
    pub block: BlockPos,
    /// The face the ray entered through.
    pub face: Face,
    /// Distance from the ray origin to the entry point, in blocks.
    pub distance: f32,
}

/// Callback deciding whether a block stops the ray.
pub trait RayFilter {
    /// Returns `true` if the block at this position should stop the ray.
    fn blocks(&self, x: i32, y: i32, z: i32) -> bool;
}

/// Casts a ray through the voxel grid and returns the first blocking block.
///
/// Returns `None` if nothing blocking is hit within `max_distance`.
pub fn raycast(
    origin: Vec3,
    direction: Vec3,
    max_distance: f32,
    filter: &impl RayFilter,
) -> Option<RayHit> {
    let dir = direction.normalize_or_zero();
    if dir == Vec3::ZERO || max_distance <= 0.0 {
        return None;
    }

    // Current voxel.
    let mut x = origin.x.floor() as i32;
    let mut y = origin.y.floor() as i32;
    let mut z = origin.z.floor() as i32;

    // Step direction per axis.
    let step_x = if dir.x > 0.0 { 1 } else { -1 };
    let step_y = if dir.y > 0.0 { 1 } else { -1 };
    let step_z = if dir.z > 0.0 { 1 } else { -1 };

    // Distance along the ray to the next voxel boundary on each axis.
    let t_delta_x = if dir.x.abs() > f32::EPSILON {
        (1.0 / dir.x).abs()
    } else {
        f32::INFINITY
    };
    let t_delta_y = if dir.y.abs() > f32::EPSILON {
        (1.0 / dir.y).abs()
    } else {
        f32::INFINITY
    };
    let t_delta_z = if dir.z.abs() > f32::EPSILON {
        (1.0 / dir.z).abs()
    } else {
        f32::INFINITY
    };

    let mut t_max_x = boundary_distance(origin.x, dir.x, step_x);
    let mut t_max_y = boundary_distance(origin.y, dir.y, step_y);
    let mut t_max_z = boundary_distance(origin.z, dir.z, step_z);

    // The origin block itself can block the ray (e.g. a player stuck in a wall).
    if filter.blocks(x, y, z) {
        return Some(RayHit {
            block: BlockPos::new(x, y, z),
            face: Face::from_normal(-dir),
            distance: 0.0,
        });
    }

    // Bounded so a bad input can never spin forever.
    let max_steps = (max_distance * 3.0).ceil() as usize + 3;
    for _ in 0..max_steps {
        // Advance along the axis with the nearest boundary.
        let (face, travelled) = if t_max_x <= t_max_y && t_max_x <= t_max_z {
            let t = t_max_x;
            x += step_x;
            t_max_x += t_delta_x;
            let face = if step_x > 0 { Face::West } else { Face::East };
            (face, t)
        } else if t_max_y <= t_max_z {
            let t = t_max_y;
            y += step_y;
            t_max_y += t_delta_y;
            let face = if step_y > 0 { Face::Down } else { Face::Up };
            (face, t)
        } else {
            let t = t_max_z;
            z += step_z;
            t_max_z += t_delta_z;
            let face = if step_z > 0 { Face::North } else { Face::South };
            (face, t)
        };

        if travelled > max_distance {
            return None;
        }
        if filter.blocks(x, y, z) {
            return Some(RayHit {
                block: BlockPos::new(x, y, z),
                face,
                distance: travelled,
            });
        }
    }
    None
}

/// Distance from `position` to the next integer boundary in the step direction.
fn boundary_distance(position: f32, direction: f32, step: i32) -> f32 {
    if direction.abs() <= f32::EPSILON {
        return f32::INFINITY;
    }
    let frac = position - position.floor();
    let to_boundary = if step > 0 { 1.0 - frac } else { frac };
    // Guard against a zero distance when exactly on a boundary.
    let to_boundary = if to_boundary <= f32::EPSILON {
        1.0
    } else {
        to_boundary
    };
    (to_boundary / direction.abs()).abs()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SolidAt(Vec<(i32, i32, i32)>);
    impl RayFilter for SolidAt {
        fn blocks(&self, x: i32, y: i32, z: i32) -> bool {
            self.0.contains(&(x, y, z))
        }
    }

    #[test]
    fn straight_ray_hits_the_expected_block_and_face() {
        let world = SolidAt(vec![(0, 0, -5)]);
        let hit = raycast(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(0.0, 0.0, -1.0),
            10.0,
            &world,
        )
        .expect("should hit the block");
        assert_eq!(hit.block, BlockPos::new(0, 0, -5));
        // Travelling toward -Z means we enter the block's +Z (south) face.
        assert_eq!(hit.face, Face::South);
    }

    #[test]
    fn ray_returns_none_when_nothing_is_in_range() {
        let world = SolidAt(vec![(0, 0, -50)]);
        assert!(
            raycast(
                Vec3::new(0.5, 0.5, 0.5),
                Vec3::new(0.0, 0.0, -1.0),
                10.0,
                &world
            )
            .is_none()
        );
    }

    #[test]
    fn ray_hits_the_nearest_block_not_a_further_one() {
        let world = SolidAt(vec![(0, 0, -3), (0, 0, -6)]);
        let hit = raycast(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(0.0, 0.0, -1.0),
            10.0,
            &world,
        )
        .expect("hit");
        assert_eq!(hit.block, BlockPos::new(0, 0, -3));
    }

    #[test]
    fn distance_is_accurate() {
        let world = SolidAt(vec![(0, 0, -5)]);
        let hit = raycast(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(0.0, 0.0, -1.0),
            10.0,
            &world,
        )
        .expect("hit");
        // Block z=-5 occupies -5..-4; entering its near face at z=-4 from z=0.5
        // is 4.5 blocks.
        assert!(
            (hit.distance - 4.5).abs() < 1e-3,
            "distance was {}",
            hit.distance
        );
    }

    #[test]
    fn vertical_ray_hits_up_face() {
        let world = SolidAt(vec![(0, 3, 0)]);
        let hit = raycast(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(0.0, 1.0, 0.0),
            10.0,
            &world,
        )
        .expect("hit");
        assert_eq!(hit.block, BlockPos::new(0, 3, 0));
        assert_eq!(
            hit.face,
            Face::Down,
            "looking up we enter the block's bottom"
        );
    }

    #[test]
    fn zero_direction_is_rejected() {
        let world = SolidAt(vec![(0, 0, 0)]);
        assert!(raycast(Vec3::ZERO, Vec3::ZERO, 10.0, &world).is_none());
    }

    #[test]
    fn ray_at_exact_boundary_does_not_hang_or_skip() {
        // Origin exactly on an integer boundary; the guard must keep it moving.
        let world = SolidAt(vec![(0, 0, -1)]);
        let hit = raycast(
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, -1.0),
            10.0,
            &world,
        );
        assert!(
            hit.is_some(),
            "ray from a boundary must still hit the next block"
        );
    }

    #[test]
    fn diagonal_ray_walks_correctly() {
        let world = SolidAt(vec![(3, 3, 3)]);
        let hit = raycast(
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(1.0, 1.0, 1.0),
            20.0,
            &world,
        );
        assert!(hit.is_some(), "diagonal ray should reach the block");
        assert_eq!(hit.expect("hit").block, BlockPos::new(3, 3, 3));
    }
}
