//! Mining rules: break time, tool tier gating and area tools.
//!
//! These functions are the contract between client prediction and server
//! authority. The client plays a crack animation using the exact value the
//! server will use, so the two never disagree about when a block breaks.

pub mod raycast;

pub use raycast::{RayFilter, RayHit, raycast};

use crate::content::defs::{ToolDef, ToolEnergy};
use crate::content::registry::{BlockInfo, ItemInfo};
use crate::voxel::BlockId;

/// Why a block cannot be mined right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MineDenied {
    /// The block is empty.
    Air,
    /// The tool tier is below the block's minimum.
    TierTooLow {
        /// Tier the block needs.
        required: u8,
        /// Tier the player's tool has.
        have: u8,
    },
    /// The block is unbreakable (bedrock).
    Unbreakable,
    /// The tool has no energy left.
    NoEnergy,
}

impl MineDenied {
    /// A short message for the HUD.
    pub fn message(&self) -> String {
        match self {
            MineDenied::Air => String::new(),
            MineDenied::TierTooLow { required, .. } => {
                format!("Too hard — needs Tier {required}")
            }
            MineDenied::Unbreakable => "Unbreakable".to_string(),
            MineDenied::NoEnergy => "Out of charge".to_string(),
        }
    }
}

/// A tool as used by the mining rules, decoupled from the full item registry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MiningTool {
    /// Tool tier.
    pub tier: u8,
    /// Speed multiplier.
    pub speed: f32,
    /// Area as `(width, height)` on the targeted face.
    pub area: (u32, u32),
}

impl MiningTool {
    /// The bare-hands fallback: tier 0, slow, single block.
    pub const HANDS: MiningTool = MiningTool {
        tier: 0,
        speed: 0.5,
        area: (1, 1),
    };

    /// Builds a tool from an item's definition, if it is a tool.
    pub fn from_item(item: &ItemInfo) -> Option<Self> {
        item.tool.as_ref().map(|t: &ToolDef| MiningTool {
            tier: t.tier,
            speed: t.speed,
            area: t.area,
        })
    }

    /// The energy source of a tool.
    pub fn energy(item: &ItemInfo) -> Option<ToolEnergy> {
        item.tool.as_ref().map(|t| t.energy)
    }
}

/// Result of asking whether a block can be mined.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MineCheck {
    /// Seconds of mining required, already divided by tool speed.
    pub seconds: f32,
    /// Server ticks of mining required (rounded up, minimum 1).
    pub ticks: u32,
}

/// Checks whether `tool` can mine `block`, and how long it takes.
///
/// Break time is `hardness / (tool_speed * power_multiplier)`. The power
/// multiplier is 1.0 here; callers apply machine or upgrade bonuses before
/// calling. The result is rounded up to whole server ticks so the server timer
/// and the client animation agree exactly.
pub fn check_mine(
    block: BlockId,
    info: &BlockInfo,
    tool: MiningTool,
    power_multiplier: f32,
    tick_hz: u32,
) -> Result<MineCheck, MineDenied> {
    if block.is_air() {
        return Err(MineDenied::Air);
    }
    if info.hardness <= 0.0 || info.min_tier >= 200 {
        return Err(MineDenied::Unbreakable);
    }
    if tool.tier < info.min_tier {
        return Err(MineDenied::TierTooLow {
            required: info.min_tier,
            have: tool.tier,
        });
    }
    let speed = (tool.speed * power_multiplier).max(0.01);
    let seconds = info.hardness / speed;
    let ticks = ((seconds * tick_hz as f32).ceil() as u32).max(1);
    Ok(MineCheck { seconds, ticks })
}

/// The offsets of every block an area tool breaks when targeting `face` at
/// `origin`. A 1×1 tool returns just the origin.
///
/// The area lies in the plane of the targeted face: a 3×3 drill hitting the top
/// of a block breaks a 3×3 square in the horizontal plane.
pub fn area_offsets(tool: MiningTool, face: Face) -> Vec<(i32, i32, i32)> {
    let (w, h) = tool.area;
    if w <= 1 && h <= 1 {
        return vec![(0, 0, 0)];
    }
    let (u, v) = face.tangents();
    let mut out = Vec::with_capacity((w * h) as usize);
    let half_w = (w as i32 - 1) / 2;
    let half_h = (h as i32 - 1) / 2;
    for du in -half_w..=half_w {
        for dv in -half_h..=half_h {
            let dx = u.0 * du + v.0 * dv;
            let dy = u.1 * du + v.1 * dv;
            let dz = u.2 * du + v.2 * dv;
            out.push((dx, dy, dz));
        }
    }
    out
}

/// The face of a block that was targeted, and its outward normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    /// Negative X.
    West,
    /// Positive X.
    East,
    /// Negative Y (bottom).
    Down,
    /// Positive Y (top).
    Up,
    /// Negative Z.
    North,
    /// Positive Z.
    South,
}

impl Face {
    /// The outward normal of the face.
    pub fn normal(self) -> (i32, i32, i32) {
        match self {
            Face::West => (-1, 0, 0),
            Face::East => (1, 0, 0),
            Face::Down => (0, -1, 0),
            Face::Up => (0, 1, 0),
            Face::North => (0, 0, -1),
            Face::South => (0, 0, 1),
        }
    }

    /// The two in-plane axes of the face, used to lay out area tools.
    pub fn tangents(self) -> ((i32, i32, i32), (i32, i32, i32)) {
        match self {
            Face::West | Face::East => ((0, 0, 1), (0, 1, 0)),
            Face::Up | Face::Down => ((1, 0, 0), (0, 0, 1)),
            Face::North | Face::South => ((1, 0, 0), (0, 1, 0)),
        }
    }

    /// The face opposite this one, i.e. the face a placed block would show.
    pub fn opposite(self) -> Face {
        match self {
            Face::West => Face::East,
            Face::East => Face::West,
            Face::Up => Face::Down,
            Face::Down => Face::Up,
            Face::North => Face::South,
            Face::South => Face::North,
        }
    }

    /// The face whose normal best matches a direction vector.
    pub fn from_normal(normal: bevy::math::Vec3) -> Face {
        let abs = normal.abs();
        if abs.x >= abs.y && abs.x >= abs.z {
            if normal.x > 0.0 {
                Face::East
            } else {
                Face::West
            }
        } else if abs.y >= abs.z {
            if normal.y > 0.0 { Face::Up } else { Face::Down }
        } else if normal.z > 0.0 {
            Face::South
        } else {
            Face::North
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::registry::ItemStack;

    fn block_info(hardness: f32, min_tier: u8) -> BlockInfo {
        BlockInfo {
            name: "test".to_string(),
            hardness,
            min_tier,
            drops: Vec::<ItemStack>::new(),
            color: (0.5, 0.5, 0.5),
            solid: true,
            placeable: true,
            emissive: 0.0,
            radiation: None,
        }
    }

    #[test]
    fn break_time_divides_hardness_by_speed() {
        let info = block_info(4.0, 1);
        let tool = MiningTool {
            tier: 1,
            speed: 1.0,
            area: (1, 1),
        };
        let check = check_mine(BlockId(1), &info, tool, 1.0, 30).expect("mineable");
        assert!((check.seconds - 4.0).abs() < 1e-6);
        assert_eq!(check.ticks, 120, "4s at 30Hz is 120 ticks");
    }

    #[test]
    fn faster_tools_break_sooner() {
        let info = block_info(6.0, 1);
        let slow = check_mine(
            BlockId(1),
            &info,
            MiningTool {
                tier: 1,
                speed: 1.0,
                area: (1, 1),
            },
            1.0,
            30,
        )
        .expect("mineable");
        let fast = check_mine(
            BlockId(1),
            &info,
            MiningTool {
                tier: 3,
                speed: 3.0,
                area: (1, 1),
            },
            1.0,
            30,
        )
        .expect("mineable");
        assert!((slow.seconds - 6.0).abs() < 1e-6);
        assert!((fast.seconds - 2.0).abs() < 1e-6);
        assert_eq!(fast.ticks * 3, slow.ticks);
    }

    #[test]
    fn tier_gating_blocks_low_tools() {
        let info = block_info(2.5, 2);
        let err = check_mine(
            BlockId(1),
            &info,
            MiningTool {
                tier: 1,
                speed: 1.0,
                area: (1, 1),
            },
            1.0,
            30,
        )
        .expect_err("tier 1 must not mine a tier 2 block");
        assert_eq!(
            err,
            MineDenied::TierTooLow {
                required: 2,
                have: 1
            }
        );
        assert!(err.message().contains("Tier 2"));
    }

    #[test]
    fn bedrock_is_unbreakable() {
        let info = block_info(1000.0, 255);
        assert_eq!(
            check_mine(
                BlockId(1),
                &info,
                MiningTool {
                    tier: 5,
                    speed: 12.0,
                    area: (3, 3)
                },
                1.0,
                30
            ),
            Err(MineDenied::Unbreakable)
        );
    }

    #[test]
    fn air_cannot_be_mined() {
        let info = block_info(1.0, 1);
        assert_eq!(
            check_mine(BlockId::AIR, &info, MiningTool::HANDS, 1.0, 30),
            Err(MineDenied::Air)
        );
    }

    #[test]
    fn ticks_are_rounded_up_so_timers_never_undercut() {
        // 1.0 hardness at speed 0.3 = 3.333s = 100 ticks exactly at 30Hz.
        let info = block_info(1.0, 1);
        let check = check_mine(
            BlockId(1),
            &info,
            MiningTool {
                tier: 1,
                speed: 0.3,
                area: (1, 1),
            },
            1.0,
            30,
        )
        .expect("mineable");
        assert!(check.ticks >= 100);
        // A tiny hardness still costs at least one tick.
        let tiny = check_mine(
            BlockId(1),
            &block_info(0.001, 1),
            MiningTool {
                tier: 1,
                speed: 9.0,
                area: (1, 1),
            },
            1.0,
            30,
        )
        .expect("mineable");
        assert_eq!(tiny.ticks, 1);
    }

    #[test]
    fn area_tool_covers_the_targeted_face() {
        let drill = MiningTool {
            tier: 5,
            speed: 12.0,
            area: (3, 3),
        };
        let offsets = area_offsets(drill, Face::Up);
        assert_eq!(offsets.len(), 9);
        assert!(
            offsets.contains(&(0, 0, 0)),
            "must include the target itself"
        );
        // On the top face, the spread is in X and Z, not Y.
        assert!(offsets.iter().all(|(_, dy, _)| *dy == 0));
        let xs: std::collections::HashSet<i32> = offsets.iter().map(|(x, _, _)| *x).collect();
        assert_eq!(xs, [-1, 0, 1].into_iter().collect());
    }

    #[test]
    fn single_block_tool_has_one_offset() {
        let pick = MiningTool {
            tier: 1,
            speed: 1.0,
            area: (1, 1),
        };
        assert_eq!(area_offsets(pick, Face::North), vec![(0, 0, 0)]);
    }

    #[test]
    fn faces_are_consistent() {
        assert_eq!(Face::Up.opposite(), Face::Down);
        assert_eq!(Face::East.normal(), (1, 0, 0));
        assert_eq!(
            Face::from_normal(bevy::math::Vec3::new(0.0, 1.0, 0.0)),
            Face::Up
        );
        assert_eq!(
            Face::from_normal(bevy::math::Vec3::new(-1.0, 0.0, 0.0)),
            Face::West
        );
    }
}
