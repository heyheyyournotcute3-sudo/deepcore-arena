//! Tests that the shipped content files load, validate and resolve.

use std::path::Path;

use deepcore_shared::content::loader::load_from_dir;
use deepcore_shared::content::registry::Registry;

fn data_dir() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/data")
        .leak()
}

#[test]
fn shipped_content_loads_and_validates() {
    let set = load_from_dir(data_dir()).expect("shipped content must load and validate");
    assert!(!set.blocks.is_empty());
    assert!(!set.items.is_empty());
    assert!(!set.recipes.is_empty());
    assert!(!set.machines.is_empty());
    assert!(!set.tech.is_empty());
    assert_eq!(set.layers.len(), 6, "all six layers must be present");
}

#[test]
fn shipped_content_resolves_to_registry() {
    let set = load_from_dir(data_dir()).expect("valid content");
    let registry = Registry::from_content(&set).expect("content must resolve");
    assert!(registry.block_count() > 30);
    assert!(registry.item_count() > 80);
    assert!(registry.recipes.len() > 100);
    assert!(registry.tech.len() >= 25);
    assert_ne!(registry.content_hash(), 0);
}

#[test]
fn layers_cover_the_whole_mine() {
    let set = load_from_dir(data_dir()).expect("valid content");
    let registry = Registry::from_content(&set).expect("resolves");
    // Layers must tile 0..512 with no gaps, which validation also enforces.
    assert_eq!(registry.layers.first().map(|l| l.depth.0), Some(0));
    assert_eq!(registry.layers.last().map(|l| l.depth.1), Some(512));
}

#[test]
fn every_tool_tier_is_reachable_by_a_tool() {
    let set = load_from_dir(data_dir()).expect("valid content");
    let registry = Registry::from_content(&set).expect("resolves");
    // The mine's deepest layer needs tier 5; a tier-5 tool must exist.
    let max_tool_tier = registry
        .items
        .iter()
        .filter_map(|i| i.tool.as_ref().map(|t| t.tier))
        .max()
        .unwrap_or(0);
    let max_block_tier = registry
        .blocks
        .iter()
        .map(|b| b.min_tier)
        .filter(|&t| t < 200)
        .max()
        .unwrap_or(0);
    assert!(
        max_tool_tier >= max_block_tier,
        "highest tool tier {max_tool_tier} cannot break blocks needing tier {max_block_tier}"
    );
}

#[test]
fn content_hash_is_stable_across_loads() {
    let a = Registry::from_content(&load_from_dir(data_dir()).expect("valid")).expect("resolves");
    let b = Registry::from_content(&load_from_dir(data_dir()).expect("valid")).expect("resolves");
    assert_eq!(a.content_hash(), b.content_hash());
}
