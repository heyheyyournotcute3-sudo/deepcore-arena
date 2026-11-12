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
fn raycast_targets_the_ground_a_player_stands_on() {
    let registry = registry();
    let world = WorldGen::new(2024, registry.clone());
    let surface = WorldGen::surface_y();

    // Raycast against the real generated terrain, using the same filter shape
    // the client uses: a block stops the ray if the world is solid there.
    struct Generated<'a>(&'a WorldGen);
    impl RayFilter for Generated<'_> {
        fn blocks(&self, x: i32, y: i32, z: i32) -> bool {
            !self.0.block_at(x, y, z).is_air()
        }
    }

    // Stand at the camp and look straight down. The eye is above the surface,
    // so the first solid block below must be the surface block itself.
    let eye = Vec3::new(0.5, surface as f32 + 2.6, 0.5);
    let hit = raycast(eye, Vec3::NEG_Y, 5.0, &Generated(&world))
        .expect("looking down at the ground should hit it");
    assert_eq!(
        hit.block,
        BlockPos::new(0, surface, 0),
        "should target the surface block directly below the player"
    );
    // Looking up from the surface must hit nothing: the sky is empty.
    assert!(
        raycast(eye, Vec3::Y, 5.0, &Generated(&world)).is_none(),
        "the sky above the camp must be empty"
    );

    // Mining must mark the chunk modified so persistence saves it. Start from
    // the real generated chunk so the block is genuinely present.
    let mut chunk = world.generate(hit.block.chunk());
    let original = chunk.get(hit.block);
    assert!(
        !original.is_air(),
        "the targeted block must be solid before mining"
    );
    chunk.set(hit.block, BlockId::AIR);
    assert!(
        chunk.is_modified(),
        "a mined block must mark its chunk modified"
    );
    assert!(chunk.get(hit.block).is_air(), "the block must be gone");
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
