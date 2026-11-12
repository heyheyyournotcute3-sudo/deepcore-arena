//! Deepcore shared game logic: content registries, voxel world, worldgen,
//! movement, inventory, crafting, power, hazards and the network protocol.
//!
//! This crate contains no rendering and no sockets. It is compiled into both
//! the client and the server so that the same rules run on both sides.

use bevy::prelude::*;

/// The shared plugin, registered by both the client and the server.
pub struct SharedPlugin;

impl Plugin for SharedPlugin {
    fn build(&self, _app: &mut App) {}
}

/// Current network protocol version. Bumped whenever a replicated component
/// or protocol message changes shape.
pub mod content;
pub mod voxel;

pub const PROTOCOL_VERSION: u32 = 1;
