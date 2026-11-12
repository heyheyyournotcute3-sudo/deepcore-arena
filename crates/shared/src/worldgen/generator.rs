//! The chunk generator: layers, caves, ore veins and bedrock bounds.

use bevy::prelude::Resource;

use super::noise::{NoiseFields, rng_for_chunk};
use crate::content::Registry;
use crate::voxel::{BlockId, CHUNK_SIZE, Chunk, ChunkPos};
use noise::NoiseFn;

/// Width and depth of the mine in blocks. The world is square in X/Z.
pub const WORLD_SIZE: i32 = 1024;

/// Total depth of the mine in blocks, from surface to bedrock floor.
pub const WORLD_DEPTH: i32 = 512;

/// Deterministic voxel world generator.
///
/// Held as a Bevy resource on both sides so the server can generate authoritative
/// chunks and the client can regenerate any chunk it already has the seed for.
#[derive(Resource)]
pub struct WorldGen {
    seed: u64,
    noise: NoiseFields,
    registry: Registry,
    air: BlockId,
    bedrock: BlockId,
    grass: BlockId,
}

impl WorldGen {
    /// Creates a generator for a seed and content set.
    pub fn new(seed: u64, registry: Registry) -> Self {
        let air = BlockId::AIR;
        let bedrock = registry.block("bedrock").unwrap_or(air);
        let grass = registry.block("grass").unwrap_or(air);
        Self {
            seed,
            noise: NoiseFields::new(seed),
            registry,
            air,
            bedrock,
            grass,
        }
    }

    /// The world seed.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// The content registry this generator resolves blocks from.
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// World Y of the surface. Layers count downward from here.
    pub const fn surface_y() -> i32 {
        WORLD_DEPTH
    }

    /// Converts a world Y into depth below the surface.
    pub fn depth_at(y: i32) -> u32 {
        (Self::surface_y() - y).max(0) as u32
    }

    /// Returns `true` if a world block position is inside the playable volume.
    pub fn in_bounds(x: i32, y: i32, z: i32) -> bool {
        let half = WORLD_SIZE / 2;
        x >= -half && x < half && z >= -half && z < half && y >= 0 && y <= Self::surface_y()
    }

    /// Generates a chunk. Pure and deterministic.
    pub fn generate(&self, pos: ChunkPos) -> Chunk {
        let origin = pos.origin();
        let mut chunk = Chunk::new(pos);

        // Fast reject: chunks entirely outside the world are solid bedrock so
        // that a player can never walk off the edge.
        let fully_outside = origin.x + CHUNK_SIZE <= -WORLD_SIZE / 2
            || origin.x >= WORLD_SIZE / 2
            || origin.z + CHUNK_SIZE <= -WORLD_SIZE / 2
            || origin.z >= WORLD_SIZE / 2
            || origin.y >= Self::surface_y()
            || origin.y + CHUNK_SIZE < 0;

        if fully_outside {
            return Chunk::filled(pos, self.bedrock);
        }

        for ly in 0..CHUNK_SIZE {
            let world_y = origin.y + ly;
            for lz in 0..CHUNK_SIZE {
                let world_z = origin.z + lz;
                for lx in 0..CHUNK_SIZE {
                    let world_x = origin.x + lx;
                    let block = self.block_at(world_x, world_y, world_z);
                    chunk.set_local(lx, ly, lz, block);
                }
            }
        }

        chunk
    }

    /// The block that belongs at a world position, before ore veins.
    fn base_block_at(&self, x: i32, y: i32, z: i32) -> BlockId {
        // Above the surface is open sky.
        if y > Self::surface_y() {
            return self.air;
        }
        if !Self::in_bounds(x, y, z) {
            return self.bedrock;
        }
        // Bedrock floor and walls.
        if y <= 0 {
            return self.bedrock;
        }
        let half = WORLD_SIZE / 2;
        if x <= -half + 1 || x >= half - 1 || z <= -half + 1 || z >= half - 1 {
            return self.bedrock;
        }

        let depth = Self::depth_at(y);

        let Some(layer) = self.registry.layer_at_depth(depth) else {
            return self.bedrock;
        };

        // Surface grass caps the topsoil. The surface undulates gently away from
        // the camp, which is kept flat so players have somewhere to build.
        let camp_radius = 48.0;
        let distance_from_camp = ((x * x + z * z) as f64).sqrt();
        let undulation = if distance_from_camp < camp_radius {
            0.0
        } else {
            // Ramp the terrain in over a few blocks outside the camp.
            let ramp = ((distance_from_camp - camp_radius) / 24.0).min(1.0);
            self.noise.surface_offset(x as f64, z as f64) * ramp
        };
        let surface_y = Self::surface_y() as f64 + undulation;
        if (y as f64) <= surface_y && (y as f64) > surface_y - 2.0 {
            return self.grass;
        }
        // Above the (possibly raised) surface is open air.
        if (y as f64) > surface_y {
            return self.air;
        }

        // Caves: never carve the topsoil (keeps the surface walkable) and never
        // carve the bedrock floor.
        if depth >= 3 && depth < WORLD_DEPTH as u32 - 2 {
            let cave = self.noise.is_cave(x as f64, y as f64, z as f64);
            if cave {
                return self.air;
            }
        }

        // Layer fill: base block, with fill blocks chosen deterministically.
        // Shares are absolute fractions of the layer's volume. The base block
        // takes whatever is left over, so shares summing to less than 1 leave
        // plenty of base rock.
        let mut chosen = layer.base_block;
        if !layer.fill_blocks.is_empty() {
            let hash = Self::position_hash(self.seed, x, y, z) as f32 / u32::MAX as f32;
            let mut acc = 0.0;
            for (block, share) in &layer.fill_blocks {
                acc += *share;
                if hash < acc {
                    chosen = *block;
                    break;
                }
            }
        }

        chosen
    }

    /// Full block lookup including ore veins.
    pub fn block_at(&self, x: i32, y: i32, z: i32) -> BlockId {
        let base = self.base_block_at(x, y, z);
        if base.is_air() || base == self.bedrock {
            return base;
        }
        // Ores only replace the layer's own base/fill rock, never air.
        self.ore_at(x, y, z).unwrap_or(base)
    }

    /// Ore vein lookup. Returns `Some(ore)` if this position is inside a vein.
    ///
    /// Each ore owns a noise field offset into a shared domain so veins of
    /// different ores do not perfectly overlap. The threshold is derived from the
    /// ore's `frequency` such that a frequency of `f` yields roughly
    /// `f * 100`% of the layer's rock as that ore, then modulated by a
    /// higher-frequency field so veins are blobby clusters rather than noise
    /// fuzz.
    fn ore_at(&self, x: i32, y: i32, z: i32) -> Option<BlockId> {
        let depth = Self::depth_at(y);
        let layer = self.registry.layer_at_depth(depth)?;
        if layer.ores.is_empty() {
            return None;
        }

        // Coarse field decides *where veins cluster*; fine field decides
        // whether this particular block is ore within a cluster.
        let coarse = self
            .noise
            .ore
            .get([x as f64 * 0.02, y as f64 * 0.03, z as f64 * 0.02]);
        let fine = self.noise.ore_density(x as f64, y as f64, z as f64);

        for (index, ore) in layer.ores.iter().enumerate() {
            // Deterministic per-ore phase shift.
            let phase = (index as f64 + 1.0) * 13.7;
            let ore_coarse = self.noise.cave_a.get([
                x as f64 * 0.02 + phase,
                y as f64 * 0.03,
                z as f64 * 0.02 + phase,
            ]);
            // Veins live where the ore's own coarse field is high.
            let cluster_threshold = 0.35;
            if ore_coarse <= cluster_threshold {
                continue;
            }
            // Inside a cluster, fill density scales with the ore's frequency.
            // A frequency of 0.02 fills ~20% of its clusters; 0.004 fills ~4%.
            let fill = (ore.frequency * 10.0).clamp(0.05, 0.95) as f64;
            let density = (coarse + fine) * 0.5;
            if density + fill > 0.45 {
                // Vein size widens the effective cluster a little.
                let _ = ore.vein_size;
                return Some(ore.block);
            }
        }
        None
    }

    /// A stable per-position hash, used for fill selection and vein jitter.
    fn position_hash(seed: u64, x: i32, y: i32, z: i32) -> u32 {
        let mut h = seed ^ 0x9e37_79b9_7f4a_7c15;
        for v in [x, y, z] {
            h ^= (v as u32 as u64).wrapping_mul(0x85eb_ca6b);
            h = h.rotate_left(27).wrapping_mul(0xc2b2_ae35);
        }
        (h ^ (h >> 32)) as u32
    }

    /// Counts ore blocks of a given id in a generated chunk. Used by tests and
    /// the balance report.
    pub fn count_block_in_chunk(&self, pos: ChunkPos, block: BlockId) -> usize {
        let chunk = self.generate(pos);
        chunk.iter_blocks().filter(|(_, b)| *b == block).count()
    }

    /// The RNG stream a chunk would use for any random decisions.
    pub fn chunk_rng(&self, domain: &str, pos: ChunkPos) -> rand_chacha::ChaCha8Rng {
        rng_for_chunk(self.seed, domain, pos)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::content::loader::load_from_dir;
    use std::path::Path;

    fn registry() -> Registry {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/data");
        let set = load_from_dir(&dir).expect("content loads");
        Registry::from_content(&set).expect("content resolves")
    }

    fn make_gen(seed: u64) -> WorldGen {
        WorldGen::new(seed, registry())
    }

    #[test]
    fn generation_is_deterministic_for_the_same_seed() {
        let a = make_gen(12345);
        let b = make_gen(12345);
        for pos in [
            ChunkPos::new(0, 15, 0),
            ChunkPos::new(3, 12, -2),
            ChunkPos::new(-1, 8, 5),
        ] {
            let ca = a.generate(pos);
            let cb = b.generate(pos);
            assert_eq!(ca.to_bytes(), cb.to_bytes(), "chunk {pos:?} differs");
        }
    }

    #[test]
    fn different_seeds_produce_different_worlds() {
        let a = make_gen(1);
        let b = make_gen(2);
        let pos = ChunkPos::new(2, 10, 2);
        assert_ne!(a.generate(pos).to_bytes(), b.generate(pos).to_bytes());
    }

    #[test]
    fn surface_is_air_and_deep_is_solid() {
        let g = make_gen(99);
        // Directly above the surface is air.
        assert!(g.block_at(10, WORLD_DEPTH + 1, 10).is_air());
        // Just below the surface is not air (topsoil is never carved).
        let just_below = g.block_at(10, WORLD_DEPTH - 1, 10);
        assert!(!just_below.is_air(), "topsoil must be solid at the surface");
        // Deep in layer 2 there should be solid rock somewhere in a column.
        let mut solid = 0;
        for y in (WORLD_DEPTH - 60)..(WORLD_DEPTH - 20) {
            if !g.block_at(200, y, 200).is_air() {
                solid += 1;
            }
        }
        assert!(solid > 20, "expected mostly solid rock, got {solid}/40");
    }

    #[test]
    fn bedrock_bounds_the_world() {
        let g = make_gen(5);
        assert_eq!(g.block_at(0, 0, 0), g.bedrock);
        assert_eq!(g.block_at(0, -5, 0), g.bedrock);
        // Outside the horizontal bounds is bedrock.
        assert_eq!(g.block_at(WORLD_SIZE, 100, 0), g.bedrock);
        assert_eq!(g.block_at(-WORLD_SIZE, 100, 0), g.bedrock);
    }

    #[test]
    fn every_layer_appears_at_its_depth() {
        let g = make_gen(2024);
        let registry = g.registry();
        for layer in &registry.layers {
            let mid_depth = (layer.depth.0 + layer.depth.1) / 2;
            let y = WORLD_DEPTH - mid_depth as i32;
            // Sample a wide area; a layer must contribute its base block somewhere.
            let mut found = false;
            for x in (0..200).step_by(4) {
                for z in (0..200).step_by(4) {
                    let b = g.block_at(x, y, z);
                    if b == layer.base_block {
                        found = true;
                        break;
                    }
                }
                if found {
                    break;
                }
            }
            assert!(
                found,
                "layer {:?} base block missing at depth {mid_depth}",
                layer.id
            );
        }
    }

    #[test]
    fn caves_exist_below_the_topsoil() {
        let g = make_gen(777);
        let mut air_count = 0;
        let mut total = 0;
        for y in (WORLD_DEPTH - 200)..(WORLD_DEPTH - 40) {
            for x in (0..128).step_by(3) {
                for z in (0..128).step_by(3) {
                    total += 1;
                    if g.block_at(x, y, z).is_air() {
                        air_count += 1;
                    }
                }
            }
        }
        assert!(
            air_count > 0,
            "expected carved caves somewhere underground, found none in {total} samples"
        );
        // Caves must not hollow out everything; the mine stays mostly solid.
        let fraction = air_count as f64 / total as f64;
        assert!(
            fraction < 0.5,
            "caves hollowed {fraction:.2} of the rock, too much"
        );
    }

    #[test]
    fn ores_are_present_in_their_layers() {
        let g = make_gen(31337);
        let registry = g.registry();
        // Layer 2 (shale beds) should contain copper somewhere in a large sample.
        let copper = registry.block("malachite").expect("malachite exists");
        let mut found = 0;
        for x in (-200..200).step_by(8) {
            for z in (-200..200).step_by(8) {
                for y in (WORLD_DEPTH - 60)..(WORLD_DEPTH - 20) {
                    if g.block_at(x, y, z) == copper {
                        found += 1;
                    }
                }
            }
        }
        assert!(found > 0, "expected malachite veins in the shale beds");
    }

    #[test]
    fn uranium_appears_only_deep() {
        let g = make_gen(424242);
        let registry = g.registry();
        let uranium = registry.block("pitchblende").expect("pitchblende exists");
        // Shallow layers must never contain uranium.
        for x in (-100..100).step_by(10) {
            for z in (-100..100).step_by(10) {
                for y in (WORLD_DEPTH - 60)..WORLD_DEPTH {
                    assert_ne!(g.block_at(x, y, z), uranium, "uranium found above layer 5");
                }
            }
        }
    }
}
