//! Deepcore client.
//!
//! Windowed first-person client. For M1 the client runs the world locally; M2
//! splits it into an authoritative server with streaming and prediction.

use bevy::prelude::*;
use bevy::window::WindowResolution;

mod diagnostics;
mod hud;
mod net;
mod player;
mod render;
mod session;
mod ui;

use deepcore_shared::content::registry::Registry;
use deepcore_shared::content::{ContentRegistry, load_from_dir};
use deepcore_shared::worldgen::WorldGen;

/// Default world seed used when none is given on the command line.
pub const DEFAULT_SEED: u64 = 2024;

/// netcode protocol id, shared with the server.
pub const NET_PROTOCOL_ID: u64 = 0x0DE9_C0DE;

/// Command-line options for the client.
#[derive(Debug, Clone, Default, bevy::prelude::Resource)]
pub struct ClientArgs {
    /// World seed for single-player.
    pub seed: u64,
    /// Address to join directly, skipping the menu.
    pub join: Option<String>,
    /// Display name.
    pub name: String,
    /// Run without a window (for automated tests).
    pub headless: bool,
    /// Exit after this many seconds (headless testing).
    pub run_secs: Option<u64>,
    /// Simulated latency in milliseconds.
    pub simulate_latency_ms: u32,
    /// Simulated packet loss percent.
    pub simulate_loss_percent: u32,
    /// Chunk view radius, overriding the default of 6.
    pub view_radius: Option<i32>,
}

fn parse_client_args() -> ClientArgs {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut parsed = ClientArgs {
        seed: DEFAULT_SEED,
        name: "Miner".to_string(),
        ..default()
    };
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--seed" => {
                if let Some(value) = args.get(index + 1).and_then(|v| v.parse().ok()) {
                    parsed.seed = value;
                }
                index += 2;
            }
            "--join" => {
                parsed.join = args.get(index + 1).cloned();
                index += 2;
            }
            "--name" => {
                if let Some(value) = args.get(index + 1) {
                    parsed.name = value.clone();
                }
                index += 2;
            }
            "--headless" => {
                parsed.headless = true;
                index += 1;
            }
            "--run-secs" => {
                parsed.run_secs = args.get(index + 1).and_then(|v| v.parse().ok());
                index += 2;
            }
            "--simulate-latency" => {
                if let Some(value) = args.get(index + 1).and_then(|v| v.parse().ok()) {
                    parsed.simulate_latency_ms = value;
                }
                index += 2;
            }
            "--view-radius" => {
                if let Some(value) = args.get(index + 1).and_then(|v| v.parse().ok()) {
                    parsed.view_radius = Some(value);
                }
                index += 2;
            }
            "--simulate-loss" => {
                if let Some(value) = args.get(index + 1).and_then(|v| v.parse().ok()) {
                    parsed.simulate_loss_percent = value;
                }
                index += 2;
            }
            _ => index += 1,
        }
    }
    parsed
}

fn main() {
    let args = parse_client_args();

    let registry = match load_registry() {
        Ok(registry) => registry,
        Err(message) => {
            eprintln!("Deepcore: {message}");
            std::process::exit(1);
        }
    };

    let mut app = App::new();

    if args.headless {
        // Headless mode exists so the network tests can run without a GPU.
        app.add_plugins(
            MinimalPlugins
                .build()
                .add(bevy::log::LogPlugin::default())
                .add(bevy::state::app::StatesPlugin),
        );
    } else {
        app.add_plugins(
            DefaultPlugins
                .set(AssetPlugin {
                    // Assets live at the workspace root, not next to the binary,
                    // so the game runs from `cargo run` and from a bundle alike.
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
        );
        app.insert_resource(ClearColor(Color::srgb(0.42, 0.56, 0.72)));
    }

    app.insert_resource(WorldGen::new(args.seed, registry.clone()))
        .insert_resource(ContentRegistry(registry))
        .insert_resource(args.clone())
        .add_plugins(deepcore_shared::SharedPlugin)
        .add_plugins(net::NetClientPlugin)
        .add_plugins(session::SessionPlugin);

    if let Some(radius) = args.view_radius {
        app.insert_resource(render::chunks::ViewRadius {
            xz: radius.clamp(1, 16),
            y: (radius / 2).clamp(1, 8),
        });
    }

    if !args.headless {
        app.add_plugins((
            render::RenderPlugin,
            player::PlayerPlugin,
            hud::HudPlugin,
            diagnostics::DiagnosticsPlugin,
            ui::menu::MenuPlugin,
        ));
    }

    // Joining directly skips the menu entirely.
    if let Some(address) = args.join.clone() {
        let name = args.name.clone();
        app.add_systems(
            Startup,
            move |mut start: MessageWriter<ui::menu::StartSession>| match net::parse_server_addr(
                &address,
                deepcore_shared::protocol::DEFAULT_PORT,
            ) {
                Ok(addr) => {
                    start.write(ui::menu::StartSession {
                        mode: net::SessionMode::Client {
                            addr,
                            name: name.clone(),
                        },
                        host_server: false,
                    });
                }
                Err(err) => error!(%err, "bad --join address"),
            },
        );
    }

    if let Some(secs) = args.run_secs {
        app.add_systems(
            Update,
            move |time: Res<Time>, mut exit: MessageWriter<AppExit>| {
                if time.elapsed_secs() >= secs as f32 {
                    info!(secs, "run duration reached, exiting");
                    exit.write(AppExit::Success);
                }
            },
        );
    }

    app.run();
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
