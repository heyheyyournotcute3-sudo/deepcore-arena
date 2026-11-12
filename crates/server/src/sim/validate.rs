//! Validation of client intents.
//!
//! **Every** intent from a client is untrusted. A client can send anything: a
//! block a mile away, a tier-5 drill it does not own, a negative slot index. The
//! rules in this module are the only thing standing between a malicious client
//! and the world, so they are pure functions with exhaustive tests.
//!
//! A rejected intent is dropped and logged. It must never panic and never
//! desync the server.

use bevy::math::Vec3;

use deepcore_shared::content::ContentRegistry;
use deepcore_shared::mining::raycast::{RayFilter, raycast};
use deepcore_shared::mining::{Face, MineDenied, MiningTool, area_offsets, check_mine};
use deepcore_shared::protocol::{FaceWire, REACH, REACH_DRILL};
use deepcore_shared::voxel::{BlockId, BlockPos};

/// Why an intent was rejected. Each maps to a log line and, where useful, a
/// message back to the client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// The target block is further than the player's reach allows.
    OutOfReach {
        /// Distance in blocks.
        distance: u32,
        /// The limit that applied.
        limit: u32,
    },
    /// Something solid is between the player's eye and the target.
    NoLineOfSight,
    /// The player's tool tier is too low for the block.
    TierTooLow {
        /// Tier the block needs.
        required: u8,
        /// Tier the player has.
        have: u8,
    },
    /// The block cannot be broken at all.
    Unbreakable,
    /// There is nothing to mine there.
    NothingThere,
    /// The player does not hold the item the intent claims.
    NoSuchItem {
        /// The slot that was claimed.
        slot: u8,
    },
    /// The item in that slot cannot be placed as a block.
    NotPlaceable,
    /// The destination already contains a block.
    DestinationOccupied,
    /// The new block would intersect the player.
    WouldTrapPlayer,
    /// The position is outside the world.
    OutOfWorld,
    /// The chunk is not loaded on the server, so the answer is unknown.
    ChunkNotLoaded,
    /// The recipe is not unlocked by the tech tree.
    RecipeLocked,
    /// The named machine is not within reach.
    MachineOutOfReach,
    /// The player entity does not exist (e.g. still joining).
    NoPlayer,
}

impl Rejected {
    /// A short reason for the client, or `None` when silence is kinder.
    pub fn client_message(&self) -> Option<String> {
        Some(match self {
            Rejected::OutOfReach { .. } => "Too far away".to_string(),
            Rejected::NoLineOfSight => "Blocked".to_string(),
            Rejected::TierTooLow { required, .. } => format!("Too hard — needs Tier {required}"),
            Rejected::Unbreakable => "Unbreakable".to_string(),
            Rejected::NothingThere => return None,
            Rejected::NoSuchItem { .. } => "You don't have that".to_string(),
            Rejected::NotPlaceable => "Can't place that".to_string(),
            Rejected::DestinationOccupied => return None,
            Rejected::WouldTrapPlayer => "No room".to_string(),
            Rejected::OutOfWorld => "Outside the mine".to_string(),
            Rejected::ChunkNotLoaded => return None,
            Rejected::RecipeLocked => "Not researched yet".to_string(),
            Rejected::MachineOutOfReach => "Too far away".to_string(),
            Rejected::NoPlayer => return None,
        })
    }

    /// A stable tag for logging, so rejections can be counted by kind.
    pub fn tag(&self) -> &'static str {
        match self {
            Rejected::OutOfReach { .. } => "out_of_reach",
            Rejected::NoLineOfSight => "no_line_of_sight",
            Rejected::TierTooLow { .. } => "tier_too_low",
            Rejected::Unbreakable => "unbreakable",
            Rejected::NothingThere => "nothing_there",
            Rejected::NoSuchItem { .. } => "no_such_item",
            Rejected::NotPlaceable => "not_placeable",
            Rejected::DestinationOccupied => "destination_occupied",
            Rejected::WouldTrapPlayer => "would_trap_player",
            Rejected::OutOfWorld => "out_of_world",
            Rejected::ChunkNotLoaded => "chunk_not_loaded",
            Rejected::RecipeLocked => "recipe_locked",
            Rejected::MachineOutOfReach => "machine_out_of_reach",
            Rejected::NoPlayer => "no_player",
        }
    }
}

/// A player's authority-relevant state, as the server sees it.
#[derive(Clone, Debug)]
pub struct PlayerView {
    /// Eye position, where reach and line of sight are measured from.
    pub eye: Vec3,
    /// Look direction, for the line-of-sight ray.
    pub look: Vec3,
    /// The tool the player is holding.
    pub tool: MiningTool,
    /// Hotbar contents as `(item id, count)`; `None` is empty.
    pub hotbar: [Option<(u16, u32)>; 9],
    /// The player's collision box, for place-block checks.
    pub body_min: Vec3,
    /// Upper corner of the player's collision box.
    pub body_max: Vec3,
}

impl PlayerView {
    /// The reach this player's tool allows.
    pub fn reach(&self) -> f32 {
        if self.tool.area.0 > 1 || self.tool.area.1 > 1 {
            REACH_DRILL
        } else {
            REACH
        }
    }

    /// Distance from the eye to the centre of a block.
    pub fn distance_to(&self, pos: BlockPos) -> f32 {
        let centre = Vec3::new(pos.x as f32 + 0.5, pos.y as f32 + 0.5, pos.z as f32 + 0.5);
        self.eye.distance(centre)
    }
}

/// Read-only block access for validation.
pub trait BlockSource {
    /// The block at a position, or `None` if the chunk is not loaded.
    fn block(&self, pos: BlockPos) -> Option<BlockId>;
}

/// The result of validating a mining intent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MiningPlan {
    /// The block that will be broken.
    pub block: BlockPos,
    /// Ticks of mining required.
    pub ticks: u32,
    /// Every block the tool breaks (1 for a normal tool, up to 9 for a drill).
    pub area: usize,
}

/// Validates a "start mining" intent.
///
/// Checks reach, line of sight, tool tier and block presence, in that order so
/// the cheapest checks fail first.
pub fn validate_mining(
    player: &PlayerView,
    target: BlockPos,
    face: FaceWire,
    registry: &ContentRegistry,
    blocks: &impl BlockSource,
) -> Result<MiningPlan, Rejected> {
    let reach = player.reach();

    // Reach, measured to the block centre.
    let distance = player.distance_to(target);
    if distance > reach + 0.5 {
        return Err(Rejected::OutOfReach {
            distance: distance as u32,
            limit: reach as u32,
        });
    }

    // The block must exist and be loaded.
    let Some(block) = blocks.block(target) else {
        return Err(Rejected::ChunkNotLoaded);
    };
    if block.is_air() {
        return Err(Rejected::NothingThere);
    }

    // Line of sight: nothing solid between the eye and the target.
    let filter = WorldFilter { blocks };
    let face_enum = face_from_wire(face);
    if let Some(hit) = raycast(player.eye, player.look, reach + 1.0, &filter) {
        // The ray may legitimately hit the target itself, or an adjacent block
        // that an area tool would break.
        let offsets = area_offsets(player.tool, face_enum);
        let allowed: Vec<BlockPos> = offsets
            .iter()
            .map(|(dx, dy, dz)| target.offset(*dx, *dy, *dz))
            .collect();
        if hit.block != target && !allowed.contains(&hit.block) {
            return Err(Rejected::NoLineOfSight);
        }
    }

    // Tool tier against the block.
    let info = registry.0.block_info(block);
    match check_mine(
        block,
        info,
        player.tool,
        1.0,
        deepcore_shared::protocol::TICK_HZ,
    ) {
        Ok(check) => Ok(MiningPlan {
            block: target,
            ticks: check.ticks,
            area: area_offsets(player.tool, face_enum).len(),
        }),
        Err(MineDenied::Unbreakable) => Err(Rejected::Unbreakable),
        Err(MineDenied::TierTooLow { required, have }) => {
            Err(Rejected::TierTooLow { required, have })
        }
        Err(MineDenied::Air) => Err(Rejected::NothingThere),
        Err(MineDenied::NoEnergy) => Err(Rejected::TierTooLow {
            required: info.min_tier,
            have: player.tool.tier,
        }),
    }
}

/// Validates a "place block" intent, returning the position that would be filled.
pub fn validate_placing(
    player: &PlayerView,
    target: BlockPos,
    face: FaceWire,
    slot: u8,
    registry: &ContentRegistry,
    blocks: &impl BlockSource,
) -> Result<BlockPos, Rejected> {
    // Slot must exist in the hotbar.
    let Some(entry) = player.hotbar.get(slot as usize).copied().flatten() else {
        return Err(Rejected::NoSuchItem { slot });
    };
    let (item_id, count) = entry;
    if count == 0 {
        return Err(Rejected::NoSuchItem { slot });
    }

    // The item must correspond to a placeable block.
    let Some(item) = registry
        .0
        .items
        .get(item_id as usize)
        .map(|info| info.name.clone())
    else {
        return Err(Rejected::NotPlaceable);
    };
    let Some(block) = registry.0.block(&item) else {
        return Err(Rejected::NotPlaceable);
    };
    if !registry.0.block_info(block).placeable {
        return Err(Rejected::NotPlaceable);
    }

    // Reach.
    let distance = player.distance_to(target);
    if distance > player.reach() + 0.5 {
        return Err(Rejected::OutOfReach {
            distance: distance as u32,
            limit: player.reach() as u32,
        });
    }

    // Destination is the neighbouring cell on the targeted face.
    let (nx, ny, nz) = face.normal();
    let destination = target.offset(nx, ny, nz);

    // The targeted block must actually be there to place against.
    match blocks.block(target) {
        None => return Err(Rejected::ChunkNotLoaded),
        Some(b) if b.is_air() => return Err(Rejected::NothingThere),
        Some(_) => {}
    }

    // The destination must be empty.
    match blocks.block(destination) {
        None => return Err(Rejected::ChunkNotLoaded),
        Some(b) if !b.is_air() => return Err(Rejected::DestinationOccupied),
        Some(_) => {}
    }

    // The new block must not intersect the player's body.
    let cell_min = Vec3::new(
        destination.x as f32,
        destination.y as f32,
        destination.z as f32,
    );
    let cell_max = cell_min + Vec3::ONE;
    let overlaps = player.body_min.x < cell_max.x
        && player.body_max.x > cell_min.x
        && player.body_min.y < cell_max.y
        && player.body_max.y > cell_min.y
        && player.body_min.z < cell_max.z
        && player.body_max.z > cell_min.z;
    if overlaps {
        return Err(Rejected::WouldTrapPlayer);
    }

    Ok(destination)
}

/// Converts the wire face to the shared `Face` type.
pub fn face_from_wire(face: FaceWire) -> Face {
    match face {
        FaceWire::West => Face::West,
        FaceWire::East => Face::East,
        FaceWire::Down => Face::Down,
        FaceWire::Up => Face::Up,
        FaceWire::North => Face::North,
        FaceWire::South => Face::South,
    }
}

/// A [`RayFilter`] backed by the authoritative world.
struct WorldFilter<'a, B: BlockSource> {
    blocks: &'a B,
}

impl<B: BlockSource> RayFilter for WorldFilter<'_, B> {
    fn blocks(&self, x: i32, y: i32, z: i32) -> bool {
        match self.blocks.block(BlockPos::new(x, y, z)) {
            // Unloaded space does not block sight, which avoids spurious
            // rejections at the edge of the loaded area.
            Some(block) => !block.is_air(),
            None => false,
        }
    }
}

/// Checks that a recipe is unlocked by the current tech state.
pub fn recipe_unlocked(recipe_tech: Option<u16>, researched: &[bool]) -> bool {
    match recipe_tech {
        None => true,
        Some(tech) => researched.get(tech as usize).copied().unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use deepcore_shared::content::loader::load_from_dir;
    use std::collections::HashMap;
    use std::path::Path;

    fn registry() -> ContentRegistry {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/data");
        let set = load_from_dir(&dir).expect("content");
        ContentRegistry(deepcore_shared::content::Registry::from_content(&set).expect("resolves"))
    }

    /// A block source backed by an explicit map, so tests are exact.
    struct MapBlocks(HashMap<BlockPos, BlockId>);

    impl BlockSource for MapBlocks {
        fn block(&self, pos: BlockPos) -> Option<BlockId> {
            // Everything inside a small box is "loaded air" unless overridden.
            if pos.y < 0 || pos.y > 520 {
                return None;
            }
            Some(self.0.get(&pos).copied().unwrap_or(BlockId::AIR))
        }
    }

    fn stone_world(registry: &ContentRegistry) -> MapBlocks {
        let stone = registry.0.block("stone").expect("stone");
        let mut map = HashMap::new();
        // A floor of stone at y=100 across a wide area.
        for x in -10..10 {
            for z in -10..10 {
                map.insert(BlockPos::new(x, 100, z), stone);
            }
        }
        MapBlocks(map)
    }

    fn pickaxe(registry: &ContentRegistry) -> MiningTool {
        let item = registry.0.item("steel_pickaxe").expect("pickaxe");
        MiningTool::from_item(registry.0.item_info(item)).expect("is a tool")
    }

    fn player_at(eye: Vec3, tool: MiningTool) -> PlayerView {
        PlayerView {
            eye,
            look: Vec3::NEG_Y,
            tool,
            hotbar: [None; 9],
            body_min: eye - Vec3::new(0.3, 1.62, 0.3),
            body_max: eye + Vec3::new(0.3, 0.18, 0.3),
        }
    }

    #[test]
    fn valid_mining_is_accepted() {
        let registry = registry();
        let blocks = stone_world(&registry);
        let player = player_at(Vec3::new(0.5, 102.0, 0.5), pickaxe(&registry));
        let plan = validate_mining(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            &registry,
            &blocks,
        )
        .expect("should be valid");
        assert_eq!(plan.block, BlockPos::new(0, 100, 0));
        assert!(plan.ticks > 0);
        assert_eq!(plan.area, 1);
    }

    #[test]
    fn out_of_reach_is_rejected() {
        let registry = registry();
        let blocks = stone_world(&registry);
        // Eye 20 blocks above the block: far beyond a 5-block reach.
        let player = player_at(Vec3::new(0.5, 122.0, 0.5), pickaxe(&registry));
        let err = validate_mining(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            &registry,
            &blocks,
        )
        .expect_err("must be out of reach");
        assert!(matches!(err, Rejected::OutOfReach { .. }), "got {err:?}");
    }

    #[test]
    fn mining_air_is_rejected() {
        let registry = registry();
        let blocks = stone_world(&registry);
        let player = player_at(Vec3::new(0.5, 102.0, 0.5), pickaxe(&registry));
        let err = validate_mining(
            &player,
            BlockPos::new(0, 101, 0),
            FaceWire::Up,
            &registry,
            &blocks,
        )
        .expect_err("air is not mineable");
        assert_eq!(err, Rejected::NothingThere);
    }

    #[test]
    fn tier_gating_is_enforced_server_side() {
        let registry = registry();
        let granite = registry.0.block("granite").expect("granite");
        let mut map = HashMap::new();
        map.insert(BlockPos::new(0, 100, 0), granite);
        let blocks = MapBlocks(map);

        // Steel pickaxe is tier 1; granite needs tier 2.
        let player = player_at(Vec3::new(0.5, 102.0, 0.5), pickaxe(&registry));
        let err = validate_mining(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            &registry,
            &blocks,
        )
        .expect_err("tier 1 must not mine granite");
        assert_eq!(
            err,
            Rejected::TierTooLow {
                required: 2,
                have: 1
            }
        );
        assert!(err.client_message().expect("message").contains("Tier 2"));
    }

    #[test]
    fn a_wall_between_player_and_block_blocks_line_of_sight() {
        let registry = registry();
        let stone = registry.0.block("stone").expect("stone");
        let mut map = HashMap::new();
        // Target block below, and a solid block directly between eye and target.
        map.insert(BlockPos::new(0, 100, 0), stone);
        map.insert(BlockPos::new(0, 101, 0), stone);
        let blocks = MapBlocks(map);
        let player = player_at(Vec3::new(0.5, 102.0, 0.5), pickaxe(&registry));
        let err = validate_mining(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            &registry,
            &blocks,
        )
        .expect_err("the intervening block should block sight");
        // The ray hits the intervening block, which is not the target.
        assert_eq!(err, Rejected::NoLineOfSight);
    }

    #[test]
    fn bedrock_is_unbreakable_even_for_the_best_tool() {
        let registry = registry();
        let bedrock = registry.0.block("bedrock").expect("bedrock");
        let mut map = HashMap::new();
        map.insert(BlockPos::new(0, 100, 0), bedrock);
        let blocks = MapBlocks(map);

        let drill = registry.0.item("nuclear_drill").expect("drill");
        let tool = MiningTool::from_item(registry.0.item_info(drill)).expect("is a tool");
        let player = player_at(Vec3::new(0.5, 102.0, 0.5), tool);
        let err = validate_mining(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            &registry,
            &blocks,
        )
        .expect_err("bedrock is unbreakable");
        assert_eq!(err, Rejected::Unbreakable);
    }

    #[test]
    fn area_tool_reports_its_area_and_gets_extra_reach() {
        let registry = registry();
        let blocks = stone_world(&registry);
        let drill = registry.0.item("nuclear_drill").expect("drill");
        let tool = MiningTool::from_item(registry.0.item_info(drill)).expect("is a tool");
        // Nuclear drill is 3x3, so it gets the 6-block reach.
        let player = player_at(Vec3::new(0.5, 103.0, 0.5), tool);
        assert!((player.reach() - REACH_DRILL).abs() < 1e-6);
        let plan = validate_mining(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            &registry,
            &blocks,
        )
        .expect("drill should mine");
        assert_eq!(plan.area, 9);
    }

    #[test]
    fn placing_requires_the_item_in_the_claimed_slot() {
        let registry = registry();
        let blocks = stone_world(&registry);
        let mut player = player_at(Vec3::new(0.5, 102.0, 0.5), pickaxe(&registry));
        // Empty slot 3.
        let err = validate_placing(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            3,
            &registry,
            &blocks,
        )
        .expect_err("empty slot must be refused");
        assert_eq!(err, Rejected::NoSuchItem { slot: 3 });

        // Out-of-range slot index must not panic.
        let err = validate_placing(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            200,
            &registry,
            &blocks,
        )
        .expect_err("slot 200 must be refused");
        assert!(matches!(err, Rejected::NoSuchItem { .. }));

        // Give the player stone bricks and try again, standing clear of the
        // cell being filled.
        let bricks = registry.0.item("stone_bricks").expect("bricks");
        player.hotbar[0] = Some((bricks.0, 10));
        player.eye = Vec3::new(2.5, 102.0, 2.5);
        player.body_min = player.eye - Vec3::new(0.3, 1.62, 0.3);
        player.body_max = player.eye + Vec3::new(0.3, 0.18, 0.3);
        let dest = validate_placing(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            0,
            &registry,
            &blocks,
        )
        .expect("placing bricks on top of stone is valid");
        assert_eq!(dest, BlockPos::new(0, 101, 0));
    }

    #[test]
    fn placing_a_tool_is_refused() {
        let registry = registry();
        let blocks = stone_world(&registry);
        let mut player = player_at(Vec3::new(0.5, 102.0, 0.5), pickaxe(&registry));
        let pick = registry.0.item("steel_pickaxe").expect("pickaxe");
        player.hotbar[0] = Some((pick.0, 1));
        let err = validate_placing(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            0,
            &registry,
            &blocks,
        )
        .expect_err("a pickaxe is not a block");
        assert_eq!(err, Rejected::NotPlaceable);
    }

    #[test]
    fn placing_into_an_occupied_cell_is_refused() {
        let registry = registry();
        let stone = registry.0.block("stone").expect("stone");
        let mut map = HashMap::new();
        map.insert(BlockPos::new(0, 100, 0), stone);
        map.insert(BlockPos::new(0, 101, 0), stone);
        let blocks = MapBlocks(map);
        // Stand right next to the column, within reach.
        let mut player = player_at(Vec3::new(1.5, 102.0, 1.5), pickaxe(&registry));
        let bricks = registry.0.item("stone_bricks").expect("bricks");
        player.hotbar[0] = Some((bricks.0, 10));
        let err = validate_placing(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            0,
            &registry,
            &blocks,
        )
        .expect_err("destination is occupied");
        assert_eq!(err, Rejected::DestinationOccupied);
    }

    #[test]
    fn placing_inside_yourself_is_refused() {
        let registry = registry();
        let blocks = stone_world(&registry);
        let mut player = player_at(Vec3::new(0.5, 102.0, 0.5), pickaxe(&registry));
        let bricks = registry.0.item("stone_bricks").expect("bricks");
        player.hotbar[0] = Some((bricks.0, 10));
        // Target the block the player is standing in, placing up into their feet.
        let err = validate_placing(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            0,
            &registry,
            &blocks,
        );
        // The player's body spans y 100.38..102.18, so y=101 is inside them.
        assert_eq!(err, Err(Rejected::WouldTrapPlayer));
    }

    #[test]
    fn unloaded_chunks_are_not_silently_treated_as_air() {
        struct Unloaded;
        impl BlockSource for Unloaded {
            fn block(&self, _pos: BlockPos) -> Option<BlockId> {
                None
            }
        }
        let registry = registry();
        let player = player_at(Vec3::new(0.5, 102.0, 0.5), pickaxe(&registry));
        let err = validate_mining(
            &player,
            BlockPos::new(0, 100, 0),
            FaceWire::Up,
            &registry,
            &Unloaded,
        )
        .expect_err("unknown is not mineable");
        assert_eq!(err, Rejected::ChunkNotLoaded);
    }

    #[test]
    fn recipes_are_gated_by_research() {
        assert!(
            recipe_unlocked(None, &[]),
            "no tech requirement means always available"
        );
        assert!(!recipe_unlocked(Some(3), &[false, false, false]), "locked");
        assert!(recipe_unlocked(Some(1), &[false, true]), "researched");
        // An out-of-range tech index must not panic.
        assert!(!recipe_unlocked(Some(99), &[true]));
    }

    #[test]
    fn rejection_tags_are_unique_per_variant() {
        let samples = [
            Rejected::OutOfReach {
                distance: 9,
                limit: 5,
            },
            Rejected::NoLineOfSight,
            Rejected::TierTooLow {
                required: 2,
                have: 1,
            },
            Rejected::Unbreakable,
            Rejected::NothingThere,
            Rejected::NoSuchItem { slot: 0 },
            Rejected::NotPlaceable,
            Rejected::DestinationOccupied,
            Rejected::WouldTrapPlayer,
            Rejected::OutOfWorld,
            Rejected::ChunkNotLoaded,
            Rejected::RecipeLocked,
            Rejected::MachineOutOfReach,
            Rejected::NoPlayer,
        ];
        let tags: std::collections::HashSet<&str> = samples.iter().map(|r| r.tag()).collect();
        assert_eq!(
            tags.len(),
            samples.len(),
            "every variant needs a distinct tag"
        );
    }
}
