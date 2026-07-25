//! Texture atlas building for the viewport renderer and OBJ exporter.
//!
//! Two modes:
//!
//! - **Procedural** (`TileSet::build`) — deterministic noise-coloured tiles
//!   derived from the block name. Used as a fallback when no Minecraft
//!   installation is present.
//!
//! - **Real** (`TileSet::build_real`) — 16×16 PNG tiles extracted from the
//!   Minecraft Java Edition client JAR via [`JarTextureLoader`]. Falls back
//!   to a procedural tile for any block not found in the JAR.
//!
//! Both produce the same `TileSet` type, so downstream code (renderer, OBJ
//! exporter) is unchanged.
//!
//! A **face-aware** variant (`FaceAwareTileSet`) stores a separate UV rect per
//! `(block_name, face_direction)`, enabling correct top/side/bottom textures
//! for blocks like grass, logs, and crafting tables.

use crate::block_model::{face_textures, FaceTextures};
use crate::blocks::block_color;
use crate::chunk::strip_namespace;
use crate::jar_textures::JarTextureLoader;
use std::collections::HashMap;

/// Tile edge in pixels (Minecraft's native texture resolution).
pub const TILE: usize = 16;
/// Tiles per atlas row.
pub const TILES_PER_ROW: usize = 16;

// ─────────────────────────────────────────────────────────────────────────────
// TileSet
// ─────────────────────────────────────────────────────────────────────────────

/// An atlas covering a set of block names: one tile each, row-major.
///
/// All six faces of a block share the same tile. Use [`FaceAwareTileSet`] when
/// per-face textures are needed (e.g. grass top ≠ grass side).
pub struct TileSet {
    pub(crate) names: Vec<String>,
    pub(crate) ids: HashMap<String, u16>,
    /// Atlas width in pixels.
    pub width: u32,
    /// Atlas height in pixels.
    pub height: u32,
    /// RGBA pixels, row-major.
    pub pixels: Vec<u8>,
}

impl TileSet {
    /// Create a `TileSet` from raw pixel data (used by Mineways atlas).
    /// The `names` and `ids` maps are empty since the Mineways atlas is a
    /// pre-built image with fixed tile positions, not a packed set of tiles.
    pub fn from_raw(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        Self {
            names: Vec::new(),
            ids: HashMap::new(),
            pixels,
            width,
            height,
        }
    }

    /// Build a **procedural** atlas (no real textures, deterministic noise).
    ///
    /// This is the fallback when no Minecraft JAR is available.
    pub fn build(names: impl IntoIterator<Item = String>) -> Self {
        let unique = collect_unique(names);
        let (width, height, mut pixels) = alloc_atlas(unique.len());
        let mut ids = HashMap::with_capacity(unique.len());
        for (index, name) in unique.iter().enumerate() {
            ids.insert(name.clone(), index as u16);
            let tile = procedural_tile(name);
            blit_tile(&mut pixels, width as usize, index, &tile);
        }
        Self {
            names: unique,
            ids,
            width,
            height,
            pixels,
        }
    }

    /// Build a **real-texture** atlas from a [`JarTextureLoader`].
    ///
    /// Each block name is resolved to a single texture name (via
    /// [`face_textures`]'s top face, for simplicity). For a full per-face
    /// atlas, use [`FaceAwareTileSet::build`].
    ///
    /// Textures not found in the JAR fall back to procedural tiles.
    pub fn build_real(names: impl IntoIterator<Item = String>, loader: &JarTextureLoader) -> Self {
        let unique = collect_unique(names);
        let (width, height, mut pixels) = alloc_atlas(unique.len());
        let mut ids = HashMap::with_capacity(unique.len());
        for (index, name) in unique.iter().enumerate() {
            ids.insert(name.clone(), index as u16);
            let short = strip_namespace(name);
            let ft = face_textures(name);
            // Use the top-face texture as the representative tile.
            let tile = load_tile(loader, &ft.top)
                .or_else(|| load_tile(loader, short))
                .unwrap_or_else(|| procedural_tile(name));
            blit_tile(&mut pixels, width as usize, index, &tile);
        }
        Self {
            names: unique,
            ids,
            width,
            height,
            pixels,
        }
    }

    /// The block names covered by this atlas (tile order).
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// UV rectangle `(u0, v0, u1, v1)` of a block's tile, or the fallback
    /// tile (index 0) for unknown names.
    pub fn uv_rect(&self, name: &str) -> [f32; 4] {
        let index = self.ids.get(name).copied().unwrap_or(0) as usize;
        uv_rect_for_index(index, self.width, self.height)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// FaceAwareTileSet
// ─────────────────────────────────────────────────────────────────────────────

/// Face direction index, matching `FACES` order in `mesh.rs` and `obj.rs`:
/// `0`=+Y (top), `1`=-Y (bottom), `2`=+X, `3`=-X, `4`=+Z, `5`=-Z.
pub type FaceDir = usize;

/// An atlas that stores a **separate tile per unique texture name**, with UV
/// lookups available per `(block_name, face_direction)`.
///
/// This enables correct per-face texturing: grass top is green, sides show the
/// transition strip, bottom is plain dirt.
pub struct FaceAwareTileSet {
    /// The underlying atlas (one tile per unique texture name).
    pub atlas: TileSet,
    /// Per-block, per-face UV rect. Key: `(namespaced_block_name, face_dir)`.
    pub(crate) face_uvs: HashMap<(String, FaceDir), [f32; 4]>,
    /// UV rect per *texture name* (`"bush"`, `"oak_log_top"`).
    ///
    /// Vanilla block models name a texture per face rather than relying on a
    /// block's face order, so model-driven geometry needs this direct lookup.
    pub(crate) texture_uvs: HashMap<String, [f32; 4]>,
}

impl FaceAwareTileSet {
    /// Build a face-aware atlas from a [`JarTextureLoader`].
    ///
    /// Collects all texture names referenced across all block faces, packs them
    /// into a single atlas, then stores per-face UV lookups.
    pub fn build(block_names: impl IntoIterator<Item = String>, loader: &JarTextureLoader) -> Self {
        let block_names = collect_unique(block_names);

        let mut texture_set: Vec<String> = Vec::new();
        let mut add = |tex: &str| {
            if !texture_set.iter().any(|t| t == tex) {
                texture_set.push(tex.to_owned());
            }
        };

        let resolver = crate::block_model::get_resolver();

        let face_map: Vec<(String, FaceTextures)> = block_names
            .iter()
            .map(|name| {
                let short = strip_namespace(name);
                if let Some(res) = resolver {
                    for tex in res.all_textures(short) {
                        add(&tex);
                    }
                }

                let ft = face_textures(name);
                add(&ft.top);
                add(&ft.bottom);
                add(&ft.south);
                add(&ft.north);
                add(&ft.east);
                add(&ft.west);
                (name.clone(), ft)
            })
            .collect();

        // Build the atlas over texture names (not block names).
        let (width, height, mut pixels) = alloc_atlas(texture_set.len());
        let mut tex_ids: HashMap<String, u16> = HashMap::new();
        for (index, tex_name) in texture_set.iter().enumerate() {
            tex_ids.insert(tex_name.clone(), index as u16);
            // Try real texture, then procedural.
            let tile =
                load_tile(loader, tex_name).unwrap_or_else(|| procedural_tile_from_name(tex_name));
            blit_tile(&mut pixels, width as usize, index, &tile);
        }

        // Build a TileSet-like wrapper using texture names as keys.
        let atlas = TileSet {
            names: texture_set.clone(),
            ids: tex_ids.clone(),
            width,
            height,
            pixels,
        };

        // Build per-face UV map.
        let mut face_uvs: HashMap<(String, FaceDir), [f32; 4]> = HashMap::new();
        let tex_uv = |tex: &str| -> [f32; 4] {
            let idx = tex_ids.get(tex).copied().unwrap_or(0) as usize;
            uv_rect_for_index(idx, width, height)
        };
        for (block_name, ft) in &face_map {
            face_uvs.insert((block_name.clone(), 0), tex_uv(&ft.top));
            face_uvs.insert((block_name.clone(), 1), tex_uv(&ft.bottom));
            face_uvs.insert((block_name.clone(), 2), tex_uv(&ft.east));
            face_uvs.insert((block_name.clone(), 3), tex_uv(&ft.west));
            face_uvs.insert((block_name.clone(), 4), tex_uv(&ft.south));
            face_uvs.insert((block_name.clone(), 5), tex_uv(&ft.north));
        }

        let texture_uvs = tex_ids
            .keys()
            .map(|name| (name.clone(), tex_uv(name)))
            .collect();
        Self { atlas, face_uvs, texture_uvs }
    }

    /// UV rect for a specific block face.
    ///
    /// Falls back to `(0,0,1,1)` (the whole atlas stretched across the
    /// face) for unknown blocks or face directions. This should never
    /// happen for a `block` key that came from the same
    /// `Chunk::texture_keys()` call used to build this tileset — if it
    /// does, something upstream (name resolution, key construction) is
    /// broken, so it's logged rather than failing silently.
    pub fn face_uv(&self, block: &str, face: FaceDir) -> [f32; 4] {
        match self.face_uvs.get(&(block.to_owned(), face)) {
            Some(uv) => *uv,
            None => {
                tracing::warn!(
                    "no UV entry for block={block:?} face={face}; falling back to full-atlas UV \
                     (texture will look wrong for this face)"
                );
                [0.0, 0.0, 1.0, 1.0]
            }
        }
    }

    /// UV rect for a specific texture name.
    pub fn tile_uv(&self, texture: &str) -> [f32; 4] {
        if let Some(uv) = self.texture_uvs.get(texture) {
            return *uv;
        }
        let idx = self.atlas.ids.get(texture).copied().unwrap_or(0) as usize;
        uv_rect_for_index(idx, self.atlas.width, self.atlas.height)
    }

    /// True when `texture` has a real tile in this atlas.
    pub fn has_texture(&self, texture: &str) -> bool {
        self.texture_uvs.contains_key(texture) || self.atlas.ids.contains_key(texture)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Collect and deduplicate block names.
fn collect_unique(names: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut unique: Vec<String> = names.into_iter().collect();
    unique.sort();
    unique.dedup();
    if unique.is_empty() {
        unique.push("minecraft:air".to_owned());
    }
    unique
}

/// Allocate atlas pixel buffer. Returns `(width, height, pixels)`.
fn alloc_atlas(tile_count: usize) -> (u32, u32, Vec<u8>) {
    let tiles = tile_count.max(1);
    let rows = tiles.div_ceil(TILES_PER_ROW);
    let width = (TILES_PER_ROW * TILE) as u32;
    let height = (rows * TILE) as u32;
    let pixels = vec![0u8; (width * height) as usize * 4];
    (width, height, pixels)
}

/// Copy a 16×16 RGBA tile into the atlas at `tile_index`.
fn blit_tile(pixels: &mut [u8], atlas_width: usize, tile_index: usize, tile: &[u8]) {
    let col = tile_index % TILES_PER_ROW;
    let row = tile_index / TILES_PER_ROW;
    for y in 0..TILE {
        let dst = ((row * TILE + y) * atlas_width + col * TILE) * 4;
        pixels[dst..dst + TILE * 4].copy_from_slice(&tile[y * TILE * 4..(y + 1) * TILE * 4]);
    }
}

/// UV rectangle for a tile at `index` inside an atlas of given dimensions.
///
/// The rect is inset by half a texel on every side so that linear filtering
/// (e.g. Blender's default interpolation) never bleeds the neighbouring tile.
fn uv_rect_for_index(index: usize, width: u32, height: u32) -> [f32; 4] {
    let col = index % TILES_PER_ROW;
    let row = index / TILES_PER_ROW;
    let (w, h) = (width as f32, height as f32);
    let (du, dv) = (0.5 / w, 0.5 / h);
    [
        (col * TILE) as f32 / w + du,
        (row * TILE) as f32 / h + dv,
        ((col + 1) * TILE) as f32 / w - du,
        ((row + 1) * TILE) as f32 / h - dv,
    ]
}

/// Try to load a 16×16 RGBA tile from the JAR loader by texture name.
///
/// Applies biome tinting for textures the game stores in grayscale (grass,
/// leaves, water) and composites the grass-block side overlay, so the atlas
/// looks like the game instead of raw gray PNGs.
///
/// Returns `None` if the texture is not in the loader or cannot be decoded.
fn load_tile(loader: &JarTextureLoader, tex_name: &str) -> Option<[u8; TILE * TILE * 4]> {
    let mut tile = decode_tile(loader.get(tex_name)?)?;

    // The grass block side is a dirt texture with a gray strip on top; the
    // game alpha-composites a tinted overlay over that strip.
    if tex_name == "grass_block_side" {
        let overlay = loader.get("grass_block_side_overlay").and_then(decode_tile);
        if let Some(mut overlay) = overlay {
            apply_tint(&mut overlay, PLAINS_GRASS);
            for (px, ov) in tile.chunks_exact_mut(4).zip(overlay.chunks_exact(4)) {
                if ov[3] > 0 {
                    px[0] = ov[0];
                    px[1] = ov[1];
                    px[2] = ov[2];
                }
            }
        }
    }
    if let Some(tint) = tint_for(tex_name) {
        apply_tint(&mut tile, tint);
    }
    Some(tile)
}

/// Decode PNG bytes into a 16×16 RGBA tile (nearest-neighbour resampled).
fn decode_tile(png_bytes: &[u8]) -> Option<[u8; TILE * TILE * 4]> {
    let img = image::load_from_memory(png_bytes).ok()?;
    let img = img.resize_exact(
        TILE as u32,
        TILE as u32,
        image::imageops::FilterType::Nearest,
    );
    let raw = img.to_rgba8().into_raw();
    if raw.len() != TILE * TILE * 4 {
        return None;
    }
    let mut out = [0u8; TILE * TILE * 4];
    out.copy_from_slice(&raw);
    Some(out)
}

/// Plains-biome grass green.
/// Source: Minecraft grass colormap at temperature=0.8, downfall=0.4 (plains).
/// Confirmed against MCprep data (linear [0.227, 0.615, 0.089] → sRGB #83CE54
/// rounds to the canonical wiki value #79C05A).
pub(crate) const PLAINS_GRASS: [u8; 3] = [0x79, 0xC0, 0x5A];
/// Plains-biome foliage green.
/// Source: Minecraft foliage colormap at temperature=0.8, downfall=0.4.
pub(crate) const PLAINS_FOLIAGE: [u8; 3] = [0x59, 0xAE, 0x30];
/// Standard blue for water in plains/temperate biomes (#3F76E4).
const PLAINS_WATER: [u8; 3] = [0x3F, 0x76, 0xE4];

/// Texture names that should receive a tint multiplied into their pixels
/// at atlas-build time, because the JAR ships them as grayscale and
/// expects the game engine to colour them from the biome colormap.
///
/// All entries cross-referenced against MCprep `desaturated` list and the
/// Minecraft wiki grass/foliage colormaps.
///
/// Redstone dust textures also appear in the MCprep desaturated list; they
/// are omitted here because the game stores them pre-tinted in modern JAR.
///
/// Self-luminous block textures (emit list, for future shader use):
/// glowstone, sea_lantern, lava_still, lava_flow, fire_layer_0/1,
/// magma, shroomlight, beacon, campfire_log_lit, soul_campfire_log_lit,
/// torch, soul_torch, lantern, soul_lantern, redstone_torch, end_rod,
/// sea_pickle, jack_o_lantern, blast_furnace_front_on, furnace_front_on,
/// glow_lichen, nether_portal.
fn tint_for(tex_name: &str) -> Option<[u8; 3]> {
    match tex_name {
        // ── Grass / ground cover ──────────────────────────────────────────
        "grass_block_top" | "grass_block_overlay" | "grass_block_side_overlay"
        | "short_grass" | "grass"
        | "tall_grass" | "tall_grass_top" | "tall_grass_bottom"
        | "fern" | "large_fern" | "large_fern_top" | "large_fern_bottom"
        // `sugar_cane` ships already coloured and must not be tinted; `vine`
        // takes the foliage colour, not the grass one, and is listed below.
        // Crop stems are grayscale and biome-tinted by the game
        | "melon_stem" | "pumpkin_stem"
        | "attached_melon_stem" | "attached_pumpkin_stem" => Some(PLAINS_GRASS),

        // ── Leaves and ground foliage ────────────────────────────────────
        // `bush` and `leaf_litter` (1.21.5) ship as grayscale like the
        // leaves do. They were listed for the atlas but not here, so blocks
        // taking the prototype path came out colourless while the very same
        // block drawn from the atlas looked right.
        // `jungle_leaves` is absent on purpose: it ships already coloured, so
        // tinting it double-darkens the canopy.
        "oak_leaves" | "acacia_leaves" | "dark_oak_leaves"
        | "mangrove_leaves" | "birch_leaves" | "pale_oak_leaves"
        | "vine" | "bush" | "leaf_litter" => Some(PLAINS_FOLIAGE),
        "spruce_leaves" => Some([0x61, 0x99, 0x61]),
        // Cherry leaves ship fully coloured in the JAR — no tint needed.

        // ── Water plants ─────────────────────────────────────────────────
        // Lily pad is fully green in-game; keep its own tint.
        "lily_pad" | "lilypad" => Some([0x20, 0x80, 0x30]),

        // ── Water ────────────────────────────────────────────────────────
        // `water` itself is listed because the fluid has no model JSON to name
        // a texture, so consumers ask for the block id and resolve by prefix —
        // and `water_overlay` is what that fallback often lands on. All three
        // are the same grayscale mask and all three need the biome colour;
        // missing them renders the ocean as a flat grey slab.
        "water" | "water_still" | "water_flow" | "water_overlay" => Some(PLAINS_WATER),

        _ => None,
    }
}

/// Biome tint for a texture the JAR ships as grayscale, if it needs one.
///
/// Exposed so consumers that write out raw JAR PNGs — the prototype exporter,
/// which emits one texture file per block rather than an atlas — apply the
/// same colour the atlas builder bakes in. Skipping it is not a subtle
/// difference: grass tops and leaves are stored as near-white grayscale and
/// render as white or grey patches without it.
pub fn biome_tint(tex_name: &str) -> Option<[u8; 3]> {
    tint_for(tex_name)
}

/// Multiply every pixel of a tile by an RGB tint.
fn apply_tint(tile: &mut [u8; TILE * TILE * 4], tint: [u8; 3]) {
    for px in tile.chunks_exact_mut(4) {
        px[0] = (u16::from(px[0]) * u16::from(tint[0]) / 255) as u8;
        px[1] = (u16::from(px[1]) * u16::from(tint[1]) / 255) as u8;
        px[2] = (u16::from(px[2]) * u16::from(tint[2]) / 255) as u8;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Procedural tile generation (fallback)
// ─────────────────────────────────────────────────────────────────────────────

/// Generate a deterministic procedural 16×16 RGBA tile from a *block* name.
fn procedural_tile(block_name: &str) -> [u8; TILE * TILE * 4] {
    let short = strip_namespace(block_name);
    procedural_tile_from_name(short)
}

/// Generate a deterministic procedural 16×16 RGBA tile from a *texture* name.
/// Used as fallback when the JAR does not have the texture.
fn procedural_tile_from_name(name: &str) -> [u8; TILE * TILE * 4] {
    // Derive a colour from the name via block_color or direct hash.
    let [r, g, b] = if let Some(c) = known_color_by_texture(name) {
        c
    } else {
        block_color(name)
    };
    let mut rng = Rng::from_str(name);
    let mut tile = [0u8; TILE * TILE * 4];
    for y in 0..TILE {
        for x in 0..TILE {
            let shade = pattern_shade(name, x, y, &mut rng);
            let offset = (y * TILE + x) * 4;
            tile[offset] = (r * shade * 255.0) as u8;
            tile[offset + 1] = (g * shade * 255.0) as u8;
            tile[offset + 2] = (b * shade * 255.0) as u8;
            tile[offset + 3] = 255;
        }
    }
    tile
}

/// Map some well-known texture names to better colours for the procedural
/// fallback (so grass_block_top looks greenish even without a JAR).
fn known_color_by_texture(name: &str) -> Option<[f32; 3]> {
    let rgb = |r: u8, g: u8, b: u8| {
        Some([
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        ])
    };
    match name {
        "grass_block_top" => rgb(100, 168, 60),
        "grass_block_side" => rgb(112, 140, 80),
        "grass_block_side_overlay" => rgb(80, 160, 50),
        "dirt" => rgb(134, 96, 67),
        "oak_log_top" => rgb(130, 108, 70),
        "oak_log" => rgb(106, 85, 52),
        "water_still" => rgb(52, 95, 218),
        "lava_still" => rgb(212, 90, 18),
        "sand" => rgb(219, 207, 163),
        "gravel" => rgb(131, 127, 126),
        "stone" => rgb(125, 125, 125),
        "bedrock" => rgb(56, 56, 56),
        _ => None,
    }
}

/// Per-pixel brightness multiplier, mimicking the look of common textures.
fn pattern_shade(name: &str, x: usize, y: usize, rng: &mut Rng) -> f32 {
    let noise = 0.9 + rng.next_f32() * 0.2;
    if name.contains("planks") {
        return if y % 4 == 3 { 0.55 } else { noise };
    }
    if name.contains("bricks") {
        let offset = if (y / 4).is_multiple_of(2) { 0 } else { 4 };
        return if y % 4 == 3 || (x + offset) % 8 == 7 {
            0.6
        } else {
            noise
        };
    }
    if name.ends_with("_log") || name.ends_with("_log_top") || name.ends_with("_stem") {
        return if x.is_multiple_of(4) { 0.65 } else { noise };
    }
    if name.contains("leaves") {
        return 0.7 + rng.next_f32() * 0.5;
    }
    if name == "glass" || name == "tinted_glass" {
        let edge = x == 0 || y == 0 || x == TILE - 1 || y == TILE - 1;
        return if edge { 1.0 } else { 0.35 };
    }
    if name.contains("_ore") {
        return if rng.next_f32() < 0.12 {
            1.5
        } else {
            noise * 0.9
        };
    }
    if name == "water_still" || name == "lava_still" {
        return if (y + x / 4).is_multiple_of(5) {
            1.15
        } else {
            noise * 0.95
        };
    }
    noise
}

/// Tiny deterministic PRNG so every block's tile is stable across runs.
struct Rng(u64);

impl Rng {
    fn from_str(seed: &str) -> Self {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in seed.bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        Self(hash | 1)
    }

    fn next_f32(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        let value = x.wrapping_mul(0x2545_f491_4f6c_dd1d);
        (value >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_tiles_are_deterministic() {
        let names = || ["minecraft:stone".to_owned(), "minecraft:dirt".to_owned()];
        let a = TileSet::build(names());
        let b = TileSet::build(names());
        assert_eq!(a.pixels, b.pixels);
        assert_eq!(a.uv_rect("minecraft:stone"), b.uv_rect("minecraft:stone"));
        assert_eq!(a.width, 256);
        assert_eq!(a.height, 16);
    }

    #[test]
    fn uv_rects_stay_inside_the_atlas() {
        let set = TileSet::build((0..40).map(|i| format!("minecraft:block_{i}")));
        assert_eq!(set.height, 16 * 3);
        for i in 0..40 {
            let name = format!("minecraft:block_{i}");
            let [u0, v0, u1, v1] = set.uv_rect(&name);
            assert!(u0 >= 0.0 && u1 <= 1.0 && v0 >= 0.0 && v1 <= 1.0);
            assert!(u1 > u0 && v1 > v0);
        }
    }

    #[test]
    fn unknown_block_maps_to_tile_zero() {
        let set = TileSet::build(["minecraft:stone".to_owned()]);
        let uv = set.uv_rect("minecraft:nonexistent_block");
        assert_eq!(uv, set.uv_rect("minecraft:stone")); // both → index 0
    }
}
