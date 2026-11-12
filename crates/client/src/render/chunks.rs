//! Chunk rendering: streaming chunk data into meshes on the async task pool.
//!
//! The client owns a local copy of every chunk near the player. Generation and
//! meshing both happen off the main thread; the main thread only uploads the
//! finished mesh. That keeps frame time stable no matter how fast the player
//! moves.

use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, poll_once};

use deepcore_shared::atlas::build_atlas;
use deepcore_shared::content::{ContentRegistry, ItemId};
use deepcore_shared::mesh::{ChunkMesh, Neighbours, SolidMask, mesh_chunk};
use deepcore_shared::voxel::{BlockId, BlockPos, Chunk, ChunkPos};
use deepcore_shared::worldgen::WorldGen;

use crate::render::atlas::chunk_material;

/// Horizontal view radius in chunks.
pub const VIEW_RADIUS_XZ: i32 = 6;
/// Vertical view radius in chunks.
pub const VIEW_RADIUS_Y: i32 = 4;

/// Client-side store of loaded chunks.
#[derive(Resource, Default)]
pub struct ClientChunks {
    /// Chunks the client currently holds.
    pub chunks: HashMap<ChunkPos, Chunk>,
    /// Positions whose mesh has been queued or built.
    meshed: HashMap<ChunkPos, Entity>,
}

impl ClientChunks {
    /// Reads a block from the client's local copy of the world.
    pub fn block_at(&self, pos: BlockPos) -> Option<BlockId> {
        self.chunks.get(&pos.chunk()).map(|c| c.get(pos))
    }

    /// Applies a block change from the server.
    pub fn set_block(&mut self, pos: BlockPos, block: BlockId) -> bool {
        match self.chunks.get_mut(&pos.chunk()) {
            Some(chunk) => chunk.set(pos, block),
            None => false,
        }
    }
}

/// Resources shared by the rendering systems.
#[derive(Resource)]
pub struct RenderAssets {
    /// Per-block opacity, for face culling.
    pub solid: SolidMask,
    /// The material every chunk mesh uses.
    pub material: Handle<StandardMaterial>,
    /// UV rectangle per block id.
    pub uvs: Vec<[f32; 4]>,
}

/// A chunk mesh that has finished building and needs uploading.
struct PendingMesh {
    pos: ChunkPos,
    mesh: ChunkMesh,
}

/// In-flight background jobs.
#[derive(Resource, Default)]
struct MeshJobs {
    tasks: Vec<Task<(ChunkPos, ChunkMesh)>>,
    ready: Vec<PendingMesh>,
}

/// Marker for a chunk mesh entity.
#[derive(Component)]
pub struct ChunkMeshEntity;

/// Chunk streaming and meshing plugin.
pub struct ChunkRenderPlugin;

impl Plugin for ChunkRenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MeshJobs>()
            .add_systems(Startup, setup_render_assets)
            .add_systems(
                Update,
                (stream_chunks, poll_mesh_jobs, apply_mesh_jobs).chain(),
            );
    }
}

/// Builds the atlas and the shared chunk material once at startup.
fn setup_render_assets(
    mut commands: Commands,
    registry: Res<ContentRegistry>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let atlas = build_atlas(&registry.0);

    // Convert the generated RGBA buffer into a Bevy image.
    let image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: atlas.image.width,
            height: atlas.image.height,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        atlas.image.pixels.clone(),
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    );
    let texture = images.add(image);

    let material = materials.add(chunk_material(texture));

    let solid = SolidMask::new(registry.0.blocks.iter().map(|b| b.solid).collect());
    let uvs = atlas
        .uvs
        .iter()
        .map(|uv| [uv.u0, uv.v0, uv.u1, uv.v1])
        .collect();

    commands.insert_resource(RenderAssets {
        solid,
        material,
        uvs,
    });
}

/// Generates chunks near the player and queues their meshes.
fn stream_chunks(
    mut commands: Commands,
    player: Query<&Transform, With<crate::player::LocalPlayer>>,
    mut chunks: ResMut<ClientChunks>,
    mut jobs: ResMut<MeshJobs>,
    assets: Option<Res<RenderAssets>>,
    world_gen: Option<Res<WorldGen>>,
) {
    let (Some(assets), Some(world_gen)) = (assets, world_gen) else {
        return;
    };
    let Ok(transform) = player.single() else {
        return;
    };
    let player_chunk = ChunkPos::containing(BlockPos::new(
        transform.translation.x.floor() as i32,
        transform.translation.y.floor() as i32,
        transform.translation.z.floor() as i32,
    ));

    // Collect the positions we want, nearest first.
    // A rectangular box, not a sphere. A sphere would leave the chunk diagonally
    // adjacent to the player unloaded, and since meshing waits on horizontal
    // neighbours that would leave the ground under the player unmeshed forever.
    let mut wanted: Vec<(i32, ChunkPos)> = Vec::new();
    for dy in -VIEW_RADIUS_Y..=VIEW_RADIUS_Y {
        for dz in -VIEW_RADIUS_XZ..=VIEW_RADIUS_XZ {
            for dx in -VIEW_RADIUS_XZ..=VIEW_RADIUS_XZ {
                let pos = ChunkPos::new(
                    player_chunk.x + dx,
                    player_chunk.y + dy,
                    player_chunk.z + dz,
                );
                if !chunks.chunks.contains_key(&pos) {
                    // Prefer chunks close in Y as well as horizontally, so the
                    // layer the player stands in fills in first.
                    let dist = dx * dx + dz * dz + dy * dy * 3;
                    wanted.push((dist, pos));
                }
            }
        }
    }
    wanted.sort_by_key(|(dist, _)| *dist);

    // Generate a bounded number per frame so the main thread never stalls.
    let pool = AsyncComputeTaskPool::get();
    for (_, pos) in wanted.into_iter().take(4) {
        // Generate synchronously (fast: ~1.3ms) but mesh on the pool.
        let chunk = world_gen.generate(pos);
        chunks.chunks.insert(pos, chunk);
    }

    // Queue meshes for any loaded chunk that has none and whose neighbours are
    // all present, so boundary faces are culled correctly. Sorting by distance
    // from the player matters: HashMap iteration order is arbitrary, and without
    // this the chunks under the player can be the last to be built.
    let mut ready_to_mesh: Vec<(i32, ChunkPos)> = chunks
        .chunks
        .keys()
        .copied()
        .filter(|pos| !chunks.meshed.contains_key(pos))
        .filter(|pos| {
            // Mesh a chunk once its four horizontal neighbours are loaded. That
            // is enough for correct side-face culling, and it avoids waiting on
            // vertical neighbours that may never exist (the chunk above the
            // surface is sky, and the chunk below the floor is bedrock).
            let sides = [
                ChunkPos::new(pos.x - 1, pos.y, pos.z),
                ChunkPos::new(pos.x + 1, pos.y, pos.z),
                ChunkPos::new(pos.x, pos.y, pos.z - 1),
                ChunkPos::new(pos.x, pos.y, pos.z + 1),
            ];
            sides.iter().all(|n| chunks.chunks.contains_key(n))
        })
        .map(|pos| {
            let dx = pos.x - player_chunk.x;
            let dy = pos.y - player_chunk.y;
            let dz = pos.z - player_chunk.z;
            (dx * dx + dy * dy + dz * dz, pos)
        })
        .collect();
    ready_to_mesh.sort_by_key(|(dist, _)| *dist);

    // Bound how many meshes we queue per frame so the task pool is not flooded.
    for (_, pos) in ready_to_mesh.into_iter().take(48) {
        // Clone the chunk data for the worker; chunks are small and cheap to copy.
        let chunk = match chunks.chunks.get(&pos) {
            Some(chunk) => chunk.clone(),
            None => continue,
        };
        let neighbours = Neighbours {
            west: chunks
                .chunks
                .get(&ChunkPos::new(pos.x - 1, pos.y, pos.z))
                .cloned(),
            east: chunks
                .chunks
                .get(&ChunkPos::new(pos.x + 1, pos.y, pos.z))
                .cloned(),
            down: chunks
                .chunks
                .get(&ChunkPos::new(pos.x, pos.y - 1, pos.z))
                .cloned(),
            up: chunks
                .chunks
                .get(&ChunkPos::new(pos.x, pos.y + 1, pos.z))
                .cloned(),
            north: chunks
                .chunks
                .get(&ChunkPos::new(pos.x, pos.y, pos.z - 1))
                .cloned(),
            south: chunks
                .chunks
                .get(&ChunkPos::new(pos.x, pos.y, pos.z + 1))
                .cloned(),
        };
        let solid = assets.solid.clone();
        let uvs = assets.uvs.clone();

        // Reserve the slot so we do not queue the same chunk twice.
        chunks.meshed.insert(pos, Entity::PLACEHOLDER);

        let task = pool.spawn(async move {
            let uv_of = move |block: BlockId| -> [f32; 4] {
                uvs.get(block.0 as usize)
                    .copied()
                    .unwrap_or([0.0, 0.0, 0.0625, 0.0625])
            };
            let mesh = mesh_chunk(&chunk, &neighbours, &solid, &uv_of);
            (pos, mesh)
        });
        jobs.tasks.push(task);
    }
    let _ = &mut commands;
}

/// Polls finished mesh jobs.
fn poll_mesh_jobs(mut jobs: ResMut<MeshJobs>) {
    let tasks = std::mem::take(&mut jobs.tasks);
    let mut still_running = Vec::with_capacity(tasks.len());
    let mut finished = Vec::new();
    for mut task in tasks {
        match block_on(poll_once(&mut task)) {
            Some((pos, mesh)) => finished.push(PendingMesh { pos, mesh }),
            None => still_running.push(task),
        }
    }
    jobs.tasks = still_running;
    jobs.ready.extend(finished);
}

/// Uploads finished meshes to the GPU.
fn apply_mesh_jobs(
    mut commands: Commands,
    mut jobs: ResMut<MeshJobs>,
    mut chunks: ResMut<ClientChunks>,
    assets: Option<Res<RenderAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    existing: Query<Entity, With<ChunkMeshEntity>>,
) {
    let Some(assets) = assets else {
        return;
    };
    let _ = &existing;

    for pending in jobs.ready.drain(..) {
        // Despawn any previous mesh for this chunk.
        if let Some(old) = chunks.meshed.insert(pending.pos, Entity::PLACEHOLDER)
            && old != Entity::PLACEHOLDER
        {
            commands.entity(old).try_despawn();
        }

        if pending.mesh.is_empty() {
            continue;
        }

        let mesh_handle = meshes.add(to_bevy_mesh(&pending.mesh));
        let origin = pending.pos.origin();
        let entity = commands
            .spawn((
                ChunkMeshEntity,
                Mesh3d(mesh_handle),
                MeshMaterial3d(assets.material.clone()),
                Transform::from_xyz(origin.x as f32, origin.y as f32, origin.z as f32),
            ))
            .id();
        chunks.meshed.insert(pending.pos, entity);
    }
}

/// Converts our chunk mesh into Bevy mesh data.
pub fn to_bevy_mesh(mesh: &ChunkMesh) -> Mesh {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::mesh::{Indices, Mesh, PrimitiveTopology};

    let mut out = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let positions: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.position).collect();
    let normals: Vec<[f32; 3]> = mesh.vertices.iter().map(|v| v.normal).collect();
    let uvs: Vec<[f32; 2]> = mesh.vertices.iter().map(|v| v.uv).collect();
    // Bake the face shade into vertex colour so one material serves all faces.
    let colors: Vec<[f32; 4]> = mesh
        .vertices
        .iter()
        .map(|v| [v.shade, v.shade, v.shade, 1.0])
        .collect();

    out.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    out.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    out.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    out.insert_indices(Indices::U32(mesh.indices.clone()));
    out
}

/// Removes chunks that have left the view radius.
pub fn unload_distant_chunks(
    player: Query<&Transform, With<crate::player::LocalPlayer>>,
    mut chunks: ResMut<ClientChunks>,
    mut commands: Commands,
) {
    let Ok(transform) = player.single() else {
        return;
    };
    let center = ChunkPos::containing(BlockPos::new(
        transform.translation.x.floor() as i32,
        transform.translation.y.floor() as i32,
        transform.translation.z.floor() as i32,
    ));
    let mut to_remove = Vec::new();
    for pos in chunks.chunks.keys() {
        let dx = pos.x - center.x;
        let dy = pos.y - center.y;
        let dz = pos.z - center.z;
        // Keep a margin beyond the view radius so chunks do not thrash.
        if dx.abs() > VIEW_RADIUS_XZ + 2
            || dy.abs() > VIEW_RADIUS_Y + 2
            || dz.abs() > VIEW_RADIUS_XZ + 2
        {
            to_remove.push(*pos);
        }
    }
    for pos in to_remove {
        chunks.chunks.remove(&pos);
        if let Some(entity) = chunks.meshed.remove(&pos)
            && entity != Entity::PLACEHOLDER
        {
            commands.entity(entity).try_despawn();
        }
    }
}

/// Converts an item id to a block id when the item is placeable, for building.
pub fn item_to_block(registry: &ContentRegistry, item: ItemId) -> Option<BlockId> {
    let name = &registry.item_info(item).name;
    registry.0.block(name)
}
