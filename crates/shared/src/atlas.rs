//! Procedural block texture atlas generation.
//!
//! The game ships with no art files. Each block's `color` and `pattern` in
//! `blocks.ron` are expanded into a 16×16 tile, and the tiles are packed into a
//! single atlas image. Both the offline tool (`tools/atlas_gen`) and the client
//! use this module, so the atlas the client renders is byte-identical to the one
//! the tool writes to disk.

use crate::content::defs::Pattern;
use crate::content::registry::Registry;
use crate::voxel::BlockId;

/// Edge length of a single tile, in pixels.
pub const TILE_SIZE: u32 = 16;

/// Tiles per atlas row.
pub const ATLAS_COLUMNS: u32 = 16;

/// An RGBA8 image.
#[derive(Clone, Debug)]
pub struct Image {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// RGBA pixels, row-major, 4 bytes each.
    pub pixels: Vec<u8>,
}

impl Image {
    /// Creates a transparent image.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            pixels: vec![0; (width * height * 4) as usize],
        }
    }

    /// Writes a pixel.
    pub fn set(&mut self, x: u32, y: u32, rgba: [u8; 4]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let index = ((y * self.width + x) * 4) as usize;
        self.pixels[index..index + 4].copy_from_slice(&rgba);
    }

    /// Reads a pixel.
    pub fn get(&self, x: u32, y: u32) -> [u8; 4] {
        let index = ((y * self.width + x) * 4) as usize;
        [
            self.pixels[index],
            self.pixels[index + 1],
            self.pixels[index + 2],
            self.pixels[index + 3],
        ]
    }

    /// Encodes the image as a PNG byte stream.
    pub fn to_png(&self) -> Vec<u8> {
        encode_png(self.width, self.height, &self.pixels)
    }
}

/// The UV rectangle of a block's tile inside the atlas, in 0..1 texture space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileUv {
    /// Left edge.
    pub u0: f32,
    /// Top edge.
    pub v0: f32,
    /// Right edge.
    pub u1: f32,
    /// Bottom edge.
    pub v1: f32,
}

/// A generated atlas plus the lookup from block id to tile.
#[derive(Clone, Debug)]
pub struct Atlas {
    /// The atlas image.
    pub image: Image,
    /// UV rectangles indexed by `BlockId.0`.
    pub uvs: Vec<TileUv>,
}

/// Builds the full atlas for a registry.
pub fn build_atlas(registry: &Registry) -> Atlas {
    let block_count = registry.block_count().max(1) as u32;
    let rows = block_count.div_ceil(ATLAS_COLUMNS).max(1);
    let mut image = Image::new(ATLAS_COLUMNS * TILE_SIZE, rows * TILE_SIZE);
    let mut uvs = Vec::with_capacity(block_count as usize);

    for index in 0..block_count {
        let col = index % ATLAS_COLUMNS;
        let row = index / ATLAS_COLUMNS;
        let info = &registry.blocks[index as usize];

        if index == 0 {
            // Air renders as a fully transparent tile.
            fill_tile(&mut image, col, row, [0, 0, 0, 0]);
        } else {
            let pattern = pattern_for(registry, BlockId(index as u16));
            draw_tile(
                &mut image,
                col,
                row,
                info.color,
                pattern,
                info.emissive,
                index,
            );
        }

        let atlas_w = image.width as f32;
        let atlas_h = image.height as f32;
        uvs.push(TileUv {
            u0: (col * TILE_SIZE) as f32 / atlas_w,
            v0: (row * TILE_SIZE) as f32 / atlas_h,
            u1: ((col + 1) * TILE_SIZE) as f32 / atlas_w,
            v1: ((row + 1) * TILE_SIZE) as f32 / atlas_h,
        });
    }

    Atlas { image, uvs }
}

/// Looks up the pattern for a block by re-deriving it from the name heuristics
/// stored on the registry's block info.
///
/// The registry keeps colour and emissive but not the pattern, so the pattern is
/// recovered from the block's name. This keeps the atlas reproducible from the
/// registry alone, which is what the client has.
fn pattern_for(registry: &Registry, block: BlockId) -> Pattern {
    let name = registry.block_info(block).name.as_str();
    if name.ends_with("_seam")
        || name.contains("vein")
        || name.contains("malachite")
        || name.contains("cassiterite")
        || name.contains("hematite")
        || name.contains("galena")
        || name.contains("sphalerite")
        || name.contains("deposit")
        || name.contains("quartz")
        || name.contains("bauxite")
        || name.contains("salt")
        || name.contains("pentlandite")
        || name.contains("wolframite")
        || name.contains("chromite")
        || name.contains("fluorite")
        || name.contains("zircon")
        || name.contains("pitchblende")
        || name.contains("monazite")
    {
        // Ore blocks: bright flecks. Uranium gets a green glow.
        let info = registry.block_info(block);
        let fleck = if name.contains("pitchblende") {
            [0.35, 0.95, 0.30]
        } else {
            // Tint the fleck toward the block's own highlight colour.
            let (r, g, b) = info.color;
            [(r * 1.6).min(1.0), (g * 1.6).min(1.0), (b * 1.5).min(1.0)]
        };
        return Pattern::OreFleck(fleck);
    }
    if name == "core_crystal" {
        return Pattern::Glow;
    }
    if name == "log" || name == "shale" || name == "gneiss" || name.contains("support") {
        return Pattern::Grain;
    }
    if name == "gravel"
        || name == "clay"
        || name == "leaves"
        || name == "concrete"
        || name == "limestone"
    {
        return Pattern::Blotch;
    }
    if name == "glass_block" || name == "steel_support" || name == "lead_shielding" {
        return Pattern::Solid;
    }
    Pattern::Speckle
}

fn fill_tile(image: &mut Image, col: u32, row: u32, rgba: [u8; 4]) {
    for y in 0..TILE_SIZE {
        for x in 0..TILE_SIZE {
            image.set(col * TILE_SIZE + x, row * TILE_SIZE + y, rgba);
        }
    }
}

/// Draws one 16×16 tile according to its pattern.
fn draw_tile(
    image: &mut Image,
    col: u32,
    row: u32,
    color: (f32, f32, f32),
    pattern: Pattern,
    emissive: f32,
    seed: u32,
) {
    let base = to_rgba(color, 1.0);
    let mut rng = TileRng::new(seed.wrapping_mul(2_654_435_761).wrapping_add(1));

    for y in 0..TILE_SIZE {
        for x in 0..TILE_SIZE {
            let noise = rng.next_f32();
            let px = match pattern {
                Pattern::Solid => base,
                Pattern::Speckle => {
                    // Fine per-pixel variation, like stone.
                    let v = 0.88 + noise * 0.24;
                    scale_rgba(base, v)
                }
                Pattern::Blotch => {
                    // Coarser clumps: quantise the noise so patches are visible.
                    let v = if noise > 0.72 {
                        1.18
                    } else if noise < 0.22 {
                        0.82
                    } else {
                        1.0
                    };
                    scale_rgba(base, v)
                }
                Pattern::Grain => {
                    // Vertical banding, like wood or shale bedding.
                    let band = ((x as f32 / 3.0).sin() * 0.5 + 0.5) * 0.18 + 0.91;
                    scale_rgba(base, band * (0.96 + noise * 0.08))
                }
                Pattern::OreFleck(fleck) => {
                    // Bright specks of ore in the host rock.
                    let fleck_rgba = to_rgba((fleck[0], fleck[1], fleck[2]), 1.0);
                    if noise > 0.86 {
                        blend_rgba(scale_rgba(base, 0.9), fleck_rgba, 0.85)
                    } else {
                        scale_rgba(base, 0.9 + noise * 0.16)
                    }
                }
                Pattern::Glow => {
                    // Bright centre falling off toward the edges.
                    let dx = x as f32 - 7.5;
                    let dy = y as f32 - 7.5;
                    let d = (dx * dx + dy * dy).sqrt() / 11.0;
                    let v = (1.35 - d * 0.7).clamp(0.7, 1.5);
                    scale_rgba(base, v)
                }
            };

            // Emissive blocks get their colour lifted so they read as lit.
            let px = if emissive > 0.0 {
                let boost = 1.0 + emissive * 0.9;
                scale_rgba(px, boost)
            } else {
                px
            };

            image.set(col * TILE_SIZE + x, row * TILE_SIZE + y, px);
        }
    }
}

fn to_rgba(color: (f32, f32, f32), alpha: f32) -> [u8; 4] {
    let convert = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
    [
        convert(color.0),
        convert(color.1),
        convert(color.2),
        convert(alpha),
    ]
}

fn scale_rgba(rgba: [u8; 4], factor: f32) -> [u8; 4] {
    let scale = |c: u8| ((c as f32 * factor).clamp(0.0, 255.0)).round() as u8;
    [scale(rgba[0]), scale(rgba[1]), scale(rgba[2]), rgba[3]]
}

fn blend_rgba(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    let mix = |x: u8, y: u8| ((x as f32 * (1.0 - t)) + (y as f32 * t)).round() as u8;
    [
        mix(a[0], b[0]),
        mix(a[1], b[1]),
        mix(a[2], b[2]),
        mix(a[3], b[3]),
    ]
}

/// A tiny deterministic PRNG so atlas generation never depends on `rand`.
struct TileRng(u32);

impl TileRng {
    fn new(seed: u32) -> Self {
        Self(seed | 1)
    }

    fn next_f32(&mut self) -> f32 {
        // xorshift32
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1 << 24) as f32
    }
}

/// Minimal PNG encoder (8-bit RGBA, no interlacing).
///
/// Written by hand so the project needs no image crate for its build tool.
fn encode_png(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.push(8); // bit depth
    ihdr.push(6); // colour type: RGBA
    ihdr.push(0); // compression
    ihdr.push(0); // filter
    ihdr.push(0); // interlace
    write_chunk(&mut out, b"IHDR", &ihdr);

    // Raw scanlines, each prefixed with filter type 0 (None).
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        let start = y * width as usize * 4;
        raw.extend_from_slice(&pixels[start..start + width as usize * 4]);
    }
    write_chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    write_chunk(&mut out, b"IEND", &[]);
    out
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

/// zlib stream using stored (uncompressed) deflate blocks. Valid, if larger
/// than a real compressor; the atlas is a build artifact, not shipped gameplay
/// data.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0x78); // CMF: deflate, 32K window
    out.push(0x01); // FLG: no dict, fastest

    let mut offset = 0;
    while offset < data.len() {
        let chunk_len = (data.len() - offset).min(65535);
        let is_last = offset + chunk_len >= data.len();
        out.push(if is_last { 1 } else { 0 });
        out.extend_from_slice(&(chunk_len as u16).to_le_bytes());
        out.extend_from_slice(&(!(chunk_len as u16)).to_le_bytes());
        out.extend_from_slice(&data[offset..offset + chunk_len]);
        offset += chunk_len;
    }
    if data.is_empty() {
        out.push(1);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(!0u16).to_le_bytes());
    }

    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xffff_ffff;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
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

    #[test]
    fn atlas_has_a_tile_for_every_block() {
        let registry = registry();
        let atlas = build_atlas(&registry);
        assert_eq!(atlas.uvs.len(), registry.block_count());
        assert_eq!(atlas.image.width % TILE_SIZE, 0);
        assert_eq!(atlas.image.height % TILE_SIZE, 0);
    }

    #[test]
    fn air_tile_is_transparent() {
        let registry = registry();
        let atlas = build_atlas(&registry);
        let px = atlas.image.get(0, 0);
        assert_eq!(px[3], 0, "air must be fully transparent");
    }

    #[test]
    fn solid_blocks_are_opaque_and_coloured() {
        let registry = registry();
        let atlas = build_atlas(&registry);
        let granite = registry.block("granite").expect("granite exists");
        let uv = atlas.uvs[granite.0 as usize];
        let px_x = (uv.u0 * atlas.image.width as f32) as u32 + 4;
        let px_y = (uv.v0 * atlas.image.height as f32) as u32 + 4;
        let px = atlas.image.get(px_x, px_y);
        assert_eq!(px[3], 255, "solid blocks must be opaque");
        // Granite is grey-brown: channels should be reasonably close together.
        let max = px[..3].iter().max().copied().unwrap_or(0) as i32;
        let min = px[..3].iter().min().copied().unwrap_or(0) as i32;
        assert!(
            max - min < 60,
            "granite should be roughly neutral, got {px:?}"
        );
    }

    #[test]
    fn uv_rectangles_are_within_the_image_and_tile_sized() {
        let registry = registry();
        let atlas = build_atlas(&registry);
        let tile_u = TILE_SIZE as f32 / atlas.image.width as f32;
        for uv in &atlas.uvs {
            assert!(uv.u0 >= 0.0 && uv.u1 <= 1.0 + 1e-6);
            assert!(uv.v0 >= 0.0 && uv.v1 <= 1.0 + 1e-6);
            assert!((uv.u1 - uv.u0 - tile_u).abs() < 1e-6);
        }
    }

    #[test]
    fn png_encoding_produces_a_valid_header() {
        let registry = registry();
        let atlas = build_atlas(&registry);
        let png = atlas.image.to_png();
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        assert!(png.len() > 100);
        // IHDR chunk follows the signature.
        assert_eq!(&png[12..16], b"IHDR");
    }

    #[test]
    fn crc32_matches_known_value() {
        // CRC32 of "123456789" is 0xCBF43926.
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926);
    }

    #[test]
    fn adler32_matches_known_value() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn atlas_generation_is_deterministic() {
        let registry = registry();
        let a = build_atlas(&registry);
        let b = build_atlas(&registry);
        assert_eq!(a.image.pixels, b.image.pixels);
    }
}
