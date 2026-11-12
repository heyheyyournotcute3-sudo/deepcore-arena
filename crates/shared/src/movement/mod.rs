//! Pure, deterministic player movement and voxel collision.
//!
//! This module is shared by the client (for prediction) and the server (for
//! authority). It must stay free of randomness and of wall-clock time: callers
//! pass a fixed timestep. Two peers applying the same inputs to the same state
//! must reach bit-identical results.

mod collide;
mod player;

pub use collide::{Aabb, BlockSolid, SweepResult, sweep_aabb};
pub use player::{PlayerInput, PlayerPhysics, PlayerState, step_player};

/// Width and depth of the player's collision box, in blocks.
pub const PLAYER_WIDTH: f32 = 0.6;
/// Height of the player's collision box, in blocks.
pub const PLAYER_HEIGHT: f32 = 1.8;
/// Height when crouching.
pub const PLAYER_HEIGHT_CROUCH: f32 = 1.1;
/// Eye height above the feet when standing.
pub const EYE_HEIGHT: f32 = 1.62;
/// Eye height above the feet when crouching.
pub const EYE_HEIGHT_CROUCH: f32 = 0.95;
