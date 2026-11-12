//! Generates the block texture atlas from `assets/data/blocks.ron`.
//!
//! The game builds its atlas at runtime from the same code path, so this tool is
//! for inspection and for shipping a prebuilt PNG. Running it requires no art
//! assets: every tile is generated from a block's colour and pattern.

use std::path::PathBuf;

use deepcore_shared::atlas::build_atlas;
use deepcore_shared::content::loader::load_from_dir;
use deepcore_shared::content::registry::Registry;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut data_dir = PathBuf::from("assets/data");
    let mut out_path = PathBuf::from("assets/atlas.png");
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--data" if index + 1 < args.len() => {
                data_dir = PathBuf::from(&args[index + 1]);
                index += 2;
            }
            "--out" if index + 1 < args.len() => {
                out_path = PathBuf::from(&args[index + 1]);
                index += 2;
            }
            "-h" | "--help" => {
                println!("usage: atlas_gen [--data <dir>] [--out <png>]");
                return;
            }
            other => {
                eprintln!("unknown argument {other:?}");
                std::process::exit(2);
            }
        }
    }

    let set = match load_from_dir(&data_dir) {
        Ok(set) => set,
        Err(err) => {
            eprintln!("failed to load content from {}: {err}", data_dir.display());
            std::process::exit(1);
        }
    };
    let registry = match Registry::from_content(&set) {
        Ok(registry) => registry,
        Err(err) => {
            eprintln!("failed to resolve content: {err}");
            std::process::exit(1);
        }
    };

    let atlas = build_atlas(&registry);
    let png = atlas.image.to_png();

    if let Some(parent) = out_path.parent()
        && !parent.as_os_str().is_empty()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        eprintln!("could not create {}: {err}", parent.display());
        std::process::exit(1);
    }

    if let Err(err) = std::fs::write(&out_path, &png) {
        eprintln!("could not write {}: {err}", out_path.display());
        std::process::exit(1);
    }

    println!(
        "wrote {} ({}×{} px, {} block tiles, {} bytes)",
        out_path.display(),
        atlas.image.width,
        atlas.image.height,
        atlas.uvs.len(),
        png.len()
    );
}
