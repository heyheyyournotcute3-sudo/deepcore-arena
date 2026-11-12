//! Content definitions as they appear in the RON files under `assets/data/`.
//!
//! These types mirror the file format exactly. They are loaded once at startup,
//! validated, and resolved into compact registries (see [`super::registry`]).
//! String ids like `"raw_copper"` appear only here and in the registries' lookup
//! maps; gameplay code uses numeric ids.

use serde::{Deserialize, Serialize};

/// How a block's texture is drawn by the procedural atlas generator.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum Pattern {
    /// Flat colour, no detail. Used for glass-like or pure-colour blocks.
    Solid,
    /// Fine random speckle, e.g. stone or dirt.
    Speckle,
    /// Larger blotches, e.g. gravel or clay.
    Blotch,
    /// Bright flecks of the given colour over the base colour, e.g. ore in rock.
    OreFleck([f32; 3]),
    /// Vertical grain, e.g. wood or shale.
    Grain,
    /// Glowing core, used for uranium and other emissive blocks.
    Glow,
}

/// Definition of a block type.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockDef {
    /// Stable string id, e.g. `"granite"`.
    pub id: String,
    /// Seconds to break with a tier-1 tool of speed 1.0.
    pub hardness: f32,
    /// Minimum tool tier required. Below this the block cannot be broken.
    pub min_tier: u8,
    /// Items dropped when broken, as `(item_id, count)`.
    #[serde(default)]
    pub drops: Vec<(String, u32)>,
    /// Base colour, linear RGB in 0..1.
    pub color: (f32, f32, f32),
    /// Procedural texture pattern.
    pub pattern: Pattern,
    /// Which layer this block is native to, for diagnostics and balance.
    #[serde(default)]
    pub layer_hint: Option<u8>,
    /// Emissive strength 0..1; non-zero blocks glow in the dark.
    #[serde(default)]
    pub emissive: f32,
    /// Radiation emitted while exposed, as `(radius_blocks, dose_per_second)`.
    #[serde(default)]
    pub radiation: Option<(f32, f32)>,
    /// Whether the block is a solid obstacle for movement and light.
    #[serde(default = "default_true")]
    pub solid: bool,
    /// Whether players may place this block back into the world.
    #[serde(default = "default_true")]
    pub placeable: bool,
}

fn default_true() -> bool {
    true
}

/// Broad item category. Drives UI grouping and stacking rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ItemCategory {
    /// Raw ore or unprocessed material.
    Raw,
    /// Smelted or refined material (ingots, plates, acids).
    Material,
    /// A tool the player holds and uses.
    Tool,
    /// A placeable machine or structure.
    Machine,
    /// A consumable or equipment item worn by the player.
    Gear,
    /// Anything else.
    #[default]
    Misc,
}

/// How a tool is powered.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToolEnergy {
    /// No energy requirement; hand tools.
    None,
    /// Burns fuel units; refilled at a boiler.
    Steam {
        /// Fuel capacity in units.
        capacity: u32,
    },
    /// Drains battery charge; recharged at a charging station.
    Battery {
        /// Charge capacity in units.
        capacity: u32,
    },
    /// Consumes a nuclear core cell after a fixed number of breaks.
    CoreCell {
        /// Block breaks per cell.
        breaks: u32,
    },
}

/// Tool statistics, present only for [`ItemCategory::Tool`] items.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDef {
    /// Tool tier, compared against a block's `min_tier`.
    pub tier: u8,
    /// Speed multiplier in the break-time formula.
    pub speed: f32,
    /// Mining area as `(width, height)` of the targeted face.
    pub area: (u32, u32),
    /// Energy source.
    pub energy: ToolEnergy,
}

/// Definition of an item type.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ItemDef {
    /// Stable string id, e.g. `"steel_ingot"`.
    pub id: String,
    /// Human-readable display name.
    pub name: String,
    /// Maximum stack size.
    pub stack: u32,
    /// Category.
    pub category: ItemCategory,
    /// Tool statistics, required for tools and absent otherwise.
    #[serde(default)]
    pub tool: Option<ToolDef>,
    /// Optional one-line description shown in the UI.
    #[serde(default)]
    pub description: String,
}

/// Definition of a crafting or machine recipe.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecipeDef {
    /// Stable string id, e.g. `"smelt_iron"`.
    pub id: String,
    /// Machine that runs this recipe, or `"workbench"` for hand crafting.
    pub machine: String,
    /// Processing time in seconds.
    pub time: f32,
    /// Inputs as `(item_id, count)`.
    pub inputs: Vec<(String, u32)>,
    /// Outputs as `(item_id, count)`.
    pub outputs: Vec<(String, u32)>,
    /// Tech tree node that must be researched before this recipe is available.
    /// Empty means the recipe is always available.
    #[serde(default)]
    pub tech: String,
    /// Electric power draw in kW, if the machine is electric.
    #[serde(default)]
    pub power_kw: f32,
}

/// How a machine is powered.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum MachineEnergy {
    /// Burns solid fuel from its own slot.
    Fuel {
        /// Number of fuel slots.
        slots: u32,
    },
    /// Consumes steam units per second from a pipe network.
    Steam {
        /// Steam units consumed per second while running.
        rate: f32,
    },
    /// Consumes electricity in kW from a cable network.
    Electric {
        /// Kilowatts drawn while running.
        kw: f32,
    },
    /// No power required (hand-cranked or passive structures).
    None,
}

/// Definition of a machine or placeable structure.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineDef {
    /// Stable string id, e.g. `"blast_furnace"`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Footprint in blocks as `(x, y, z)`.
    pub size: (u32, u32, u32),
    /// Power source.
    pub energy: MachineEnergy,
    /// Number of input slots.
    pub input_slots: u32,
    /// Number of output slots.
    pub output_slots: u32,
    /// Items required to build it, as `(item_id, count)`.
    #[serde(default)]
    pub build_cost: Vec<(String, u32)>,
    /// Tech node that unlocks building it. Empty means always available.
    #[serde(default)]
    pub tech: String,
    /// If this machine is part of a cascade, the min and max chain length.
    #[serde(default)]
    pub cascade: Option<(u32, u32)>,
}

/// A node in the team-shared tech tree.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TechDef {
    /// Stable string id, e.g. `"steam_power"`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Era this node belongs to (0..=5).
    pub era: u8,
    /// Parent nodes that must be researched first.
    #[serde(default)]
    pub parents: Vec<String>,
    /// Items that must be delivered to research this node.
    #[serde(default)]
    pub cost: Vec<(String, u32)>,
    /// Machine, recipe or block ids unlocked by this node.
    #[serde(default)]
    pub unlocks: Vec<String>,
}

/// An ore spawn rule inside a layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OreRule {
    /// Block id of the ore.
    pub block: String,
    /// Noise threshold / spawn frequency, 0..1.
    pub frequency: f32,
    /// Vein size range in blocks, `(min, max)`.
    pub vein_size: (u32, u32),
}

/// A hazard active in a layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayerHazard {
    /// Water pockets that flood tunnels when breached.
    Flooding {
        /// Chance per chunk of a pocket.
        pocket_chance: f32,
        /// Maximum water volume per pocket in blocks.
        max_volume: u32,
    },
    /// Methane pockets that drain oxygen and ignite on open flame.
    Gas {
        /// Chance per chunk of a pocket.
        pocket_chance: f32,
    },
    /// Ambient heat that raises body temperature.
    Heat {
        /// Additional degrees above the layer's ambient temperature.
        extra_degrees: f32,
    },
    /// Unsupported spans collapse when mined beneath.
    CaveIn {
        /// Maximum safe span in blocks.
        max_span: u32,
    },
    /// Ambient radiation dose per second.
    Radiation {
        /// Dose per second while in the layer.
        dose_per_sec: f32,
    },
    /// Total darkness; requires a light source.
    Darkness,
}

/// Definition of a depth layer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LayerDef {
    /// Stable string id, e.g. `"granite_depths"`.
    pub id: String,
    /// Depth range below the surface, `(start, end)`.
    pub depth: (u32, u32),
    /// Block that fills most of the layer.
    pub base_block: String,
    /// Secondary filler blocks with their relative share.
    #[serde(default)]
    pub fill_blocks: Vec<(String, f32)>,
    /// Ore spawn rules.
    #[serde(default)]
    pub ores: Vec<OreRule>,
    /// Hazards present in this layer.
    #[serde(default)]
    pub hazards: Vec<LayerHazard>,
    /// Ambient temperature in degrees Celsius.
    #[serde(default)]
    pub ambient_temp: f32,
}

/// Top-level container for every content file, loaded together.
#[derive(Clone, Debug, Default)]
pub struct ContentSet {
    /// All block definitions.
    pub blocks: Vec<BlockDef>,
    /// All item definitions.
    pub items: Vec<ItemDef>,
    /// All recipes.
    pub recipes: Vec<RecipeDef>,
    /// All machines.
    pub machines: Vec<MachineDef>,
    /// All tech tree nodes.
    pub tech: Vec<TechDef>,
    /// All layers, expected ordered shallow to deep.
    pub layers: Vec<LayerDef>,
}
