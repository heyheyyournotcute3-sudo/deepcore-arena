//! Chunk material: Bevy's `StandardMaterial` sampling the generated atlas.
//!
//! Earlier revisions tried a custom shader. `StandardMaterial` with an atlas
//! texture and vertex colours gives the same visual result — textured blocks,
//! real lighting and shadows — while depending only on Bevy's supported API.
//! That matters more than saving a few instructions per pixel.

use bevy::prelude::*;

/// Builds the material every chunk mesh uses.
///
/// `atlas` is the procedurally generated block texture; `uvs` maps each block to
/// its tile, which the mesher bakes into vertex UVs. Vertex colour carries the
/// per-face shade.
pub fn chunk_material(atlas: Handle<Image>) -> StandardMaterial {
    StandardMaterial {
        base_color_texture: Some(atlas),
        base_color: Color::WHITE,
        perceptual_roughness: 0.95,
        metallic: 0.0,
        // Keep chunks lit by the sun and ambient light; no emission by default.
        // Emissive ore blocks are handled by a separate overlay in M9 polish.
        ..default()
    }
}
