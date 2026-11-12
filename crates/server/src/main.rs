//! Deepcore dedicated server.
//!
//! Headless, authoritative, no window and no renderer. The simulation runs at a
//! fixed 30 Hz in [`FixedUpdate`]; everything gameplay-related ticks there.
//!
//! Usage:
//! ```text
//! server [--port N] [--seed N] [--bots N] [--simulate-latency MS]
//!        [--simulate-loss PCT] [--max-ticks N]
//! ```

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;

mod config;
mod net;
mod sim;
mod world;

use bevy_replicon::prelude::ToClients;
use deepcore_shared::content::registry::Registry;
use deepcore_shared::content::{ContentRegistry, load_from_dir};
use deepcore_shared::protocol::{BlockChangeBatch, PROTOCOL_VERSION};
use deepcore_shared::worldgen::WorldGen;

use config::{ServerConfig, parse_args};
use net::{ContentHash, NetServerPlugin};
use world::ServerWorld;

/// Simulation rate. 30 Hz means a 33.3 ms budget per tick.
pub const TICK_HZ: u64 = 30;
/// Duration of a single fixed simulation tick.
pub const TICK: Duration = Duration::from_nanos(1_000_000_000 / TICK_HZ);

/// Monotonic count of fixed simulation ticks executed since startup.
#[derive(Resource, Default)]
pub struct TickCounter {
    /// Total ticks executed.
    pub ticks: u64,
}

/// Block changes produced during the current tick, flushed as one batch.
#[derive(Resource, Default)]
pub struct PendingBlockChanges(pub BlockChangeBatch);

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = match parse_args(&args) {
        Ok(config) => config,
        Err(message) => {
            // `--help` also lands here; it is informational, not an error.
            if message.starts_with("Deepcore server") {
                println!("{message}");
                return;
            }
            eprintln!("server: {message}");
            std::process::exit(2);
        }
    };

    let registry = match load_registry(config.data_dir.as_deref()) {
        Ok(registry) => registry,
        Err(message) => {
            eprintln!("server: {message}");
            std::process::exit(1);
        }
    };
    let content_hash = registry.content_hash();

    info!(
        protocol = PROTOCOL_VERSION,
        content_hash = format_args!("{content_hash:016x}"),
        "deepcore server starting"
    );

    let mut app = App::new();
    app.add_plugins(
        MinimalPlugins
            .set(ScheduleRunnerPlugin::run_loop(Duration::ZERO))
            .build()
            .add(bevy::log::LogPlugin::default())
            // Replicon is state-driven; MinimalPlugins does not include states.
            .add(bevy::state::app::StatesPlugin),
    )
    .insert_resource(Time::<Fixed>::from_duration(TICK))
    .insert_resource(WorldGen::new(config.seed, registry.clone()))
    .insert_resource(ContentRegistry(registry))
    .insert_resource(ContentHash(content_hash))
    .insert_resource(ServerWorld::default())
    .insert_resource(PendingBlockChanges::default())
    .insert_resource(sim::MiningTimers::default())
    .insert_resource(deepcore_shared::movement::PlayerPhysics::default())

    .insert_resource(TickCounter::default())
    .insert_resource(config.clone())
    .add_plugins(deepcore_shared::SharedPlugin)
    .add_plugins(NetServerPlugin)
    .add_systems(Startup, start_server)
    .add_systems(FixedUpdate, (count_ticks, sim::tick_mining).chain())
    .add_systems(
        Update,
        (
            sim::players::spawn_avatars,
            sim::players::apply_movement,
            sim::players::handle_start_mining,
            sim::players::handle_stop_mining,
            sim::players::handle_place_block,
        ),
    )
    // Chunk streaming runs on the fixed tick, not per frame: the per-tick budget
    // in `stream_chunks` is what keeps renet's reliable send buffer from
    // overflowing when a client first loads a large interest set.
    .add_systems(FixedUpdate, world::stream::stream_chunks)
    .add_systems(
        Update,
        (flush_block_changes, log_tick_rate, stop_after_budget),
    );

    if config.bots > 0 {
        app.add_plugins(bots::BotPlugin::new(config.bots));
    }

    app.run();
}

/// Loads and validates content, or returns a readable failure.
///
/// `override_dir` lets a dedicated server point at a different content set,
/// which is also how the "Content mismatch" rejection is tested.
fn load_registry(override_dir: Option<&str>) -> Result<Registry, String> {
    let dir = match override_dir {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::path::PathBuf::from(asset_dir()).join("data"),
    };
    let set = load_from_dir(&dir)
        .map_err(|err| format!("failed to load content from {}: {err}", dir.display()))?;
    Registry::from_content(&set).map_err(|err| format!("failed to resolve content: {err}"))
}

/// Finds the `assets` directory, preferring the workspace copy.
fn asset_dir() -> String {
    let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
    if dev.exists() {
        return dev.display().to_string();
    }
    "assets".to_string()
}

/// Starts the transport once the ECS is up.
fn start_server(
    mut commands: Commands,
    channels: Res<bevy_replicon::prelude::RepliconChannels>,
    config: Res<ServerConfig>,
) {
    match net::start_transport(&mut commands, &channels, &config) {
        Ok(addr) => {
            info!(
                addr = %addr,
                port = config.port,
                seed = config.seed,
                latency_ms = config.simulate_latency_ms,
                loss_percent = config.simulate_loss_percent,
                "listening"
            );
        }
        Err(err) => {
            error!("{err}");
            std::process::exit(1);
        }
    }
}

/// Advances the tick counter once per fixed step.
fn count_ticks(mut counter: ResMut<TickCounter>) {
    counter.ticks = counter.ticks.wrapping_add(1);
}

/// Sends any block changes accumulated this frame as one batched message.
fn flush_block_changes(
    mut out: bevy::prelude::MessageWriter<ToClients<BlockChangeBatch>>,
    mut pending: ResMut<PendingBlockChanges>,
) {
    if pending.0.changes.is_empty() {
        return;
    }
    let batch = std::mem::take(&mut pending.0);
    let count = batch.changes.len();
    out.write(bevy_replicon::prelude::ToClients {
        targets: bevy_replicon::prelude::SendTargets::All,
        message: batch,
    });
    debug!(changes = count, "broadcast block changes");
}

/// Logs the observed tick rate once per second.
fn log_tick_rate(
    counter: Res<TickCounter>,
    registry: Option<Res<net::ClientRegistry>>,
    world: Option<Res<ServerWorld>>,
    server: Option<Res<bevy_replicon_renet::RenetServer>>,
    mut last: Local<Option<(std::time::Instant, u64)>>,
) {
    let now = std::time::Instant::now();
    match *last {
        Some((instant, ticks)) => {
            let elapsed = now.duration_since(instant).as_secs_f32();
            if elapsed >= 1.0 {
                let rate = (counter.ticks - ticks) as f32 / elapsed;
                let players = registry.as_ref().map(|r| r.player_count()).unwrap_or(0);
                let connected = server
                    .as_ref()
                    .map(|s| net::connected_count(s))
                    .unwrap_or(0);
                let chunks = world.as_ref().map(|w| w.loaded_count()).unwrap_or(0);
                info!(
                    ticks = counter.ticks,
                    tick_rate_hz = format_args!("{rate:.1}"),
                    players,
                    connected,
                    chunks,
                    "server heartbeat"
                );
                *last = Some((now, counter.ticks));
            }
        }
        None => *last = Some((now, counter.ticks)),
    }
}

/// Exits once `--max-ticks` is reached, so automated runs terminate.
fn stop_after_budget(
    counter: Res<TickCounter>,
    config: Res<ServerConfig>,
    mut exit: MessageWriter<AppExit>,
) {
    if let Some(budget) = config.max_ticks
        && counter.ticks >= budget
    {
        info!(ticks = counter.ticks, "reached --max-ticks, shutting down");
        exit.write(AppExit::Success);
    }
}

mod bots;
