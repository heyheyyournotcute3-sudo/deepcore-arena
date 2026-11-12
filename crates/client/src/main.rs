//! Deepcore client.
//!
//! Windowed first-person client. For M0 it opens a window and renders a lit
//! test scene to prove the render stack works end to end.

use bevy::prelude::*;
use bevy::window::WindowResolution;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Deepcore".to_string(),
                resolution: WindowResolution::new(1280, 720),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.05, 0.07, 0.10)))
        .insert_resource(GlobalAmbientLight {
            color: Color::srgb(0.6, 0.7, 0.9),
            brightness: 220.0,
            ..default()
        })
        .add_plugins(deepcore_shared::SharedPlugin)
        .add_systems(Startup, setup_scene)
        .add_systems(Update, spin_cube)
        .run();
}

/// Marker for the placeholder cube that M0 renders.
#[derive(Component)]
struct TestCube;

/// Builds a camera, a sun, a fill light and a test cube.
fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Ground plane so the cube reads as sitting on something.
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(32.0, 32.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.18, 0.20, 0.22),
            perceptual_roughness: 0.95,
            ..default()
        })),
    ));

    // The test cube: a stand-in for a voxel block.
    commands.spawn((
        TestCube,
        Mesh3d(meshes.add(Cuboid::new(1.0, 1.0, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.62, 0.45, 0.32),
            perceptual_roughness: 0.8,
            ..default()
        })),
        Transform::from_xyz(0.0, 0.5, 0.0),
    ));

    // Sun.
    commands.spawn((
        DirectionalLight {
            illuminance: 10_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Fill light.
    commands.spawn((
        PointLight {
            intensity: 150_000.0,
            range: 30.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(-4.0, 6.0, 6.0),
    ));

    // Camera.
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(-3.0, 3.0, 7.0).looking_at(Vec3::new(0.0, 0.5, 0.0), Vec3::Y),
    ));
}

/// Slowly rotates the test cube so the render loop is visibly alive.
fn spin_cube(time: Res<Time>, mut query: Query<&mut Transform, With<TestCube>>) {
    for mut transform in &mut query {
        transform.rotate_y(time.delta_secs() * 0.6);
    }
}
