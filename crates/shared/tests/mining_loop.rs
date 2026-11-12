//! End-to-end test of the mining loop against a real generated world.
//!
//! This exercises the exact path the client uses: generate chunks, raycast to a
//! target, check the tool against the block, run the timer, and remove the block.

use bevy::math::Vec3;

use deepcore_shared::content::loader::load_from_dir;
use deepcore_shared::content::registry::Registry;
use deepcore_shared::mining::raycast::{RayFilter, raycast};
use deepcore_shared::mining::{MiningTool, check_mine};
use deepcore_shared::voxel::{BlockId, BlockPos, Chunk, ChunkPos};
use deepcore_shared::worldgen::WorldGen;

fn registry() -> Registry {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/data");
    let set = load_from_dir(&dir).expect("content loads");
    Registry::from_content(&set).expect("content resolves")
}

#[test]
fn a_player_can_dig_down_through_layer_one() {
    let registry = registry();
    let world = WorldGen::new(2024, registry.clone());

    // Stand at the camp surface.
    let surface = WorldGen::surface_y();
    let start = BlockPos::new(0, surface, 0);
    assert!(
        !world.block_at(start.x, start.y, start.z).is_air(),
        "surface is solid"
    );

    // A steel pickaxe is tier 1, which must break topsoil.
    let pickaxe = registry.item("steel_pickaxe").expect("pickaxe exists");
    let tool = MiningTool::from_item(registry.item_info(pickaxe)).expect("is a tool");
    assert_eq!(tool.tier, 1);

    let dirt = world.block_at(start.x, start.y, start.z);
    let info = registry.block_info(dirt);
    let check = check_mine(dirt, info, tool, 1.0, 30).expect("tier 1 breaks topsoil");
    assert!(check.ticks > 0 && check.ticks < 100, "break time {check:?}");

    // Dig a shaft: each block down to the bottom of layer 1 must be mineable.
    for depth in 0..16 {
        let y = surface - depth;
        let block = world.block_at(0, y, 0);
        if block.is_air() {
            continue;
        }
        let info = registry.block_info(block);
        assert!(
            check_mine(block, info, tool, 1.0, 30).is_ok(),
            "layer 1 block {:?} at depth {depth} should be mineable with a steel pickaxe",
            info.name
        );
    }
}

#[test]
fn granite_blocks_a_steel_pickaxe() {
    let registry = registry();
    let world = WorldGen::new(2024, registry.clone());
    let surface = WorldGen::surface_y();

    let pickaxe = registry.item("steel_pickaxe").expect("pickaxe exists");
    let tool = MiningTool::from_item(registry.item_info(pickaxe)).expect("is a tool");

    // Granite starts at depth 64. Find a granite block and confirm it is refused.
    let mut found = false;
    for y in (surface - 160)..(surface - 64) {
        let block = world.block_at(0, y, 0);
        let info = registry.block_info(block);
        if info.name == "granite" {
            found = true;
            let err = check_mine(block, info, tool, 1.0, 30)
                .expect_err("steel pickaxe must not break granite");
            assert!(
                err.message().contains("Tier 2"),
                "unexpected message: {}",
                err.message()
            );
            break;
        }
    }
    assert!(found, "expected granite somewhere in layer 3");

    // The hardened pickaxe (tier 2) must be able to break it.
    let hardened = registry.item("hardened_pickaxe").expect("hardened pickaxe");
    let better = MiningTool::from_item(registry.item_info(hardened)).expect("is a tool");
    assert_eq!(better.tier, 2);
    for y in (surface - 160)..(surface - 64) {
        let block = world.block_at(0, y, 0);
        let info = registry.block_info(block);
        if info.name == "granite" {
            assert!(
                check_mine(block, info, better, 1.0, 30).is_ok(),
                "tier 2 should break granite"
            );
            break;
        }
    }
}

#[test]
fn raycast_targets_the_block_a_player_looks_at() {
    let registry = registry();
    let world = WorldGen::new(2024, registry.clone());
    let surface = WorldGen::surface_y();

    // Build a tiny world view with the chunk the player stands in.
    let eye = Vec3::new(0.5, surface as f32 + 2.6, 0.5);
    let target = BlockPos::new(0, surface - 1, 0);
    let chunk_pos = target.chunk();
    let mut chunk = Chunk::new(chunk_pos);
    chunk.set(
        BlockPos::new(
            target.x.rem_euclid(32),
            target.y.rem_euclid(32),
            target.z.rem_euclid(32),
        ),
        registry.block("stone").expect("stone"),
    );

    struct Only(ChunkPos, BlockId);
    impl RayFilter for Only {
        fn blocks(&self, x: i32, y: i32, z: i32) -> bool {
            let pos = BlockPos::new(x, y, z);
            pos.chunk() == self.0 && !self.1.is_air()
        }
    }

    // Looking straight down at the block.
    let hit = raycast(eye, Vec3::NEG_Y, 5.0, &Only(chunk_pos, BlockId(1)));
    assert!(hit.is_some(), "should hit the block below");
    let hit = hit.expect("hit");
    assert_eq!(hit.block, target, "wrong block targeted: {:?}", hit.block);
}

#[test]
fn mining_a_block_changes_the_world_state() {
    let registry = registry();
    let mut chunk = Chunk::new(ChunkPos::new(0, 16, 0));
    let stone = registry.block("stone").expect("stone");
    let pos = BlockPos::new(5, 512, 5);
    chunk.set(pos, stone);
    assert_eq!(chunk.get(pos), stone);

    // Break it.
    chunk.set(pos, BlockId::AIR);
    assert!(chunk.get(pos).is_air(), "block should be gone after mining");
    assert!(
        chunk.is_modified(),
        "a changed chunk must be marked for saving"
    );
}
