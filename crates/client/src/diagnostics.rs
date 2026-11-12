//! Frame-time and throughput diagnostics.
//!
//! Measures what the performance budgets in the spec actually ask about: client
//! frame rate at the configured view radius, and chunk streaming progress. The
//! numbers are logged periodically and are also available as a resource so an
//! F3 overlay (M2) can draw them without re-measuring.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::render::chunks::{ClientChunks, VIEW_RADIUS_XZ};

/// How often to emit a performance summary line.
const REPORT_INTERVAL_SECS: f32 = 5.0;

/// Rolling frame-time statistics for the current reporting window.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct FrameStats {
    /// Frames observed in the current window.
    pub frames: u32,
    /// Sum of frame times in the window, in milliseconds.
    pub total_ms: f64,
    /// Worst single frame in the window, in milliseconds.
    pub worst_ms: f64,
    /// Best single frame in the window, in milliseconds.
    pub best_ms: f64,
    /// Average frames per second over the window.
    pub avg_fps: f64,
    /// Average frame time over the window, in milliseconds.
    pub avg_ms: f64,
}

/// Marker for the on-screen diagnostics readout.
#[derive(Component)]
pub struct PerfOverlay;

/// Installs frame timing and the periodic report.
pub struct DiagnosticsPlugin;

impl Plugin for DiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FrameTimeDiagnosticsPlugin::default())
            .init_resource::<FrameStats>()
            .add_systems(Startup, spawn_perf_overlay)
            .add_systems(Update, (accumulate_frame_time, report_frame_stats));
    }
}

/// Accumulates the current frame's duration into the window.
fn accumulate_frame_time(diagnostics: Res<DiagnosticsStore>, mut stats: ResMut<FrameStats>) {
    let Some(frame_time) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FRAME_TIME) else {
        return;
    };
    let Some(ms) = frame_time.value() else {
        return;
    };
    if stats.frames == 0 {
        stats.best_ms = ms;
        stats.worst_ms = ms;
    }
    stats.frames += 1;
    stats.total_ms += ms;
    stats.best_ms = stats.best_ms.min(ms);
    stats.worst_ms = stats.worst_ms.max(ms);
}

/// Emits a summary every few seconds and resets the window.
///
/// This is how the "60 FPS at view radius 6" target is made observable rather
/// than asserted. On a software renderer the numbers will be far below target,
/// which is the point: the measurement is honest about the hardware.
fn report_frame_stats(
    time: Res<Time>,
    mut stats: ResMut<FrameStats>,
    chunks: Res<ClientChunks>,
    mut overlay: Query<&mut Text, With<PerfOverlay>>,
    mut elapsed: Local<f32>,
) {
    *elapsed += time.delta_secs();

    // Keep the live readout current every frame.
    let live_fps = if stats.frames > 0 {
        stats.frames as f64 / (stats.total_ms / 1000.0).max(1e-6)
    } else {
        0.0
    };
    if let Ok(mut text) = overlay.single_mut() {
        **text = format!(
            "FPS {:.0}  ·  {:.1} ms  ·  chunks {}  ·  view r{}",
            live_fps,
            if stats.frames > 0 {
                stats.total_ms / stats.frames as f64
            } else {
                0.0
            },
            chunks.chunks.len(),
            VIEW_RADIUS_XZ
        );
    }

    if *elapsed < REPORT_INTERVAL_SECS {
        return;
    }
    *elapsed = 0.0;

    if stats.frames == 0 {
        return;
    }
    let avg_ms = stats.total_ms / stats.frames as f64;
    stats.avg_ms = avg_ms;
    stats.avg_fps = 1000.0 / avg_ms.max(1e-6);

    info!(
        fps = format_args!("{:.1}", stats.avg_fps),
        avg_ms = format_args!("{:.2}", avg_ms),
        worst_ms = format_args!("{:.2}", stats.worst_ms),
        best_ms = format_args!("{:.2}", stats.best_ms),
        frames = stats.frames,
        chunks_loaded = chunks.chunks.len(),
        view_radius_chunks = VIEW_RADIUS_XZ,
        "client performance"
    );

    // Reset the window, keeping the best/worst seeds for the next one.
    stats.frames = 0;
    stats.total_ms = 0.0;
    stats.best_ms = f64::MAX;
    stats.worst_ms = 0.0;
}

/// A small always-on readout, top-right. The full F3 overlay is M2.
fn spawn_perf_overlay(mut commands: Commands) {
    commands.spawn((
        PerfOverlay,
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::srgba(0.75, 0.85, 1.0, 0.75)),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(12.0),
            top: Val::Px(10.0),
            ..default()
        },
    ));
}
