//! Chunk rendering, block highlight and camera.

pub mod atlas;
pub mod chunks;

use bevy::prelude::*;

use chunks::ChunkRenderPlugin;

/// Everything the client needs to draw the voxel world.
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ChunkRenderPlugin)
            .add_systems(Startup, setup_scene)
            .add_systems(
                Update,
                (update_block_highlight, chunks::unload_distant_chunks),
            );
    }
}

/// Marker for the wireframe cube showing the targeted block.
#[derive(Component)]
pub struct BlockHighlight;

/// Marker for the sun.
#[derive(Component)]
pub struct Sun;

/// Spawns lighting and the block highlight cube.
fn setup_scene(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Sky-ish ambient so unlit caves are still navigable at the surface.
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.55, 0.65, 0.85),
        brightness: 400.0,
        ..default()
    });

    // Sun.
    commands.spawn((
        Sun,
        DirectionalLight {
            illuminance: 12_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(60.0, 120.0, 40.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // Block highlight: a slightly inflated wireframe-ish cube.
    let highlight_mesh = meshes.add(Cuboid::new(1.02, 1.02, 1.02));
    let highlight_material = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.18),
        emissive: LinearRgba::new(0.5, 0.5, 0.5, 1.0),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        ..default()
    });
    commands.spawn((
        BlockHighlight,
        Mesh3d(highlight_mesh),
        MeshMaterial3d(highlight_material),
        Transform::from_xyz(0.0, -1000.0, 0.0),
        Visibility::Hidden,
    ));
}

/// Moves the highlight cube onto the block the player is aiming at.
fn update_block_highlight(
    target: Res<crate::player::TargetedBlock>,
    mut highlight: Query<(&mut Transform, &mut Visibility), With<BlockHighlight>>,
) {
    let Ok((mut transform, mut visibility)) = highlight.single_mut() else {
        return;
    };
    match target.0 {
        Some(hit) => {
            let pos = hit.block;
            transform.translation =
                Vec3::new(pos.x as f32 + 0.5, pos.y as f32 + 0.5, pos.z as f32 + 0.5);
            *visibility = Visibility::Visible;
        }
        None => {
            *visibility = Visibility::Hidden;
        }
    }
}
