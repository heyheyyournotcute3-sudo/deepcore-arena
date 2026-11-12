//! Walks the tech tree and prints the raw ore needed to reach each Era.
//!
//! This is the pacing check: it answers "how much mining does Era N actually
//! cost?" on paper, without playing. Run it after every content change.

use std::collections::{HashMap, HashSet};

use deepcore_shared::content::loader::load_from_dir;
use deepcore_shared::content::registry::{ItemId, Registry};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let data_dir = if args.len() > 1 {
        std::path::PathBuf::from(&args[1])
    } else {
        std::path::PathBuf::from("assets/data")
    };

    let set = match load_from_dir(&data_dir) {
        Ok(set) => set,
        Err(err) => {
            eprintln!("content error: {err}");
            std::process::exit(1);
        }
    };
    let registry = match Registry::from_content(&set) {
        Ok(registry) => registry,
        Err(err) => {
            eprintln!("content resolve error: {err}");
            std::process::exit(1);
        }
    };

    println!("Deepcore balance report");
    println!("=======================");
    println!(
        "content: {} blocks, {} items, {} recipes, {} machines, {} tech nodes, {} layers",
        registry.block_count(),
        registry.item_count(),
        registry.recipes.len(),
        registry.machines.len(),
        registry.tech.len(),
        registry.layers.len()
    );
    println!();

    report_layers(&registry);
    report_era_costs(&registry);
    report_recipe_coverage(&registry);
}

fn report_layers(registry: &Registry) {
    println!("Layers");
    println!("------");
    for layer in &registry.layers {
        let (start, end) = layer.depth;
        let ore_names: Vec<&str> = layer
            .ores
            .iter()
            .map(|o| registry.block_info(o.block).name.as_str())
            .collect();
        println!(
            "  {:<16} depth {:>3}..{:<3} base {:<14} ores: {}",
            format!("{:?}", layer.id),
            start,
            end,
            registry.block_info(layer.base_block).name,
            ore_names.join(", ")
        );
    }
    println!();
}

/// For each Era, computes the cumulative raw materials required by all tech
/// nodes of that Era and earlier, fully expanded through the recipe graph.
fn report_era_costs(registry: &Registry) {
    println!("Cumulative tech-tree cost by era");
    println!("--------------------------------");

    // Which items count as "raw" (dug out of the ground).
    let raw_items: HashSet<ItemId> = registry
        .blocks
        .iter()
        .flat_map(|b| b.drops.iter().map(|d| d.item))
        .collect();

    for era in 0u8..=5 {
        // Collect every tech node up to and including this era.
        let nodes: Vec<_> = registry.tech.iter().filter(|t| t.era <= era).collect();
        let mut totals: HashMap<ItemId, f64> = HashMap::new();
        for node in &nodes {
            for cost in &node.cost {
                expand(
                    registry,
                    cost.item,
                    cost.count as f64,
                    &mut totals,
                    &raw_items,
                    &mut HashSet::new(),
                    0,
                );
            }
        }
        let mut rows: Vec<(String, f64)> = totals
            .into_iter()
            .map(|(item, qty)| (registry.item_info(item).display.clone(), qty))
            .collect();
        rows.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        println!("  Era {era}: {} tech nodes", nodes.len());
        for (name, qty) in rows.iter().take(12) {
            println!("      {:<28} {:>8.1}", name, qty);
        }
        println!();
    }
}

/// Recursively expands an item into raw materials through recipes.
fn expand(
    registry: &Registry,
    item: ItemId,
    amount: f64,
    totals: &mut HashMap<ItemId, f64>,
    raw_items: &HashSet<ItemId>,
    visiting: &mut HashSet<ItemId>,
    depth: usize,
) {
    // Depth guard: content validation already rejects cycles, but never loop.
    if depth > 24 || visiting.contains(&item) {
        *totals.entry(item).or_insert(0.0) += amount;
        return;
    }

    // If it is raw, count it directly.
    if raw_items.contains(&item) {
        *totals.entry(item).or_insert(0.0) += amount;
        return;
    }

    // Find the cheapest recipe producing it.
    let recipe = registry
        .recipes
        .iter()
        .filter(|r| r.outputs.iter().any(|o| o.item == item))
        .min_by(|a, b| {
            a.time
                .partial_cmp(&b.time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

    let Some(recipe) = recipe else {
        // No recipe: it must be a base item (tool, gear, etc.). Count it as-is.
        *totals.entry(item).or_insert(0.0) += amount;
        return;
    };

    let produced = recipe
        .outputs
        .iter()
        .find(|o| o.item == item)
        .map(|o| o.count as f64)
        .unwrap_or(1.0)
        .max(1.0);
    let batches = amount / produced;

    visiting.insert(item);
    for input in &recipe.inputs {
        expand(
            registry,
            input.item,
            input.count as f64 * batches,
            totals,
            raw_items,
            visiting,
            depth + 1,
        );
    }
    visiting.remove(&item);
}

fn report_recipe_coverage(registry: &Registry) {
    println!("Machine coverage");
    println!("----------------");
    let mut per_machine: HashMap<&str, usize> = HashMap::new();
    for recipe in &registry.recipes {
        let key = match recipe.machine {
            Some(machine) => registry.machine_info(machine).id.as_str(),
            None => "workbench (hand)",
        };
        *per_machine.entry(key).or_insert(0) += 1;
    }
    let mut rows: Vec<_> = per_machine.into_iter().collect();
    rows.sort_by_key(|(name, _)| *name);
    for (name, count) in rows {
        println!("  {name:<24} {count:>3} recipes");
    }

    // Machines with no recipe at all are content gaps worth flagging.
    let used: HashSet<&str> = registry
        .recipes
        .iter()
        .filter_map(|r| r.machine.map(|m| registry.machine_info(m).id.as_str()))
        .collect();
    let unused: Vec<&str> = registry
        .machines
        .iter()
        .map(|m| m.id.as_str())
        .filter(|id| !used.contains(id))
        .collect();
    if !unused.is_empty() {
        println!();
        println!(
            "  machines with no recipe (passive/structure or gap): {}",
            unused.join(", ")
        );
    }
}
