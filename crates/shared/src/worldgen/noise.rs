//! Seeded noise helpers.
//!
//! Every noise source is derived from the world seed and a fixed domain string,
//! so adding a new noise channel never perturbs existing ones.

use noise::{NoiseFn, Perlin};
use rand_chacha::ChaCha8Rng;
use rand_chacha::rand_core::SeedableRng;

/// Derives a stable 32-bit seed from the world seed and a domain tag.
///
/// Uses FNV-1a so the mapping is fixed across platforms and Rust versions,
/// unlike `DefaultHasher`.
pub fn seed_for(world_seed: u64, domain: &str) -> u32 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in world_seed.to_le_bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    for byte in domain.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    (hash ^ (hash >> 32)) as u32
}

/// A deterministic random generator for a chunk, seeded by world seed + domain
/// + chunk coordinates.
pub fn rng_for_chunk(world_seed: u64, domain: &str, chunk: crate::voxel::ChunkPos) -> ChaCha8Rng {
    let mut hash: u64 = seed_for(world_seed, domain) as u64;
    for value in [chunk.x, chunk.y, chunk.z] {
        hash ^= (value as u32 as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        hash = hash.rotate_left(31).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    }
    // ChaCha8 in rand_chacha 0.10 seeds from a full 32-byte key; expand the
    // 64-bit chunk hash deterministically across it.
    let mut seed = [0u8; 32];
    let mut state = hash;
    for chunk in seed.chunks_mut(8) {
        state = state.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(17) ^ hash;
        chunk.copy_from_slice(&state.to_le_bytes());
    }
    ChaCha8Rng::from_seed(seed)
}

/// Perlin noise fields used by the generator, created once and reused.
pub struct NoiseFields {
    /// Large-scale cave tunnels.
    pub cave_a: Perlin,
    /// Second cave axis, combined with `cave_a` for tube-like caves.
    pub cave_b: Perlin,
    /// Caverns: low-frequency blobs.
    pub cavern: Perlin,
    /// Ore vein density field.
    pub ore: Perlin,
    /// Surface height variation.
    pub surface: Perlin,
}

impl NoiseFields {
    /// Builds the noise fields for a world seed.
    pub fn new(world_seed: u64) -> Self {
        Self {
            cave_a: Perlin::new(seed_for(world_seed, "cave_a")),
            cave_b: Perlin::new(seed_for(world_seed, "cave_b")),
            cavern: Perlin::new(seed_for(world_seed, "cavern")),
            ore: Perlin::new(seed_for(world_seed, "ore")),
            surface: Perlin::new(seed_for(world_seed, "surface")),
        }
    }

    /// Cave carve decision at a world position.
    ///
    /// Two independent 3D fields are combined: where both are near zero the
    /// world is carved, which produces winding tunnels rather than blobs.
    pub fn is_cave(&self, x: f64, y: f64, z: f64) -> bool {
        let a = self.cave_a.get([x * 0.03, y * 0.045, z * 0.03]);
        let b = self.cave_b.get([x * 0.03, y * 0.045, z * 0.03]);
        let tube = a.abs() < 0.075 && b.abs() < 0.075;
        // Caverns are rarer, larger, and only below the topsoil.
        let cavern = self.cavern.get([x * 0.012, y * 0.02, z * 0.012]) > 0.62;
        tube || cavern
    }

    /// Raw ore density at a position, in `-1..1`.
    pub fn ore_density(&self, x: f64, y: f64, z: f64) -> f64 {
        self.ore.get([x * 0.06, y * 0.06, z * 0.06])
    }

    /// Surface height offset in blocks, used to make the camp area flat-ish.
    pub fn surface_offset(&self, x: f64, z: f64) -> f64 {
        self.surface.get([x * 0.01, 0.0, z * 0.01]) * 3.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_for_is_stable_and_domain_separated() {
        assert_eq!(seed_for(42, "cave_a"), seed_for(42, "cave_a"));
        assert_ne!(seed_for(42, "cave_a"), seed_for(42, "cave_b"));
        assert_ne!(seed_for(42, "cave_a"), seed_for(43, "cave_a"));
    }

    #[test]
    fn chunk_rng_is_deterministic_per_chunk() {
        use crate::voxel::ChunkPos;
        let a = rng_for_chunk(7, "ores", ChunkPos::new(1, 2, 3));
        let b = rng_for_chunk(7, "ores", ChunkPos::new(1, 2, 3));
        let c = rng_for_chunk(7, "ores", ChunkPos::new(1, 2, 4));
        // Same chunk -> same stream; different chunk -> different stream.
        assert_eq!(a.clone(), b);
        assert_ne!(a, c);
    }
}
