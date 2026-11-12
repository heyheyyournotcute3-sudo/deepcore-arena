//! Rendering and interpolating other players.
//!
//! Remote players are replicated as `PlayerTransform` components. Those arrive
//! at the server's tick rate and with network jitter, so drawing them directly
//! makes them stutter. Instead each remote body is rendered ~100 ms in the past,
//! interpolating between the two most recent snapshots — the standard approach
//! for co-op games and the one the spec asks for.

use bevy::prelude::*;

use deepcore_shared::protocol::{PlayerInfo, PlayerTransform};

/// How far behind the newest snapshot remote players are rendered.
pub const INTERPOLATION_DELAY_SECS: f32 = 0.1;

/// A snapshot of a remote player's transform, with the time it was received.
#[derive(Clone, Copy, Debug)]
pub struct Snapshot {
    /// Client time when this snapshot arrived.
    pub received_at: f32,
    /// The transform.
    pub transform: PlayerTransform,
}

/// Per-remote-player interpolation buffer.
#[derive(Component, Debug, Default)]
pub struct RemoteInterpolation {
    /// Recent snapshots, oldest first.
    pub snapshots: Vec<Snapshot>,
}

/// Marker for a rendered remote player body.
#[derive(Component)]
pub struct RemotePlayerBody;

/// Marker for the name label above a remote player.
#[derive(Component)]
pub struct RemoteNameLabel;

/// The remote-player plugin.
pub struct RemotePlayersPlugin;

impl Plugin for RemotePlayersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                spawn_remote_bodies,
                record_snapshots,
                interpolate_remote_bodies,
                follow_remote_bodies,
            )
                .chain(),
        );
    }
}

/// Creates a visible body for each replicated remote player.
fn spawn_remote_bodies(
    mut commands: Commands,
    remote: Query<(Entity, &PlayerInfo), (With<PlayerTransform>, Without<RemotePlayerBody>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if remote.is_empty() {
        return;
    }
    let body_mesh = meshes.add(Capsule3d::new(0.35, 0.9));
    let head_mesh = meshes.add(Sphere::new(0.25));

    for (entity, info) in &remote {
        let material = materials.add(StandardMaterial {
            base_color: Color::srgb(0.85, 0.55, 0.30),
            perceptual_roughness: 0.8,
            ..default()
        });
        let head_material = materials.add(StandardMaterial {
            base_color: Color::srgb(0.90, 0.75, 0.60),
            perceptual_roughness: 0.7,
            ..default()
        });

        commands
            .entity(entity)
            .insert(RemoteInterpolation::default());
        commands.entity(entity).with_children(|parent| {
            parent.spawn((
                RemotePlayerBody,
                Mesh3d(body_mesh.clone()),
                MeshMaterial3d(material),
                Transform::from_xyz(0.0, 0.9, 0.0),
            ));
            parent.spawn((
                RemotePlayerBody,
                Mesh3d(head_mesh.clone()),
                MeshMaterial3d(head_material),
                Transform::from_xyz(0.0, 1.65, 0.0),
            ));
            parent.spawn((
                RemoteNameLabel,
                Text2d::new(info.name.clone()),
                TextFont {
                    font_size: bevy::text::FontSize::Px(28.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                Transform::from_xyz(0.0, 2.15, 0.0),
            ));
        });
        info!(name = %info.name, "remote player body spawned");
    }
}

/// Records every arriving transform snapshot, keeping a short history.
fn record_snapshots(
    time: Res<Time>,
    mut remote: Query<(&PlayerTransform, &mut RemoteInterpolation), Changed<PlayerTransform>>,
) {
    let now = time.elapsed_secs();
    for (transform, mut interpolation) in &mut remote {
        interpolation.snapshots.push(Snapshot {
            received_at: now,
            transform: *transform,
        });
        // Keep only the recent history: a couple of hundred ms is plenty.
        let cutoff = now - 0.5;
        interpolation
            .snapshots
            .retain(|snapshot| snapshot.received_at >= cutoff);
    }
}

/// Interpolates remote bodies to `now - INTERPOLATION_DELAY_SECS`.
fn interpolate_remote_bodies(
    time: Res<Time>,
    mut remote: Query<(&mut Transform, &RemoteInterpolation), Without<RemotePlayerBody>>,
) {
    let render_time = time.elapsed_secs() - INTERPOLATION_DELAY_SECS;
    for (mut transform, interpolation) in &mut remote {
        let Some(position) = interpolated_position(&interpolation.snapshots, render_time) else {
            continue;
        };
        transform.translation = position.0;
        transform.rotation = Quat::from_rotation_y(position.1);
    }
}

/// Finds the interpolated position and yaw for a render time.
///
/// Returns `None` when there are no snapshots yet.
pub fn interpolated_position(snapshots: &[Snapshot], render_time: f32) -> Option<(Vec3, f32)> {
    if snapshots.is_empty() {
        return None;
    }
    // Before the oldest snapshot: clamp to it.
    if render_time <= snapshots[0].received_at {
        let first = snapshots[0].transform;
        return Some((Vec3::from_array(first.position), first.yaw));
    }

    // Find the pair that brackets the render time.
    for pair in snapshots.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if render_time >= a.received_at && render_time <= b.received_at {
            let span = (b.received_at - a.received_at).max(1e-6);
            let t = ((render_time - a.received_at) / span).clamp(0.0, 1.0);
            let pa = Vec3::from_array(a.transform.position);
            let pb = Vec3::from_array(b.transform.position);
            // Interpolate yaw the short way around the circle.
            let yaw = lerp_angle(a.transform.yaw, b.transform.yaw, t);
            return Some((pa.lerp(pb, t), yaw));
        }
    }

    // Past the newest snapshot: hold the last one.
    let last = snapshots[snapshots.len() - 1].transform;
    Some((Vec3::from_array(last.position), last.yaw))
}

/// Interpolates two angles by the shortest path.
fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let mut delta = (b - a) % std::f32::consts::TAU;
    if delta > std::f32::consts::PI {
        delta -= std::f32::consts::TAU;
    } else if delta < -std::f32::consts::PI {
        delta += std::f32::consts::TAU;
    }
    a + delta * t
}

/// Keeps remote name labels facing the local camera.
fn follow_remote_bodies(
    camera: Query<&Transform, (With<crate::player::LocalPlayer>, Without<RemoteNameLabel>)>,
    mut labels: Query<&mut Transform, With<RemoteNameLabel>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    for mut label in &mut labels {
        // Billboard: copy the camera's rotation so the text always faces us.
        label.rotation = camera.rotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(at: f32, position: [f32; 3], yaw: f32) -> Snapshot {
        Snapshot {
            received_at: at,
            transform: PlayerTransform {
                position,
                yaw,
                pitch: 0.0,
            },
        }
    }

    #[test]
    fn no_snapshots_means_nothing_to_draw() {
        assert!(interpolated_position(&[], 1.0).is_none());
    }

    #[test]
    fn single_snapshot_holds_its_position() {
        let snaps = [snapshot(1.0, [5.0, 10.0, 5.0], 0.0)];
        let (pos, _) = interpolated_position(&snaps, 1.0).expect("has a snapshot");
        assert_eq!(pos, Vec3::new(5.0, 10.0, 5.0));
    }

    #[test]
    fn position_is_interpolated_between_snapshots() {
        let snaps = [
            snapshot(1.0, [0.0, 0.0, 0.0], 0.0),
            snapshot(1.2, [10.0, 0.0, 0.0], 0.0),
        ];
        // Halfway between the two.
        let (pos, _) = interpolated_position(&snaps, 1.1).expect("interpolates");
        assert!((pos.x - 5.0).abs() < 1e-3, "expected halfway, got {pos:?}");
    }

    #[test]
    fn render_time_before_history_clamps_to_the_oldest() {
        let snaps = [
            snapshot(2.0, [3.0, 0.0, 0.0], 0.0),
            snapshot(2.2, [9.0, 0.0, 0.0], 0.0),
        ];
        let (pos, _) = interpolated_position(&snaps, 1.0).expect("clamps");
        assert_eq!(
            pos.x, 3.0,
            "should hold the oldest snapshot, not extrapolate"
        );
    }

    #[test]
    fn render_time_after_history_holds_the_newest() {
        let snaps = [
            snapshot(1.0, [0.0, 0.0, 0.0], 0.0),
            snapshot(1.2, [9.0, 0.0, 0.0], 0.0),
        ];
        let (pos, _) = interpolated_position(&snaps, 5.0).expect("holds");
        assert_eq!(pos.x, 9.0);
    }

    #[test]
    fn yaw_interpolates_the_short_way_around() {
        // 350 degrees to 10 degrees should pass through 0, not through 180.
        let a = 350.0_f32.to_radians();
        let b = 10.0_f32.to_radians();
        let mid = lerp_angle(a, b, 0.5);
        // Normalise to -pi..pi and compare against 0 (i.e. 360 degrees).
        let normalised = if mid > std::f32::consts::PI {
            mid - std::f32::consts::TAU
        } else {
            mid
        };
        assert!(normalised.abs() < 0.05, "expected ~0 rad, got {normalised}");
    }

    #[test]
    fn interpolation_delay_is_about_a_tenth_of_a_second() {
        assert!((INTERPOLATION_DELAY_SECS - 0.1).abs() < 1e-6);
    }
}
