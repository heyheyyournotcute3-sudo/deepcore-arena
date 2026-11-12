//! Data-driven content: RON definitions, loaders, validation and registries.

pub mod defs;
pub mod loader;
pub mod registry;

pub use defs::*;
pub use loader::{ContentError, load_from_dir, load_from_strs, validate};
pub use registry::{
    BlockInfo, ItemId, ItemInfo, ItemStack, LayerId, MachineId, RecipeId, Registry, ResolvedLayer,
    ResolvedOre, ResolvedRecipe, ResolvedTech, TechId,
};

use bevy::prelude::*;

/// Bevy resource holding the resolved content database.
#[derive(Resource)]
pub struct ContentRegistry(pub Registry);

impl std::ops::Deref for ContentRegistry {
    type Target = Registry;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
