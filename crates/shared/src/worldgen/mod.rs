//! Deterministic world generation.
//!
//! The mine is a fixed 1024×1024×512 volume. Layer 0 (the surface) is at the
//! top; depth increases downward. Generation is a pure function of
//! `(seed, chunk position, registry)`: the same seed always produces the same
//! chunk on every machine, which is what lets the server store only *modified*
//! chunks.

mod generator;
mod noise;

pub use generator::{WORLD_DEPTH, WORLD_SIZE, WorldGen};
pub use noise::seed_for;
