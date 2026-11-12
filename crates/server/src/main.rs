//! Deepcore dedicated server.
//!
//! Headless, authoritative, no window and no renderer. The simulation runs at a
//! fixed 30 Hz in [`FixedUpdate`]; everything gameplay-related ticks there.

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use std::time::Duration;

/// Simulation rate. 30 Hz means a 33.3 ms budget per tick.
pub const TICK_HZ: u64 = 30;
/// Duration of a single fixed simulation tick.
pub const TICK: Duration = Duration::from_nanos(1_000_000_000 / TICK_HZ);

fn main() {
    App::new()
        .add_plugins(
            MinimalPlugins
                .set(ScheduleRunnerPlugin::run_loop(Duration::ZERO))
                .build()
                .add(bevy::log::LogPlugin::default()),
        )
        .insert_resource(Time::<Fixed>::from_duration(TICK))
        .add_plugins(deepcore_shared::SharedPlugin)
        .add_plugins(ServerPlugin)
        .run();
}

/// Core server plugin: fixed-rate simulation bookkeeping.
pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(TickCounter::default())
            .add_systems(FixedUpdate, count_ticks)
            .add_systems(Update, log_tick_rate);
    }
}

/// Monotonic count of fixed simulation ticks executed since startup.
#[derive(Resource, Default)]
pub struct TickCounter {
    /// Total ticks executed.
    pub ticks: u64,
}

/// Advances the tick counter once per fixed step.
fn count_ticks(mut counter: ResMut<TickCounter>) {
    counter.ticks = counter.ticks.wrapping_add(1);
}

/// Logs the observed tick rate once per second so operators can confirm the
/// server is keeping up with its 30 Hz budget.
fn log_tick_rate(counter: Res<TickCounter>, mut last: Local<Option<(std::time::Instant, u64)>>) {
    let now = std::time::Instant::now();
    match *last {
        Some((instant, ticks)) => {
            let elapsed = now.duration_since(instant).as_secs_f32();
            if elapsed >= 1.0 {
                let rate = (counter.ticks - ticks) as f32 / elapsed;
                info!(
                    ticks = counter.ticks,
                    tick_rate_hz = format_args!("{rate:.1}"),
                    "server heartbeat"
                );
                *last = Some((now, counter.ticks));
            }
        }
        None => *last = Some((now, counter.ticks)),
    }
}
