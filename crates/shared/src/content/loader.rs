//! Loading and validation of the RON content files.
//!
//! Loading is fail-fast: every reference between files is resolved and checked
//! up front, so a typo in `recipes.ron` produces one clear error at startup
//! instead of a confusing failure hours into a session.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use thiserror::Error;

use super::defs::{
    BlockDef, ContentSet, ItemCategory, ItemDef, LayerDef, MachineDef, RecipeDef, TechDef,
};

/// Everything that can go wrong while loading content.
#[derive(Debug, Error)]
pub enum ContentError {
    /// A content file could not be read from disk.
    #[error("failed to read {path}: {source}")]
    Io {
        /// Path that failed.
        path: String,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// A content file was not valid RON.
    #[error("failed to parse {path}: {source}")]
    Parse {
        /// Path that failed.
        path: String,
        /// Underlying error, boxed to keep `ContentError` small.
        #[source]
        source: Box<ron::error::SpannedError>,
    },
    /// Two definitions share an id.
    #[error("duplicate {kind} id {id:?}")]
    DuplicateId {
        /// Kind of definition ("block", "item", ...).
        kind: &'static str,
        /// The repeated id.
        id: String,
    },
    /// A definition referenced an id that does not exist.
    #[error("{kind} {owner:?} references unknown {target_kind} {target:?}")]
    UnknownReference {
        /// Kind of the definition holding the reference.
        kind: &'static str,
        /// Id of the definition holding the reference.
        owner: String,
        /// Kind of the missing target.
        target_kind: &'static str,
        /// The missing id.
        target: String,
    },
    /// A structural problem with the content set.
    #[error("{0}")]
    Invalid(String),
}

/// Loads and validates every content file from a directory.
///
/// Expects `blocks.ron`, `items.ron`, `recipes.ron`, `machines.ron`,
/// `tech_tree.ron` and `layers.ron` inside `dir`.
pub fn load_from_dir(dir: &Path) -> Result<ContentSet, ContentError> {
    let set = ContentSet {
        blocks: read_ron(dir, "blocks.ron")?,
        items: read_ron(dir, "items.ron")?,
        recipes: read_ron(dir, "recipes.ron")?,
        machines: read_ron(dir, "machines.ron")?,
        tech: read_ron(dir, "tech_tree.ron")?,
        layers: read_ron(dir, "layers.ron")?,
    };
    validate(&set)?;
    Ok(set)
}

/// Loads a content set from in-memory strings. Used by tests.
pub fn load_from_strs(
    blocks: &str,
    items: &str,
    recipes: &str,
    machines: &str,
    tech: &str,
    layers: &str,
) -> Result<ContentSet, ContentError> {
    let set = ContentSet {
        blocks: parse_ron(blocks, "blocks.ron")?,
        items: parse_ron(items, "items.ron")?,
        recipes: parse_ron(recipes, "recipes.ron")?,
        machines: parse_ron(machines, "machines.ron")?,
        tech: parse_ron(tech, "tech_tree.ron")?,
        layers: parse_ron(layers, "layers.ron")?,
    };
    validate(&set)?;
    Ok(set)
}

fn read_ron<T: serde::de::DeserializeOwned>(dir: &Path, file: &str) -> Result<T, ContentError> {
    let path = dir.join(file);
    let text = std::fs::read_to_string(&path).map_err(|source| ContentError::Io {
        path: path.display().to_string(),
        source,
    })?;
    parse_ron(&text, &path.display().to_string())
}

fn parse_ron<T: serde::de::DeserializeOwned>(text: &str, path: &str) -> Result<T, ContentError> {
    ron::from_str(text).map_err(|source| ContentError::Parse {
        path: path.to_string(),
        source: Box::new(source),
    })
}

/// Validates a content set, returning every problem found.
///
/// Checks:
/// - no duplicate ids within a file;
/// - every item reference resolves;
/// - every recipe names a machine that exists (or the workbench);
/// - every tech node's parents exist and the graph is acyclic;
/// - every machine's build cost and tech reference resolve;
/// - every block's drops resolve, and at least one block drops each raw item;
/// - every ore block in a layer exists;
/// - every layer's blocks exist.
pub fn validate(set: &ContentSet) -> Result<(), ContentError> {
    let blocks = unique_ids(&set.blocks, "block", |b| &b.id)?;
    let items = unique_ids(&set.items, "item", |i| &i.id)?;
    let machines = unique_ids(&set.machines, "machine", |m| &m.id)?;
    let tech = unique_ids(&set.tech, "tech", |t| &t.id)?;
    let _recipes = unique_ids(&set.recipes, "recipe", |r| &r.id)?;
    let _layers = unique_ids(&set.layers, "layer", |l| &l.id)?;

    // --- blocks -----------------------------------------------------------
    for block in &set.blocks {
        for (item, _count) in &block.drops {
            require(&items, item, "block", &block.id, "item")?;
        }
        if block.hardness <= 0.0 {
            return Err(ContentError::Invalid(format!(
                "block {:?} has non-positive hardness",
                block.id
            )));
        }
    }

    // --- items ------------------------------------------------------------
    for item in &set.items {
        if item.stack == 0 {
            return Err(ContentError::Invalid(format!(
                "item {:?} has stack size 0",
                item.id
            )));
        }
        match (item.category, item.tool.is_some()) {
            (ItemCategory::Tool, false) => {
                return Err(ContentError::Invalid(format!(
                    "tool item {:?} is missing its `tool` block",
                    item.id
                )));
            }
            (ItemCategory::Tool, true) => {}
            (_, true) => {
                return Err(ContentError::Invalid(format!(
                    "non-tool item {:?} must not define `tool`",
                    item.id
                )));
            }
            (_, false) => {}
        }
    }

    // --- machines ---------------------------------------------------------
    for machine in &set.machines {
        for (item, _count) in &machine.build_cost {
            require(&items, item, "machine", &machine.id, "item")?;
        }
        if !machine.tech.is_empty() {
            require(&tech, &machine.tech, "machine", &machine.id, "tech node")?;
        }
        if machine.size.0 == 0 || machine.size.1 == 0 || machine.size.2 == 0 {
            return Err(ContentError::Invalid(format!(
                "machine {:?} has a zero dimension",
                machine.id
            )));
        }
    }

    // --- recipes ----------------------------------------------------------
    for recipe in &set.recipes {
        let machine_known =
            recipe.machine == "workbench" || machines.contains_key(recipe.machine.as_str());
        if !machine_known {
            return Err(ContentError::UnknownReference {
                kind: "recipe",
                owner: recipe.id.clone(),
                target_kind: "machine",
                target: recipe.machine.clone(),
            });
        }
        for (item, count) in &recipe.inputs {
            require(&items, item, "recipe", &recipe.id, "item")?;
            if *count == 0 {
                return Err(ContentError::Invalid(format!(
                    "recipe {:?} has a zero-count input {item:?}",
                    recipe.id
                )));
            }
        }
        if recipe.outputs.is_empty() {
            return Err(ContentError::Invalid(format!(
                "recipe {:?} has no outputs",
                recipe.id
            )));
        }
        for (item, count) in &recipe.outputs {
            require(&items, item, "recipe", &recipe.id, "item")?;
            if *count == 0 {
                return Err(ContentError::Invalid(format!(
                    "recipe {:?} has a zero-count output {item:?}",
                    recipe.id
                )));
            }
        }
        if recipe.time <= 0.0 {
            return Err(ContentError::Invalid(format!(
                "recipe {:?} has non-positive time",
                recipe.id
            )));
        }
        if !recipe.tech.is_empty() {
            require(&tech, &recipe.tech, "recipe", &recipe.id, "tech node")?;
        }
    }

    // --- tech tree --------------------------------------------------------
    for node in &set.tech {
        for parent in &node.parents {
            require(&tech, parent, "tech node", &node.id, "tech node")?;
        }
        for (item, _count) in &node.cost {
            require(&items, item, "tech node", &node.id, "item")?;
        }
        for unlock in &node.unlocks {
            let known = machines.contains_key(unlock.as_str())
                || items.contains_key(unlock.as_str())
                || blocks.contains_key(unlock.as_str())
                || set.recipes.iter().any(|r| &r.id == unlock);
            if !known {
                return Err(ContentError::UnknownReference {
                    kind: "tech node",
                    owner: node.id.clone(),
                    target_kind: "machine, item, block or recipe",
                    target: unlock.clone(),
                });
            }
        }
    }
    detect_tech_cycles(set)?;

    // --- layers -----------------------------------------------------------
    let mut previous_end = 0u32;
    for (index, layer) in set.layers.iter().enumerate() {
        require(&blocks, &layer.base_block, "layer", &layer.id, "block")?;
        for (block, _share) in &layer.fill_blocks {
            require(&blocks, block, "layer", &layer.id, "block")?;
        }
        for ore in &layer.ores {
            require(&blocks, &ore.block, "layer", &layer.id, "block")?;
            if ore.vein_size.0 == 0 || ore.vein_size.0 > ore.vein_size.1 {
                return Err(ContentError::Invalid(format!(
                    "layer {:?} ore {:?} has invalid vein_size {:?}",
                    layer.id, ore.block, ore.vein_size
                )));
            }
        }
        let (start, end) = layer.depth;
        if end <= start {
            return Err(ContentError::Invalid(format!(
                "layer {:?} has empty depth range {:?}",
                layer.id, layer.depth
            )));
        }
        if index > 0 && start != previous_end {
            return Err(ContentError::Invalid(format!(
                "layer {:?} starts at {start} but previous layer ended at {previous_end}; \
                 layers must be contiguous",
                layer.id
            )));
        }
        previous_end = end;
    }

    // --- every raw item must come from somewhere --------------------------
    let mut obtainable: HashSet<&str> = HashSet::new();
    for block in &set.blocks {
        for (item, _) in &block.drops {
            obtainable.insert(item.as_str());
        }
    }
    for recipe in &set.recipes {
        for (item, _) in &recipe.outputs {
            obtainable.insert(item.as_str());
        }
    }
    for item in &set.items {
        if item.category == ItemCategory::Raw && !obtainable.contains(item.id.as_str()) {
            return Err(ContentError::Invalid(format!(
                "raw item {:?} is not dropped by any block and is not crafted; it is a dead end",
                item.id
            )));
        }
    }

    Ok(())
}

fn unique_ids<'a, T>(
    defs: &'a [T],
    kind: &'static str,
    id_of: impl Fn(&'a T) -> &'a String,
) -> Result<HashMap<&'a str, &'a T>, ContentError> {
    let mut map = HashMap::with_capacity(defs.len());
    for def in defs {
        let id = id_of(def);
        if map.insert(id.as_str(), def).is_some() {
            return Err(ContentError::DuplicateId {
                kind,
                id: id.clone(),
            });
        }
    }
    Ok(map)
}

fn require<T>(
    index: &HashMap<&str, &T>,
    target: &str,
    kind: &'static str,
    owner: &str,
    target_kind: &'static str,
) -> Result<(), ContentError> {
    if index.contains_key(target) {
        Ok(())
    } else {
        Err(ContentError::UnknownReference {
            kind,
            owner: owner.to_string(),
            target_kind,
            target: target.to_string(),
        })
    }
}

fn detect_tech_cycles(set: &ContentSet) -> Result<(), ContentError> {
    let by_id: HashMap<&str, &TechDef> = set.tech.iter().map(|t| (t.id.as_str(), t)).collect();
    // 0 = unvisited, 1 = in progress, 2 = done.
    let mut state: HashMap<&str, u8> = HashMap::new();
    fn visit<'a>(
        id: &'a str,
        by_id: &HashMap<&'a str, &'a TechDef>,
        state: &mut HashMap<&'a str, u8>,
    ) -> Result<(), ContentError> {
        match state.get(id) {
            Some(2) => return Ok(()),
            Some(1) => {
                return Err(ContentError::Invalid(format!(
                    "tech tree contains a cycle involving {id:?}"
                )));
            }
            _ => {}
        }
        state.insert(id, 1);
        if let Some(node) = by_id.get(id) {
            for parent in &node.parents {
                visit(parent, by_id, state)?;
            }
        }
        state.insert(id, 2);
        Ok(())
    }
    for node in &set.tech {
        visit(node.id.as_str(), &by_id, &mut state)?;
    }
    Ok(())
}

// Silence unused-import warnings for types only used in signatures above.
#[allow(dead_code)]
fn _type_witness(_: &BlockDef, _: &ItemDef, _: &MachineDef, _: &RecipeDef, _: &LayerDef) {}
