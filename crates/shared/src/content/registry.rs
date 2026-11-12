//! Compact numeric registries resolved from the content files.
//!
//! Gameplay code looks up `BlockId`/`ItemId`/`MachineId` and reads plain structs;
//! it never touches strings or does map lookups in hot paths.

use std::collections::HashMap;

use super::defs::{ContentSet, ItemCategory, LayerHazard, MachineDef, MachineEnergy, ToolDef};
use crate::voxel::BlockId;

/// Compact id for an item type.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct ItemId(pub u16);

/// Compact id for a machine type.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct MachineId(pub u16);

/// Compact id for a recipe.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RecipeId(pub u16);

/// Compact id for a tech tree node.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TechId(pub u16);

/// Compact id for a layer.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct LayerId(pub u8);

/// An item stack template: an item id and a count.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ItemStack {
    /// The item.
    pub item: ItemId,
    /// How many.
    pub count: u32,
}

/// A resolved recipe, with numeric ids.
#[derive(Clone, Debug)]
pub struct ResolvedRecipe {
    /// Compact id.
    pub id: RecipeId,
    /// Machine that runs it. `None` means hand crafting at the workbench.
    pub machine: Option<MachineId>,
    /// Seconds of processing.
    pub time: f32,
    /// Required inputs.
    pub inputs: Vec<ItemStack>,
    /// Produced outputs.
    pub outputs: Vec<ItemStack>,
    /// Required tech node, if any.
    pub tech: Option<TechId>,
    /// Electric draw in kW.
    pub power_kw: f32,
}

/// A resolved tech node.
#[derive(Clone, Debug)]
pub struct ResolvedTech {
    /// Compact id.
    pub id: TechId,
    /// Display name.
    pub name: String,
    /// Era 0..=5.
    pub era: u8,
    /// Parent node ids.
    pub parents: Vec<TechId>,
    /// Delivery cost.
    pub cost: Vec<ItemStack>,
    /// Unlock names (machines/items/blocks/recipes).
    pub unlocks: Vec<String>,
}

/// A resolved ore rule.
#[derive(Clone, Copy, Debug)]
pub struct ResolvedOre {
    /// Ore block.
    pub block: BlockId,
    /// Frequency 0..1.
    pub frequency: f32,
    /// Vein size range.
    pub vein_size: (u32, u32),
}

/// A resolved depth layer.
#[derive(Clone, Debug)]
pub struct ResolvedLayer {
    /// Compact id.
    pub id: LayerId,
    /// Depth range below the surface.
    pub depth: (u32, u32),
    /// Dominant block.
    pub base_block: BlockId,
    /// Secondary fillers with relative share.
    pub fill_blocks: Vec<(BlockId, f32)>,
    /// Ore rules.
    pub ores: Vec<ResolvedOre>,
    /// Hazards.
    pub hazards: Vec<LayerHazard>,
    /// Ambient temperature.
    pub ambient_temp: f32,
}

/// Every block's gameplay data, indexed by [`BlockId`].
#[derive(Clone, Debug, Default)]
pub struct BlockInfo {
    /// String id, for diagnostics and UI.
    pub name: String,
    /// Seconds to break at tier 1 / speed 1.
    pub hardness: f32,
    /// Minimum tool tier.
    pub min_tier: u8,
    /// Drops as `(item, count)`.
    pub drops: Vec<ItemStack>,
    /// Base colour for the atlas.
    pub color: (f32, f32, f32),
    /// Whether the block blocks movement and light.
    pub solid: bool,
    /// Whether players can place it.
    pub placeable: bool,
    /// Emissive strength.
    pub emissive: f32,
    /// Radiation `(radius, dose_per_sec)`.
    pub radiation: Option<(f32, f32)>,
}

/// Every item's gameplay data, indexed by [`ItemId`].
#[derive(Clone, Debug, Default)]
pub struct ItemInfo {
    /// String id.
    pub name: String,
    /// Display name.
    pub display: String,
    /// Max stack.
    pub stack: u32,
    /// Category.
    pub category: ItemCategory,
    /// Tool stats, if a tool.
    pub tool: Option<ToolDef>,
}

/// The full resolved content database.
///
/// Construct with [`Registry::from_content`], then pass it around as a resource.
#[derive(Clone, Debug)]
pub struct Registry {
    /// Block info indexed by `BlockId.0`.
    pub blocks: Vec<BlockInfo>,
    /// Item info indexed by `ItemId.0`.
    pub items: Vec<ItemInfo>,
    /// Machine definitions indexed by `MachineId.0`.
    pub machines: Vec<MachineDef>,
    /// Resolved recipes.
    pub recipes: Vec<ResolvedRecipe>,
    /// Resolved tech nodes.
    pub tech: Vec<ResolvedTech>,
    /// Resolved layers, shallow to deep.
    pub layers: Vec<ResolvedLayer>,

    block_ids: HashMap<String, BlockId>,
    item_ids: HashMap<String, ItemId>,
    machine_ids: HashMap<String, MachineId>,
    recipe_ids: HashMap<String, RecipeId>,
    tech_ids: HashMap<String, TechId>,
}

impl Registry {
    /// Resolves a validated content set into numeric registries.
    ///
    /// The content set must already have passed [`super::validate`]; this
    /// function returns an error rather than panicking if a reference is still
    /// missing, so it is safe to call on untrusted input.
    pub fn from_content(set: &ContentSet) -> Result<Self, String> {
        let mut block_ids = HashMap::new();
        let mut blocks = Vec::with_capacity(set.blocks.len());
        // Air is always id 0 and is never declared in blocks.ron.
        block_ids.insert("air".to_string(), BlockId::AIR);
        blocks.push(BlockInfo {
            name: "air".to_string(),
            hardness: 0.0,
            min_tier: 0,
            drops: Vec::new(),
            color: (0.0, 0.0, 0.0),
            solid: false,
            placeable: false,
            emissive: 0.0,
            radiation: None,
        });

        for def in &set.blocks {
            let id = BlockId(blocks.len() as u16);
            block_ids.insert(def.id.clone(), id);
            blocks.push(BlockInfo {
                name: def.id.clone(),
                hardness: def.hardness,
                min_tier: def.min_tier,
                drops: Vec::new(), // filled after items are resolved
                color: def.color,
                solid: def.solid,
                placeable: def.placeable,
                emissive: def.emissive,
                radiation: def.radiation,
            });
        }

        let mut item_ids = HashMap::new();
        let mut items = Vec::with_capacity(set.items.len());
        for def in &set.items {
            let id = ItemId(items.len() as u16);
            item_ids.insert(def.id.clone(), id);
            items.push(ItemInfo {
                name: def.id.clone(),
                display: def.name.clone(),
                stack: def.stack,
                category: def.category,
                tool: def.tool,
            });
        }

        let item = |name: &str| -> Result<ItemId, String> {
            item_ids
                .get(name)
                .copied()
                .ok_or_else(|| format!("unknown item {name:?}"))
        };

        // Second pass: resolve drops now that items exist.
        for def in &set.blocks {
            let block_id = block_ids[&def.id];
            let mut drops = Vec::with_capacity(def.drops.len());
            for (name, count) in &def.drops {
                drops.push(ItemStack {
                    item: item(name)?,
                    count: *count,
                });
            }
            blocks[block_id.0 as usize].drops = drops;
        }

        let mut machine_ids = HashMap::new();
        let mut machines = Vec::with_capacity(set.machines.len());
        for def in &set.machines {
            let id = MachineId(machines.len() as u16);
            machine_ids.insert(def.id.clone(), id);
            machines.push(def.clone());
        }

        let mut tech_ids = HashMap::new();
        for def in &set.tech {
            let id = TechId(tech_ids.len() as u16);
            tech_ids.insert(def.id.clone(), id);
        }

        let mut recipes = Vec::with_capacity(set.recipes.len());
        for (index, def) in set.recipes.iter().enumerate() {
            let machine = if def.machine == "workbench" {
                None
            } else {
                Some(*machine_ids.get(&def.machine).ok_or_else(|| {
                    format!(
                        "recipe {:?} names unknown machine {:?}",
                        def.id, def.machine
                    )
                })?)
            };
            let mut inputs = Vec::with_capacity(def.inputs.len());
            for (name, count) in &def.inputs {
                inputs.push(ItemStack {
                    item: item(name)?,
                    count: *count,
                });
            }
            let mut outputs = Vec::with_capacity(def.outputs.len());
            for (name, count) in &def.outputs {
                outputs.push(ItemStack {
                    item: item(name)?,
                    count: *count,
                });
            }
            let tech = if def.tech.is_empty() {
                None
            } else {
                Some(*tech_ids.get(&def.tech).ok_or_else(|| {
                    format!("recipe {:?} names unknown tech node {:?}", def.id, def.tech)
                })?)
            };
            recipes.push(ResolvedRecipe {
                id: RecipeId(index as u16),
                machine,
                time: def.time,
                inputs,
                outputs,
                tech,
                power_kw: def.power_kw,
            });
        }

        let mut tech = Vec::with_capacity(set.tech.len());
        for def in &set.tech {
            let mut parents = Vec::with_capacity(def.parents.len());
            for parent in &def.parents {
                parents.push(
                    *tech_ids.get(parent).ok_or_else(|| {
                        format!("tech {:?} names unknown parent {parent:?}", def.id)
                    })?,
                );
            }
            let mut cost = Vec::with_capacity(def.cost.len());
            for (name, count) in &def.cost {
                cost.push(ItemStack {
                    item: item(name)?,
                    count: *count,
                });
            }
            tech.push(ResolvedTech {
                id: tech_ids[&def.id],
                name: def.name.clone(),
                era: def.era,
                parents,
                cost,
                unlocks: def.unlocks.clone(),
            });
        }

        let mut layers = Vec::with_capacity(set.layers.len());
        for (index, def) in set.layers.iter().enumerate() {
            let mut ores = Vec::with_capacity(def.ores.len());
            for ore in &def.ores {
                ores.push(ResolvedOre {
                    block: *block_ids.get(&ore.block).ok_or_else(|| {
                        format!("layer {:?} names unknown ore {:?}", def.id, ore.block)
                    })?,
                    frequency: ore.frequency,
                    vein_size: ore.vein_size,
                });
            }
            let mut fill_blocks = Vec::with_capacity(def.fill_blocks.len());
            for (name, share) in &def.fill_blocks {
                fill_blocks.push((
                    *block_ids.get(name).ok_or_else(|| {
                        format!("layer {:?} names unknown block {name:?}", def.id)
                    })?,
                    *share,
                ));
            }
            layers.push(ResolvedLayer {
                id: LayerId(index as u8),
                depth: def.depth,
                base_block: *block_ids
                    .get(&def.base_block)
                    .ok_or_else(|| format!("layer {:?} names unknown base block", def.id))?,
                fill_blocks,
                ores,
                hazards: def.hazards.clone(),
                ambient_temp: def.ambient_temp,
            });
        }

        Ok(Self {
            blocks,
            items,
            machines,
            recipes,
            tech,
            layers,
            block_ids,
            item_ids,
            machine_ids,
            recipe_ids: set
                .recipes
                .iter()
                .enumerate()
                .map(|(i, r)| (r.id.clone(), RecipeId(i as u16)))
                .collect(),
            tech_ids,
        })
    }

    /// Looks up a block by string id.
    pub fn block(&self, name: &str) -> Option<BlockId> {
        self.block_ids.get(name).copied()
    }

    /// Looks up an item by string id.
    pub fn item(&self, name: &str) -> Option<ItemId> {
        self.item_ids.get(name).copied()
    }

    /// Looks up a machine by string id.
    pub fn machine(&self, name: &str) -> Option<MachineId> {
        self.machine_ids.get(name).copied()
    }

    /// Looks up a recipe by string id.
    pub fn recipe(&self, name: &str) -> Option<RecipeId> {
        self.recipe_ids.get(name).copied()
    }

    /// Looks up a tech node by string id.
    pub fn tech_node(&self, name: &str) -> Option<TechId> {
        self.tech_ids.get(name).copied()
    }

    /// Block data for an id.
    #[inline]
    pub fn block_info(&self, id: BlockId) -> &BlockInfo {
        &self.blocks[id.0 as usize]
    }

    /// Item data for an id.
    #[inline]
    pub fn item_info(&self, id: ItemId) -> &ItemInfo {
        &self.items[id.0 as usize]
    }

    /// Machine definition for an id.
    #[inline]
    pub fn machine_info(&self, id: MachineId) -> &MachineDef {
        &self.machines[id.0 as usize]
    }

    /// Recipe for an id.
    #[inline]
    pub fn recipe_info(&self, id: RecipeId) -> &ResolvedRecipe {
        &self.recipes[id.0 as usize]
    }

    /// Tech node for an id.
    #[inline]
    pub fn tech_info(&self, id: TechId) -> &ResolvedTech {
        &self.tech[id.0 as usize]
    }

    /// The layer whose depth range contains `depth_below_surface`.
    pub fn layer_at_depth(&self, depth_below_surface: u32) -> Option<&ResolvedLayer> {
        self.layers
            .iter()
            .find(|l| depth_below_surface >= l.depth.0 && depth_below_surface < l.depth.1)
    }

    /// Recipes runnable by a machine.
    pub fn recipes_for_machine(&self, machine: MachineId) -> impl Iterator<Item = &ResolvedRecipe> {
        self.recipes
            .iter()
            .filter(move |r| r.machine == Some(machine))
    }

    /// Hand-crafting recipes (those with no machine).
    pub fn hand_recipes(&self) -> impl Iterator<Item = &ResolvedRecipe> {
        self.recipes.iter().filter(|r| r.machine.is_none())
    }

    /// The energy type of a machine, for power solving.
    pub fn machine_energy(&self, machine: MachineId) -> MachineEnergy {
        self.machine_info(machine).energy
    }

    /// Number of block types, including air.
    pub fn block_count(&self) -> usize {
        self.blocks.len()
    }

    /// Number of item types.
    pub fn item_count(&self) -> usize {
        self.items.len()
    }

    /// Builds a stable content hash over the resolved registries, for the
    /// client/server handshake. Two peers with different content are refused.
    pub fn content_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        for b in &self.blocks {
            b.name.hash(&mut hasher);
            b.hardness.to_bits().hash(&mut hasher);
            b.min_tier.hash(&mut hasher);
            for d in &b.drops {
                d.item.0.hash(&mut hasher);
                d.count.hash(&mut hasher);
            }
        }
        for i in &self.items {
            i.name.hash(&mut hasher);
            i.stack.hash(&mut hasher);
        }
        for r in &self.recipes {
            r.time.to_bits().hash(&mut hasher);
            for i in &r.inputs {
                i.item.0.hash(&mut hasher);
                i.count.hash(&mut hasher);
            }
            for o in &r.outputs {
                o.item.0.hash(&mut hasher);
                o.count.hash(&mut hasher);
            }
        }
        for m in &self.machines {
            m.id.hash(&mut hasher);
            m.size.hash(&mut hasher);
        }
        for t in &self.tech {
            t.name.hash(&mut hasher);
            t.era.hash(&mut hasher);
        }
        for l in &self.layers {
            l.depth.hash(&mut hasher);
            l.base_block.0.hash(&mut hasher);
        }
        hasher.finish()
    }
}
