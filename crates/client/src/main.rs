//! Deepcore client.
//!
//! Windowed first-person client. For M1 the client runs the world locally; M2
//! splits it into an authoritative server with streaming and prediction.

use bevy::prelude::*;
use bevy::window::WindowResolution;

mod hud;
mod player;
mod render;

use deepcore_shared::content::registry::Registry;
use deepcore_shared::content::{ContentRegistry, load_from_dir};
use deepcore_shared::worldgen::WorldGen;

/// Default world seed used when none is given on the command line.
pub const DEFAULT_SEED: u64 = 2024;

fn main() {
    let seed = std::env::args()
        .skip_while(|a| a != "--seed")
        .nth(1)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_SEED);

    let registry = match load_registry() {
        Ok(registry) => registry,
        Err(message) => {
            eprintln!("Deepcore: {message}");
            std::process::exit(1);
        }
    };

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    // Assets live at the workspace root, not next to the binary, so the
                    // game runs correctly from `cargo run` and from a release bundle.
                    file_path: asset_root(),
                    ..default()
                })
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Deepcore".to_string(),
                        resolution: WindowResolution::new(1280, 720),
                        ..default()
                    }),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(Color::srgb(0.42, 0.56, 0.72)))
        .insert_resource(WorldGen::new(seed, registry.clone()))
        .insert_resource(ContentRegistry(registry))
        .add_plugins(deepcore_shared::SharedPlugin)
        .add_plugins((render::RenderPlugin, player::PlayerPlugin, hud::HudPlugin))
        .run();
}

/// Finds the `assets` directory.
///
/// Prefers `CARGO_MANIFEST_DIR/../../assets` during development so `cargo run`
/// works from anywhere, and falls back to `assets` relative to the working
/// directory for a packaged build.
fn asset_root() -> String {
    let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    if dev.exists() {
        return dev.display().to_string();
    }
    "assets".to_string()
}

/// Loads and validates the content files, turning any error into a readable
/// message. The client must fail loudly rather than run with broken content.
fn load_registry() -> Result<Registry, String> {
    let dir = std::path::PathBuf::from(asset_root()).join("data");
    let set = load_from_dir(&dir)
        .map_err(|err| format!("failed to load content from {}: {err}", dir.display()))?;
    Registry::from_content(&set).map_err(|err| format!("failed to resolve content: {err}"))
}
