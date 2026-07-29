//! Mineways-compatible block texture mapping.
//!
//! Uses Mineways' `terrainExt.png` texture atlas and `gTilesTable[]`
//! tile-position data to assign correct per-face textures to any block.
//! This replaces the homegrown `block_model.rs` + `jar_textures.rs` pipeline
//! with Mineways' battle-tested 15-year block-definition database.

use crate::block_definitions::G_BLOCK_DEFINITIONS;
use crate::chunk::strip_namespace;
use crate::mineways_data::{swatch_by_filename, swatch_uv, TILE_TABLE};

/// Common block name transformations: maps non-standard Bedrock names
/// to the vanilla Java texture filenames used in Mineways' tile table.
fn normalize_block_name(name: &str) -> &str {
    // Bedrock sometimes uses different naming conventions.
    // These are the most common mappings.
    match name {
        "grass_block" => "grass_block_top",
        "grass" => "short_grass",
        "stonebrick" => "stone_bricks",
        "stonebricks" => "stone_bricks",
        "planks" => "oak_planks",
        "log" => "oak_log",
        "log2" => "acacia_log",
        "leaves" => "oak_leaves",
        "leaves2" => "acacia_leaves",
        "wool" => "white_wool",
        "red_flower" => "poppy",
        "yellow_flower" => "dandelion",
        "double_plant" => "sunflower_bottom",
        "lit_pumpkin" => "jack_o_lantern",
        "lit_furnace" => "furnace_front_on",
        "unlit_redstone_torch" => "redstone_torch_off",
        "stone_slab" => "stone_slab_top",
        "wooden_slab" => "oak_planks",
        "melon_block" => "melon_side",
        "nether_brick" => "nether_bricks",
        "end_bricks" => "end_stone_bricks",
        "end_brick_stairs" => "end_stone_bricks",
        "hardened_clay" => "terracotta",
        "stained_hardened_clay" => "white_terracotta",
        "concretepowder" => "white_concrete_powder",
        "glazedterracotta" => "white_glazed_terracotta",
        "slime" => "slime_block",
        "prismarine_rough" => "prismarine",
        "stone_andesite" => "andesite",
        "stone_diorite" => "diorite",
        "stone_granite" => "granite",
        "monster_egg" => "stone",
        "mob_spawner" => "spawner",
        "quartz_ore" => "nether_quartz_ore",
        "lit_redstone_ore" => "redstone_ore",
        "pumpkin" => "carved_pumpkin",
        "beacon" => "beacon",
        "reeds" => "sugar_cane",
        "snow_layer" => "snow",
        "snowball" => "snow",
        "fire" => "fire_0",
        "portal" => "nether_portal",
        "stationary_water" => "water_still",
        "stationary_lava" => "lava_still",
        "web" => "cobweb",
        "deadbush" => "dead_bush",
        "waterlily" => "lily_pad",
        "mushroom" => "brown_mushroom",
        _ => name,
    }
}

/// Return the primary texture filename for a given block name.
/// Most blocks have the same name as their primary texture.
fn block_texture_name(name: &str) -> &str {
    let norm = normalize_block_name(name);
    // Direct lookup: is there a swatch with exactly this name?
    if swatch_by_filename(norm).is_some() {
        return norm;
    }
    // Try common suffixes for blocks whose texture name matches
    // the block name pattern:
    //   "stone" -> "stone"
    //   "dirt" -> "dirt"
    //   "oak_planks" -> "oak_planks"
    // If nothing found, return the name as-is
    norm
}

/// Splits a [`crate::chunk::BlockState::texture_key`] back into
/// `(block_name, color)`. Keys without a colour suffix (most blocks) yield
/// `color: None`.
pub fn split_texture_key(key: &str) -> (&str, Option<&str>) {
    match key.split_once('|') {
        Some((name, color)) => (name, Some(color)),
        None => (key, None),
    }
}

/// Legacy (pre-flattening) block IDs that pack their dye colour into a
/// separate `color` state instead of the block name, e.g. `wool` +
/// `color=red` rather than `red_wool`. Returns the texture-name suffix to
/// splice the colour into (`"red" + "_wool" = "red_wool"`), or `None` if
/// `short` isn't one of these.
///
/// Every one of these previously fell straight through to
/// [`normalize_block_name`], which hardcodes the *white* variant — so any
/// non-white wool, carpet, concrete powder, stained glass (pane), or
/// glazed/stained terracotta rendered as its pale "white_*" swatch
/// regardless of actual colour. `carpet` reuses the wool texture, matching
/// Mineways' own swatch table (there's no separate `<color>_carpet` tile).
fn legacy_color_suffix(short: &str) -> Option<&'static str> {
    match short {
        "wool" | "carpet" => Some("_wool"),
        "concretepowder" => Some("_concrete_powder"),
        "stained_hardened_clay" => Some("_terracotta"),
        "glazedterracotta" => Some("_glazed_terracotta"),
        // Panes have no separate `<color>_stained_glass_pane` swatch in the
        // atlas — they reuse the plain glass texture, same as vanilla.
        "stained_glass" | "stained_glass_pane" => Some("_stained_glass"),
        _ => None,
    }
}

/// Fallback: look up a block in the authoritative [`G_BLOCK_DEFINITIONS`]
/// to derive the swatch index (`txr_y * 16 + txr_x`).
fn swatch_from_definitions(short: &str) -> Option<usize> {
    let mw = short
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    let (txr_x, txr_y) = G_BLOCK_DEFINITIONS
        .iter()
        .find(|bi| bi.name == mw)
        .map(|bi| (bi.txr_x, bi.txr_y))?;
    Some((txr_y * 16 + txr_x) as usize)
}

/// Try to find the swatch index for a texture name, with fallback logic.
fn find_swatch(name: &str) -> Option<usize> {
    // Try exact match first
    if let Some(idx) = swatch_by_filename(name) {
        return Some(idx);
    }
    // Try with common block prefixes stripped
    let stripped = name
        .strip_prefix("block_")
        .or_else(|| name.strip_prefix("tile_"))
        .unwrap_or(name);
    if let Some(idx) = swatch_by_filename(stripped) {
        return Some(idx);
    }
    None
}

/// Last-resort swatch resolution for blocks whose own name is not a texture
/// filename in the atlas.
///
/// The Mineways tile table is keyed by *texture* names, not block ids, and the
/// two diverge for whole families: a two-tall plant ships only as
/// `<name>_top`/`<name>_bottom`, and slabs/stairs/walls/fences/carpets have no
/// texture of their own at all — they reuse their base block's. Without these
/// rules each such block fails to resolve and the tileset builder falls it
/// through to `[0; 6]`, rendering it as `grass_block_top`. Handling them by
/// family rather than one arm per block keeps new variants working for free.
fn fallback_swatch(short: &str) -> Option<usize> {
    // Two-tall / multi-part blocks: use the upper half's texture, or the
    // side texture for blocks that only ship `_side`/`_top` pairs.
    if let Some(idx) = find_swatch(&format!("{short}_top")) {
        return Some(idx);
    }
    if let Some(idx) = find_swatch(&format!("{short}_side")) {
        return Some(idx);
    }

    // Cut-down variants reuse the full block's texture. `stone_brick_slab`
    // needs the pluralised base (`stone_bricks`) and `birch_slab` the wood's
    // plank texture, hence the extra attempts.
    for suffix in ["_slab", "_stairs", "_wall"] {
        if let Some(base) = short.strip_suffix(suffix) {
            if let Some(idx) = find_swatch(base) {
                return Some(idx);
            }
            if let Some(idx) = find_swatch(&format!("{base}s")) {
                return Some(idx);
            }
            if let Some(idx) = find_swatch(&format!("{base}_planks")) {
                return Some(idx);
            }
            if let Some(idx) = find_swatch(&format!("{base}_top")) {
                return Some(idx);
            }
        }
    }

    // Plant stems are drawn from the parent plant's texture.
    if let Some(base) = short.strip_suffix("_stem") {
        if let Some(idx) = find_swatch(&format!("{base}_side")) {
            return Some(idx);
        }
        if let Some(idx) = find_swatch(&format!("{base}_top")) {
            return Some(idx);
        }
    }

    // Wooden fences/gates/doors are drawn from their plank texture.
    for suffix in ["_fence_gate", "_fence", "_trapdoor", "_door"] {
        if let Some(base) = short.strip_suffix(suffix) {
            if let Some(idx) = find_swatch(&format!("{base}_planks")) {
                return Some(idx);
            }
        }
    }

    // Waxing only stops oxidation; the texture is the unwaxed block's.
    if let Some(base) = short.strip_prefix("waxed_") {
        if let Some(idx) = find_swatch(base) {
            return Some(idx);
        }
        if let Some(idx) = fallback_swatch(base) {
            return Some(idx);
        }
    }

    // Carpet takes the matching wool colour; non-dyed carpets (moss) fall
    // back to their source block.
    if let Some(base) = short.strip_suffix("_carpet") {
        if let Some(idx) = find_swatch(&format!("{base}_wool")) {
            return Some(idx);
        }
        if let Some(idx) = find_swatch(&format!("{base}_block")) {
            return Some(idx);
        }
        if let Some(idx) = find_swatch(base) {
            return Some(idx);
        }
    }

    // Beds share one texture set across all dye colours.
    if short.ends_with("_bed") {
        if let Some(idx) = find_swatch("MW_bed_head_top") {
            return Some(idx);
        }
    }

    // A potted plant is textured as the plant it holds.
    if let Some(plant) = short.strip_prefix("potted_") {
        if let Some(idx) = find_swatch(plant) {
            return Some(idx);
        }
    }

    // Wall-mounted variants reuse the standing block's texture.
    if let Some(base) = short.strip_suffix("_wall_torch") {
        if let Some(idx) = find_swatch(&format!("{base}_torch")) {
            return Some(idx);
        }
    }
    if short == "wall_torch" {
        if let Some(idx) = find_swatch("torch") {
            return Some(idx);
        }
    }

    // Infested variants are visually identical to the host block.
    if let Some(base) = short.strip_prefix("infested_") {
        if let Some(idx) = find_swatch(base) {
            return Some(idx);
        }
        if let Some(idx) = find_swatch(&format!("{base}_top")) {
            return Some(idx);
        }
    }

    None
}

/// Remaps a physical cube face index (`[top, bottom, east, west, south,
/// north]`, matching [`block_shape::CUBE_FACES`](crate::block_shape::CUBE_FACES)
/// order) onto the logical face slot that [`get_swatch_for_block`]'s
/// per-face tables assume — i.e. as if the block's pillar `axis` were `y`.
///
/// [`get_swatch_for_block`] hardcodes end-grain on faces 0/1 (top/bottom)
/// and bark on faces 2..6, which is only correct for vertically-placed
/// pillars (logs, hay/bone blocks, quartz/purpur pillars, chains, ...).
/// A log placed on its side (`axis=x` or `axis=z`) needs the end-grain
/// texture on the faces perpendicular to its actual axis instead. Blocks
/// without an `axis` property (or with `axis="y"`, the default) pass
/// through unchanged.
pub fn remap_pillar_face(face_idx: usize, axis: &str) -> usize {
    match axis {
        "x" => match face_idx {
            2 => 0, // east: end-grain slot
            3 => 1, // west: end-grain slot
            _ => 2, // top/bottom/south/north: any bark slot
        },
        "z" => match face_idx {
            4 => 0, // south: end-grain slot
            5 => 1, // north: end-grain slot
            _ => 2, // top/bottom/east/west: any bark slot
        },
        _ => face_idx, // "y" (default) or unrecognised: no remap
    }
}

/// Get the swatch indices for all 6 faces of a block.
///
/// Returns `[top, bottom, east, west, south, north]` swatch indices.
/// Most blocks use the same texture on all faces.
/// Special blocks (grass, logs, etc.) have per-face textures.
pub fn get_swatch_for_block(name: &str, color: Option<&str>) -> Option<[usize; 6]> {
    let short = strip_namespace(name);

    // Legacy colour-variant blocks (wool, carpet, concrete powder, stained
    // glass (pane), stained/glazed terracotta) carry their actual colour in
    // a separate `color` state rather than the block name. Resolve that
    // first — e.g. `wool` + `red` -> `red_wool` — so the block renders in
    // its real colour instead of always falling back to the white variant.
    if let (Some(color), Some(suffix)) = (color, legacy_color_suffix(short)) {
        let colored_name = format!("{color}{suffix}");
        if let Some(idx) = find_swatch(&colored_name) {
            return Some([idx; 6]);
        }
        // Unrecognised colour string: fall through to the name-only lookup
        // below, which resolves to the white/default swatch rather than
        // failing the whole block.
    }

    let tex_name = block_texture_name(short);

    // Deliberately NOT `?` here. Many blocks below have a correct explicit
    // per-face mapping even though no swatch shares their base name — the
    // atlas ships `podzol_top`/`podzol_side` but no bare `podzol`, and
    // likewise for `tall_grass`, `large_fern`, beds and others. Bailing on
    // the name-only lookup before the match ran would throw those mappings
    // away and drop the block into the caller's `[0; 6]` fallback, i.e.
    // render it as `grass_block_top`.
    //
    // `UNRESOLVED` stands in for "no name-matching swatch" so the per-face
    // arms can still be consulted; any arm that genuinely needed the default
    // is caught after the match and reported as a real failure.
    const UNRESOLVED: usize = usize::MAX;
    let default_swatch = find_swatch(tex_name)
        .or_else(|| swatch_from_definitions(short))
        .or_else(|| fallback_swatch(short))
        .unwrap_or(UNRESOLVED);

    // Per-face overrides for blocks with different textures per side
    let faces = match short {
        // Grass: top=0, side=3, bottom=2
        "grass_block" => [
            find_swatch("grass_block_top").unwrap_or(0),
            find_swatch("dirt").unwrap_or(2),
            find_swatch("grass_block_side").unwrap_or(3),
            find_swatch("grass_block_side").unwrap_or(3),
            find_swatch("grass_block_side").unwrap_or(3),
            find_swatch("grass_block_side").unwrap_or(3),
        ],

        // Grass snow: top=0, side=4,row=4 (snow side), bottom=2
        "grass_block_snow" => [
            find_swatch("grass_block_top").unwrap_or(0),
            find_swatch("dirt").unwrap_or(2),
            find_swatch("grass_block_snow").unwrap_or(68),
            find_swatch("grass_block_snow").unwrap_or(68),
            find_swatch("grass_block_snow").unwrap_or(68),
            find_swatch("grass_block_snow").unwrap_or(68),
        ],

        // Podzol: top=15,17 (podzol_top), side=14,17 (podzol_side), bottom=2
        "podzol" => [
            find_swatch("podzol_top").unwrap_or(287),
            find_swatch("dirt").unwrap_or(2),
            find_swatch("podzol_side").unwrap_or(286),
            find_swatch("podzol_side").unwrap_or(286),
            find_swatch("podzol_side").unwrap_or(286),
            find_swatch("podzol_side").unwrap_or(286),
        ],

        // Mycelium: top=14,4, side=13,4, bottom=2
        "mycelium" => [
            find_swatch("mycelium_top").unwrap_or(78),
            find_swatch("dirt").unwrap_or(2),
            find_swatch("mycelium_side").unwrap_or(77),
            find_swatch("mycelium_side").unwrap_or(77),
            find_swatch("mycelium_side").unwrap_or(77),
            find_swatch("mycelium_side").unwrap_or(77),
        ],

        // Logs: side=log, top/bottom=log_top
        "oak_log" => [
            find_swatch("oak_log_top").unwrap_or(21),
            find_swatch("oak_log_top").unwrap_or(21),
            find_swatch("oak_log").unwrap_or(20),
            find_swatch("oak_log").unwrap_or(20),
            find_swatch("oak_log").unwrap_or(20),
            find_swatch("oak_log").unwrap_or(20),
        ],
        "spruce_log" => [
            find_swatch("spruce_log_top").unwrap_or(188),
            find_swatch("spruce_log_top").unwrap_or(188),
            find_swatch("spruce_log").unwrap_or(116),
            find_swatch("spruce_log").unwrap_or(116),
            find_swatch("spruce_log").unwrap_or(116),
            find_swatch("spruce_log").unwrap_or(116),
        ],
        "birch_log" => [
            find_swatch("birch_log_top").unwrap_or(187),
            find_swatch("birch_log_top").unwrap_or(187),
            find_swatch("birch_log").unwrap_or(117),
            find_swatch("birch_log").unwrap_or(117),
            find_swatch("birch_log").unwrap_or(117),
            find_swatch("birch_log").unwrap_or(117),
        ],
        "jungle_log" => [
            find_swatch("jungle_log_top").unwrap_or(189),
            find_swatch("jungle_log_top").unwrap_or(189),
            find_swatch("jungle_log").unwrap_or(153),
            find_swatch("jungle_log").unwrap_or(153),
            find_swatch("jungle_log").unwrap_or(153),
            find_swatch("jungle_log").unwrap_or(153),
        ],
        "acacia_log" => [
            find_swatch("acacia_log_top").unwrap_or(317),
            find_swatch("acacia_log_top").unwrap_or(317),
            find_swatch("acacia_log").unwrap_or(181),
            find_swatch("acacia_log").unwrap_or(181),
            find_swatch("acacia_log").unwrap_or(181),
            find_swatch("acacia_log").unwrap_or(181),
        ],
        "dark_oak_log" => [
            find_swatch("dark_oak_log_top").unwrap_or(319),
            find_swatch("dark_oak_log_top").unwrap_or(319),
            find_swatch("dark_oak_log").unwrap_or(318),
            find_swatch("dark_oak_log").unwrap_or(318),
            find_swatch("dark_oak_log").unwrap_or(318),
            find_swatch("dark_oak_log").unwrap_or(318),
        ],
        "mangrove_log" => [
            find_swatch("mangrove_log_top").unwrap_or(629),
            find_swatch("mangrove_log_top").unwrap_or(629),
            find_swatch("mangrove_log").unwrap_or(628),
            find_swatch("mangrove_log").unwrap_or(628),
            find_swatch("mangrove_log").unwrap_or(628),
            find_swatch("mangrove_log").unwrap_or(628),
        ],
        "cherry_log" => [
            find_swatch("cherry_log_top").unwrap_or(892),
            find_swatch("cherry_log_top").unwrap_or(892),
            find_swatch("cherry_log").unwrap_or(891),
            find_swatch("cherry_log").unwrap_or(891),
            find_swatch("cherry_log").unwrap_or(891),
            find_swatch("cherry_log").unwrap_or(891),
        ],
        "pale_oak_log" => [
            find_swatch("pale_oak_log_top").unwrap_or(780),
            find_swatch("pale_oak_log_top").unwrap_or(780),
            find_swatch("pale_oak_log").unwrap_or(779),
            find_swatch("pale_oak_log").unwrap_or(779),
            find_swatch("pale_oak_log").unwrap_or(779),
            find_swatch("pale_oak_log").unwrap_or(779),
        ],

        // Stripped logs
        "stripped_oak_log" => [
            default_swatch,
            default_swatch,
            default_swatch,
            default_swatch,
            default_swatch,
            default_swatch,
        ],
        "stripped_spruce_log" => [
            find_swatch("spruce_log_top").unwrap_or(188),
            find_swatch("spruce_log_top").unwrap_or(188),
            default_swatch,
            default_swatch,
            default_swatch,
            default_swatch,
        ],
        "stripped_birch_log" => [
            find_swatch("birch_log_top").unwrap_or(187),
            find_swatch("birch_log_top").unwrap_or(187),
            default_swatch,
            default_swatch,
            default_swatch,
            default_swatch,
        ],
        "stripped_jungle_log" => [
            find_swatch("jungle_log_top").unwrap_or(189),
            find_swatch("jungle_log_top").unwrap_or(189),
            default_swatch,
            default_swatch,
            default_swatch,
            default_swatch,
        ],
        "stripped_acacia_log" => [
            find_swatch("acacia_log_top").unwrap_or(317),
            find_swatch("acacia_log_top").unwrap_or(317),
            default_swatch,
            default_swatch,
            default_swatch,
            default_swatch,
        ],
        "stripped_dark_oak_log" => [
            find_swatch("dark_oak_log_top").unwrap_or(319),
            find_swatch("dark_oak_log_top").unwrap_or(319),
            default_swatch,
            default_swatch,
            default_swatch,
            default_swatch,
        ],
        "stripped_mangrove_log" => [
            find_swatch("stripped_mangrove_log_top").unwrap_or(630),
            find_swatch("stripped_mangrove_log_top").unwrap_or(630),
            find_swatch("stripped_mangrove_log").unwrap_or(631),
            find_swatch("stripped_mangrove_log").unwrap_or(631),
            find_swatch("stripped_mangrove_log").unwrap_or(631),
            find_swatch("stripped_mangrove_log").unwrap_or(631),
        ],
        "stripped_cherry_log" => [
            find_swatch("stripped_cherry_log_top").unwrap_or(894),
            find_swatch("stripped_cherry_log_top").unwrap_or(894),
            find_swatch("stripped_cherry_log").unwrap_or(893),
            find_swatch("stripped_cherry_log").unwrap_or(893),
            find_swatch("stripped_cherry_log").unwrap_or(893),
            find_swatch("stripped_cherry_log").unwrap_or(893),
        ],
        "stripped_pale_oak_log" => [
            find_swatch("stripped_pale_oak_log_top").unwrap_or(782),
            find_swatch("stripped_pale_oak_log_top").unwrap_or(782),
            find_swatch("stripped_pale_oak_log").unwrap_or(781),
            find_swatch("stripped_pale_oak_log").unwrap_or(781),
            find_swatch("stripped_pale_oak_log").unwrap_or(781),
            find_swatch("stripped_pale_oak_log").unwrap_or(781),
        ],

        // Stripped wood (no bark, same on all sides)
        "stripped_oak_wood" => [default_swatch; 6],
        "stripped_spruce_wood" => [default_swatch; 6],
        "stripped_birch_wood" => [default_swatch; 6],
        "stripped_jungle_wood" => [default_swatch; 6],
        "stripped_acacia_wood" => [default_swatch; 6],
        "stripped_dark_oak_wood" => [default_swatch; 6],

        // Barrel (top/bottom different from side)
        "barrel" => [
            default_swatch, // top — barrel top
            default_swatch, // bottom — oak planks
            default_swatch,
            default_swatch,
            default_swatch,
            default_swatch,
        ],

        // Hay block: top = hay_block_top, side = hay_block_side
        "hay_block" => [
            find_swatch("hay_block_top").unwrap_or(250),
            find_swatch("hay_block_top").unwrap_or(250),
            find_swatch("hay_block_side").unwrap_or(249),
            find_swatch("hay_block_side").unwrap_or(249),
            find_swatch("hay_block_side").unwrap_or(249),
            find_swatch("hay_block_side").unwrap_or(249),
        ],

        // Bone block: top = bone_block_top, side = bone_block_side
        "bone_block" => [
            find_swatch("bone_block_top").unwrap_or(420),
            find_swatch("bone_block_top").unwrap_or(420),
            find_swatch("bone_block_side").unwrap_or(419),
            find_swatch("bone_block_side").unwrap_or(419),
            find_swatch("bone_block_side").unwrap_or(419),
            find_swatch("bone_block_side").unwrap_or(419),
        ],

        // Basalt: top = basalt_top, side = basalt_side
        "basalt" => [
            find_swatch("basalt_top").unwrap_or(592),
            find_swatch("basalt_top").unwrap_or(592),
            find_swatch("basalt_side").unwrap_or(593),
            find_swatch("basalt_side").unwrap_or(593),
            find_swatch("basalt_side").unwrap_or(593),
            find_swatch("basalt_side").unwrap_or(593),
        ],
        "polished_basalt" => [
            find_swatch("polished_basalt_top").unwrap_or(594),
            find_swatch("polished_basalt_top").unwrap_or(594),
            find_swatch("polished_basalt_side").unwrap_or(595),
            find_swatch("polished_basalt_side").unwrap_or(595),
            find_swatch("polished_basalt_side").unwrap_or(595),
            find_swatch("polished_basalt_side").unwrap_or(595),
        ],

        // Crimson/warped stems (like logs)
        "crimson_stem" => [
            find_swatch("crimson_stem_top").unwrap_or(568),
            find_swatch("crimson_stem_top").unwrap_or(568),
            find_swatch("crimson_stem").unwrap_or(567),
            find_swatch("crimson_stem").unwrap_or(567),
            find_swatch("crimson_stem").unwrap_or(567),
            find_swatch("crimson_stem").unwrap_or(567),
        ],
        "warped_stem" => [
            find_swatch("warped_stem_top").unwrap_or(570),
            find_swatch("warped_stem_top").unwrap_or(570),
            find_swatch("warped_stem").unwrap_or(569),
            find_swatch("warped_stem").unwrap_or(569),
            find_swatch("warped_stem").unwrap_or(569),
            find_swatch("warped_stem").unwrap_or(569),
        ],
        "stripped_crimson_stem" => [
            find_swatch("stripped_crimson_stem_top").unwrap_or(573),
            find_swatch("stripped_crimson_stem_top").unwrap_or(573),
            find_swatch("stripped_crimson_stem").unwrap_or(575),
            find_swatch("stripped_crimson_stem").unwrap_or(575),
            find_swatch("stripped_crimson_stem").unwrap_or(575),
            find_swatch("stripped_crimson_stem").unwrap_or(575),
        ],
        "stripped_warped_stem" => [
            find_swatch("stripped_warped_stem_top").unwrap_or(574),
            find_swatch("stripped_warped_stem_top").unwrap_or(574),
            find_swatch("stripped_warped_stem").unwrap_or(576),
            find_swatch("stripped_warped_stem").unwrap_or(576),
            find_swatch("stripped_warped_stem").unwrap_or(576),
            find_swatch("stripped_warped_stem").unwrap_or(576),
        ],

        // Crimson/warped nylium (like grass): top=nylium, side=nylium_side, bottom=netherrack
        "crimson_nylium" => [
            find_swatch("crimson_nylium").unwrap_or(585),
            find_swatch("netherrack").unwrap_or(103),
            find_swatch("crimson_nylium_side").unwrap_or(586),
            find_swatch("crimson_nylium_side").unwrap_or(586),
            find_swatch("crimson_nylium_side").unwrap_or(586),
            find_swatch("crimson_nylium_side").unwrap_or(586),
        ],
        "warped_nylium" => [
            find_swatch("warped_nylium").unwrap_or(587),
            find_swatch("netherrack").unwrap_or(103),
            find_swatch("warped_nylium_side").unwrap_or(588),
            find_swatch("warped_nylium_side").unwrap_or(588),
            find_swatch("warped_nylium_side").unwrap_or(588),
            find_swatch("warped_nylium_side").unwrap_or(588),
        ],

        // Target block: top = target_top, side = target_side
        "target" => [
            find_swatch("target_top").unwrap_or(608),
            find_swatch("target_top").unwrap_or(608),
            find_swatch("target_side").unwrap_or(609),
            find_swatch("target_side").unwrap_or(609),
            find_swatch("target_side").unwrap_or(609),
            find_swatch("target_side").unwrap_or(609),
        ],

        // Lodestone: top = lodestone_top, side = lodestone_side
        "lodestone" => [
            find_swatch("lodestone_top").unwrap_or(610),
            find_swatch("lodestone_top").unwrap_or(610),
            find_swatch("lodestone_side").unwrap_or(611),
            find_swatch("lodestone_side").unwrap_or(611),
            find_swatch("lodestone_side").unwrap_or(611),
            find_swatch("lodestone_side").unwrap_or(611),
        ],

        // Observer: front = observer_front, sides vary
        "observer" => [default_swatch; 6],

        // Dispenser/dropper: front = *_front, top = furnace_top
        "dispenser" => [
            find_swatch("furnace_top").unwrap_or(62),
            find_swatch("furnace_top").unwrap_or(62),
            default_swatch,
            default_swatch,
            default_swatch,
            default_swatch,
        ],
        "dropper" => [
            find_swatch("furnace_top").unwrap_or(62),
            find_swatch("furnace_top").unwrap_or(62),
            default_swatch,
            default_swatch,
            default_swatch,
            default_swatch,
        ],

        // Furnace: front = furnace_front, top = furnace_top, side = furnace_side
        "furnace" => [
            find_swatch("furnace_top").unwrap_or(62),
            find_swatch("furnace_top").unwrap_or(62),
            find_swatch("furnace_side").unwrap_or(45),
            find_swatch("furnace_side").unwrap_or(45),
            find_swatch("furnace_side").unwrap_or(45),
            find_swatch("furnace_front").unwrap_or(44), // front faces south by default
        ],
        "lit_furnace" => [
            find_swatch("furnace_top").unwrap_or(62),
            find_swatch("furnace_top").unwrap_or(62),
            find_swatch("furnace_side").unwrap_or(45),
            find_swatch("furnace_side").unwrap_or(45),
            find_swatch("furnace_side").unwrap_or(45),
            find_swatch("furnace_front_on").unwrap_or(61),
        ],

        // Crafting table: top = crafting_table_top, side = crafting_table_side, front = crafting_table_front
        "crafting_table" => [
            find_swatch("crafting_table_top").unwrap_or(43),
            find_swatch("crafting_table_bottom").unwrap_or(919),
            find_swatch("crafting_table_side").unwrap_or(59),
            find_swatch("crafting_table_side").unwrap_or(59),
            find_swatch("crafting_table_side").unwrap_or(59),
            find_swatch("crafting_table_front").unwrap_or(60),
        ],

        // Bookshelf: side = bookshelf, top/bottom = oak_planks
        "bookshelf" => [
            find_swatch("oak_planks").unwrap_or(4),
            find_swatch("oak_planks").unwrap_or(4),
            find_swatch("bookshelf").unwrap_or(35),
            find_swatch("bookshelf").unwrap_or(35),
            find_swatch("bookshelf").unwrap_or(35),
            find_swatch("bookshelf").unwrap_or(35),
        ],

        // Jukebox: side = jukebox_side, top = jukebox_top
        "jukebox" => [
            find_swatch("jukebox_top").unwrap_or(75),
            find_swatch("jukebox_top").unwrap_or(75),
            find_swatch("jukebox_side").unwrap_or(74),
            find_swatch("jukebox_side").unwrap_or(74),
            find_swatch("jukebox_side").unwrap_or(74),
            find_swatch("jukebox_side").unwrap_or(74),
        ],

        // Melon: side = melon_side, top = melon_top
        "melon" => [
            find_swatch("melon_top").unwrap_or(137),
            find_swatch("melon_top").unwrap_or(137),
            find_swatch("melon_side").unwrap_or(136),
            find_swatch("melon_side").unwrap_or(136),
            find_swatch("melon_side").unwrap_or(136),
            find_swatch("melon_side").unwrap_or(136),
        ],

        // Pumpkin/carved_pumpkin: top = pumpkin_top, side = pumpkin_side
        "pumpkin" | "carved_pumpkin" => [
            find_swatch("pumpkin_top").unwrap_or(102),
            find_swatch("pumpkin_top").unwrap_or(102),
            find_swatch("pumpkin_side").unwrap_or(118),
            find_swatch("pumpkin_side").unwrap_or(118),
            find_swatch("pumpkin_side").unwrap_or(118),
            find_swatch("carved_pumpkin").unwrap_or(119),
        ],
        "jack_o_lantern" => [
            find_swatch("pumpkin_top").unwrap_or(102),
            find_swatch("pumpkin_top").unwrap_or(102),
            find_swatch("pumpkin_side").unwrap_or(118),
            find_swatch("pumpkin_side").unwrap_or(118),
            find_swatch("pumpkin_side").unwrap_or(118),
            find_swatch("jack_o_lantern").unwrap_or(120),
        ],

        // Cactus: top = cactus_top, side = cactus_side, bottom = cactus_bottom
        "cactus" => [
            find_swatch("cactus_top").unwrap_or(69),
            find_swatch("cactus_bottom").unwrap_or(71),
            find_swatch("cactus_side").unwrap_or(70),
            find_swatch("cactus_side").unwrap_or(70),
            find_swatch("cactus_side").unwrap_or(70),
            find_swatch("cactus_side").unwrap_or(70),
        ],

        // Dirt path: top = dirt_path_top, side = dirt_path_side
        "dirt_path" | "grass_path" => [
            find_swatch("dirt_path_top").unwrap_or(392),
            find_swatch("dirt").unwrap_or(2),
            find_swatch("dirt_path_side").unwrap_or(393),
            find_swatch("dirt_path_side").unwrap_or(393),
            find_swatch("dirt_path_side").unwrap_or(393),
            find_swatch("dirt_path_side").unwrap_or(393),
        ],

        // Daylight detector: top = daylight_detector_top, side = daylight_detector_side
        "daylight_detector" => [
            find_swatch("daylight_detector_top").unwrap_or(246),
            find_swatch("daylight_detector_top").unwrap_or(246),
            find_swatch("daylight_detector_side").unwrap_or(245),
            find_swatch("daylight_detector_side").unwrap_or(245),
            find_swatch("daylight_detector_side").unwrap_or(245),
            find_swatch("daylight_detector_side").unwrap_or(245),
        ],

        // Note block: oak_planks with note_block texture
        "note_block" => [find_swatch("note_block").unwrap_or(140); 6],

        // Deepslate: top = deepslate_top, side = deepslate
        "deepslate" => [
            find_swatch("deepslate_top").unwrap_or(664),
            find_swatch("deepslate_top").unwrap_or(664),
            find_swatch("deepslate").unwrap_or(663),
            find_swatch("deepslate").unwrap_or(663),
            find_swatch("deepslate").unwrap_or(663),
            find_swatch("deepslate").unwrap_or(663),
        ],

        // Ancient debris: top = ancient_debris_top, side = ancient_debris_side
        "ancient_debris" => [
            find_swatch("ancient_debris_top").unwrap_or(591),
            find_swatch("ancient_debris_top").unwrap_or(591),
            find_swatch("ancient_debris_side").unwrap_or(590),
            find_swatch("ancient_debris_side").unwrap_or(590),
            find_swatch("ancient_debris_side").unwrap_or(590),
            find_swatch("ancient_debris_side").unwrap_or(590),
        ],

        // Shroomlight
        "shroomlight" => [find_swatch("shroomlight").unwrap_or(607); 6],

        // Respawn anchor: top, bottom, 4 levels of side
        "respawn_anchor" => [
            find_swatch("respawn_anchor_top").unwrap_or(612),
            find_swatch("respawn_anchor_bottom").unwrap_or(617),
            find_swatch("respawn_anchor_side0").unwrap_or(613),
            find_swatch("respawn_anchor_side0").unwrap_or(613),
            find_swatch("respawn_anchor_side0").unwrap_or(613),
            find_swatch("respawn_anchor_side0").unwrap_or(613),
        ],

        // Sculk catalyst
        "sculk_catalyst" => [
            find_swatch("sculk_catalyst_top").unwrap_or(656),
            find_swatch("sculk_catalyst_bottom").unwrap_or(658),
            find_swatch("sculk_catalyst_side").unwrap_or(657),
            find_swatch("sculk_catalyst_side").unwrap_or(657),
            find_swatch("sculk_catalyst_side").unwrap_or(657),
            find_swatch("sculk_catalyst_side").unwrap_or(657),
        ],

        // Muddy mangrove roots
        "muddy_mangrove_roots" => [
            find_swatch("muddy_mangrove_roots_top").unwrap_or(637),
            find_swatch("muddy_mangrove_roots_top").unwrap_or(637),
            find_swatch("muddy_mangrove_roots_side").unwrap_or(636),
            find_swatch("muddy_mangrove_roots_side").unwrap_or(636),
            find_swatch("muddy_mangrove_roots_side").unwrap_or(636),
            find_swatch("muddy_mangrove_roots_side").unwrap_or(636),
        ],

        // Mangrove roots
        "mangrove_roots" => [
            find_swatch("mangrove_roots_side").unwrap_or(634),
            find_swatch("mangrove_roots_side").unwrap_or(634),
            find_swatch("mangrove_roots_side").unwrap_or(634),
            find_swatch("mangrove_roots_side").unwrap_or(634),
            find_swatch("mangrove_roots_side").unwrap_or(634),
            find_swatch("mangrove_roots_side").unwrap_or(634),
        ],

        // Copper blocks: use the block (sides) for all faces
        // Top/bottom variants exist in the tile table (copper_block_top etc.)
        // but for the OBJ we use the side texture by default
        "copper_block" => [find_swatch("copper_block").unwrap_or(696); 6],
        "exposed_copper" => [find_swatch("exposed_copper").unwrap_or(697); 6],
        "weathered_copper" => [find_swatch("weathered_copper").unwrap_or(698); 6],
        "oxidized_copper" => [find_swatch("oxidized_copper").unwrap_or(699); 6],
        "waxed_copper_block" => [find_swatch("waxed_copper_block").unwrap_or(700); 6],
        "waxed_exposed_copper" => [find_swatch("waxed_exposed_copper").unwrap_or(701); 6],
        "waxed_weathered_copper" => [find_swatch("waxed_weathered_copper").unwrap_or(702); 6],
        "waxed_oxidized_copper" => [find_swatch("waxed_oxidized_copper").unwrap_or(703); 6],
        "cut_copper" => [find_swatch("cut_copper").unwrap_or(704); 6],
        "exposed_cut_copper" => [find_swatch("exposed_cut_copper").unwrap_or(705); 6],
        "weathered_cut_copper" => [find_swatch("weathered_cut_copper").unwrap_or(706); 6],
        "oxidized_cut_copper" => [find_swatch("oxidized_cut_copper").unwrap_or(707); 6],
        "waxed_cut_copper" => [find_swatch("waxed_cut_copper").unwrap_or(708); 6],

        // All other blocks: same texture on all 6 faces
        _ => [default_swatch; 6],
    };

    // An arm that actually consumed the unresolved default is a genuine
    // lookup failure — report it rather than emitting a bogus swatch index.
    if faces.iter().any(|&f| f == UNRESOLVED) {
        return None;
    }

    Some(faces)
}

/// Get the UV rectangle for a specific block face.
/// Returns `[u0, v0, u1, v1]` in atlas space, or `None` if the block is unknown.
pub fn block_face_uv(name: &str, color: Option<&str>, face: usize) -> Option<[f32; 4]> {
    let swatches = get_swatch_for_block(name, color)?;
    let swatch = swatches.get(face)?;
    swatch_uv(*swatch)
}

/// Load terrainExt.png from embedded bytes and decode it.
/// Returns (pixels, width, height) as RGBA8.
pub fn load_terrain_atlas() -> Result<(Vec<u8>, u32, u32), String> {
    let png_bytes = include_bytes!("../assets/terrainExt.png");
    let img = image::load_from_memory(png_bytes)
        .map_err(|e| format!("Failed to decode terrainExt.png: {e}"))?
        .into_rgba8();
    let (width, height) = img.dimensions();
    let pixels = img.into_raw();
    Ok((pixels, width, height))
}

/// Tint for a swatch texture name that `terrainExt.png` ships pre-desaturated
/// (grayscale), meant to be coloured by the biome at render time — same idea
/// as [`crate::texture::tint_for`] for the JAR-loader path, but this is a
/// *separate*, empirically-verified list: the two atlases don't agree on
/// which textures are grayscale (e.g. this atlas's `mangrove_leaves` and
/// `pale_oak_leaves` swatches are already fully coloured, unlike the JAR's).
/// Deliberately excludes `grass_block_side`: that swatch is a pre-composited
/// dirt+strip image, not grayscale — tinting it as a whole (rather than just
/// its top face) darkens the dirt too. This is why biome tint must be baked
/// per-swatch here rather than applied per-material in the Blender importer.
fn tint_for_atlas_swatch(name: &str) -> Option<[u8; 3]> {
    match name {
        // ── Grass colormap ───────────────────────────────────────────────
        // `grass_block_side` is deliberately absent: unlike the others it is
        // a pre-composited dirt+strip image, not grayscale, so tinting the
        // whole tile browns out the dirt. Its separate `_overlay` strip is
        // the part that takes the colour.
        "grass_block_top"
        | "grass_block_side_overlay"
        | "short_grass"
        | "fern"
        | "tall_grass_top"
        | "tall_grass_bottom"
        | "large_fern_top"
        | "large_fern_bottom"
        | "melon_stem"
        | "attached_melon_stem"
        | "pumpkin_stem"
        | "attached_pumpkin_stem" => Some(crate::texture::PLAINS_GRASS),

        // ── Foliage colormap ─────────────────────────────────────────────
        // `bush` and `leaf_litter` (1.21.5) use the dry-foliage colour in
        // some biomes; foliage green is the closest single approximation.
        // `pale_oak_leaves` is deliberately absent here while being present in
        // the prototype table: the two draw from different images. The client
        // JAR ships it grayscale (max channel spread 10/255), but this atlas
        // ships that tile already coloured (spread 0.029), so tinting it here
        // would double-darken it. `every_tinted_swatch_is_actually_grayscale`
        // measures the atlas and enforces exactly that.
        "oak_leaves" | "acacia_leaves" | "dark_oak_leaves" | "birch_leaves"
        | "mangrove_leaves" | "vine" | "bush" | "leaf_litter" => {
            Some(crate::texture::PLAINS_FOLIAGE)
        }

        // Spruce has its own fixed colour rather than a biome lookup.
        "spruce_leaves" => Some([0x61, 0x99, 0x61]),
        // Lily pad is a fixed green in game, not biome-driven.
        "lily_pad" => Some([0x20, 0x80, 0x30]),

        // Everything else ships already-coloured in this atlas and must be
        // left alone — jungle/cherry/azalea/pale_oak leaves, firefly_bush,
        // sugar_cane and moss_block all measure as clearly non-grayscale.
        _ => None,
    }
}

/// Multiply the RGB channels (alpha untouched) of every biome-tintable
/// swatch's 16x16 region in-place within the full atlas pixel buffer.
fn tint_biome_swatches(pixels: &mut [u8], width: u32, height: u32) {
    let tile = 16usize;
    let width = width as usize;
    let height = height as usize;
    // The table is not one-entry-per-tile: several entries can share the same
    // (col, row) when one image serves multiple block states. Tinting is a
    // multiply applied in place, so visiting a tile twice would darken it
    // twice — track which tiles have been done.
    let mut tinted: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
    for &(col, row, name) in TILE_TABLE.iter() {
        let Some(tint) = tint_for_atlas_swatch(name) else {
            continue;
        };
        if !tinted.insert((col, row)) {
            continue;
        }
        let x0 = col as usize * tile;
        let y0 = row as usize * tile;
        if x0 + tile > width || y0 + tile > height {
            continue;
        }
        for y in y0..y0 + tile {
            for x in x0..x0 + tile {
                let idx = (y * width + x) * 4;
                pixels[idx] = (u16::from(pixels[idx]) * u16::from(tint[0]) / 255) as u8;
                pixels[idx + 1] = (u16::from(pixels[idx + 1]) * u16::from(tint[1]) / 255) as u8;
                pixels[idx + 2] = (u16::from(pixels[idx + 2]) * u16::from(tint[2]) / 255) as u8;
            }
        }
    }
}

/// Build a `FaceAwareTileSet` from the Mineways terrain atlas.
/// This replaces the dynamic JAR-based atlas for OBJ export.
///
/// `texture_keys` should come from [`crate::chunk::Chunk::texture_keys`],
/// not `block_names` — each entry is a plain block name (`minecraft:stone`)
/// or a colour-suffixed key (`minecraft:wool|red`) for legacy blocks whose
/// colour lives in a separate state. Using `block_names` instead would
/// collapse every colour of wool/concrete/glass onto whichever one
/// happened to be inserted first.
pub fn build_mineways_tileset(texture_keys: &[String]) -> crate::texture::FaceAwareTileSet {
    use crate::texture::{FaceAwareTileSet, TileSet};

    let (mut pixels, width, height) = load_terrain_atlas().unwrap_or_else(|e| {
        tracing::warn!("Failed to load Mineways atlas: {e}, falling back to procedural");
        // Return a 16x16 placeholder
        (vec![0u8; 16 * 16 * 4], 16, 16)
    });
    tint_biome_swatches(&mut pixels, width, height);

    // Build per-face UV mappings for all requested blocks
    // The atlas is a single large image (terrainExt.png), not a packed set of tiles.
    // We just need pixels/width/height for the atlas image.
    // Neutral stand-in for blocks with no swatch in this atlas. Swatch 0 is
    // `grass_block_top`, so the old fallback silently rendered every
    // unrecognised block as bright (and, since tinting, green) grass —
    // indistinguishable from real terrain and easy to miss in a render.
    // `terrainExt.png` predates newer blocks (1.21.5's `leaf_litter`, `bush`,
    // `firefly_bush`, plus `sulfur`/`cinnabar`), so misses are expected on an
    // up-to-date save; stone at least reads as "generic block".
    const NEUTRAL_SWATCH: usize = 1; // "stone"

    let mut face_uvs = std::collections::HashMap::new();
    // Pass 1: resolve every key against the Mineways table, noting the ones
    // it has no tile for. Those get filled in from the game's own JAR below.
    let mut resolved: Vec<(&String, [usize; 6])> = Vec::new();
    let mut unmapped: Vec<&str> = Vec::new();
    for key in texture_keys {
        let (name, color) = split_texture_key(key);
        let short = strip_namespace(name);
        // Air emits no geometry, so it never needs a tile — resolving it
        // would only add noise to the "missing texture" report.
        if crate::blocks::is_air(short) {
            continue;
        }
        match get_swatch_for_block(short, color) {
            Some(faces) => resolved.push((key, faces)),
            None => unmapped.push(short),
        }
    }
    unmapped.sort_unstable();
    unmapped.dedup();

    // Pass 2: pull the missing textures straight from the installed client
    // JAR and append them to the atlas as extra rows. `terrainExt.png` is a
    // snapshot of whatever blocks existed when it was built, so an
    // up-to-date save always has some it predates; the JAR is by definition
    // current with the world being exported.
    let extra_tiles = if unmapped.is_empty() {
        Vec::new()
    } else {
        collect_jar_tiles(&unmapped)
    };
    let extra_index: std::collections::HashMap<&str, usize> = extra_tiles
        .iter()
        .enumerate()
        .map(|(i, (block, _, _))| (block.as_str(), i))
        .collect();

    // Pass 3: grow the pixel buffer, then compute every UV against the final
    // height — appending rows changes the V of *existing* tiles too, so the
    // UVs cannot be taken from `swatch_uv`'s fixed-height constant.
    let base_rows = height / 16;
    let extra_rows = (extra_tiles.len() as u32).div_ceil(16);
    let final_height = height + extra_rows * 16;
    if extra_rows > 0 {
        pixels.resize((width * final_height * 4) as usize, 0);
        for (i, (_, _, tile)) in extra_tiles.iter().enumerate() {
            let col = i as u32 % 16;
            let row = base_rows + i as u32 / 16;
            for ty in 0..16u32 {
                let dst = (((row * 16 + ty) * width + col * 16) * 4) as usize;
                let src = (ty * 16 * 4) as usize;
                pixels[dst..dst + 64].copy_from_slice(&tile[src..src + 64]);
            }
        }
    }

    // Inset by half a texel. Sampling right on a tile boundary lets the
    // neighbouring tile bleed in under any filtering, which shows up as a thin
    // grid of wrong-coloured lines tracing every block edge in the model.
    let uv_for = |col: u32, row: u32| -> [f32; 4] {
        let du = 0.5 / 256.0;
        let dv = 0.5 / final_height as f32;
        [
            col as f32 / 16.0 + du,
            row as f32 * 16.0 / final_height as f32 + dv,
            (col + 1) as f32 / 16.0 - du,
            (row + 1) as f32 * 16.0 / final_height as f32 - dv,
        ]
    };
    let uv_for_swatch = |swatch: usize| -> Option<[f32; 4]> {
        let &(col, row, _) = TILE_TABLE.get(swatch)?;
        Some(uv_for(col, row))
    };

    for (key, faces) in resolved {
        for (i, &swatch) in faces.iter().enumerate() {
            if let Some(uv) = uv_for_swatch(swatch) {
                face_uvs.insert((key.clone(), i), uv);
            }
        }
    }

    let mut still_missing: Vec<&str> = Vec::new();
    for key in texture_keys {
        let (name, _) = split_texture_key(key);
        let short = strip_namespace(name);
        if crate::blocks::is_air(short) || face_uvs.contains_key(&(key.clone(), 0)) {
            continue;
        }
        let uv = match extra_index.get(short) {
            Some(&i) => uv_for(i as u32 % 16, base_rows + i as u32 / 16),
            None => {
                still_missing.push(short);
                uv_for_swatch(NEUTRAL_SWATCH).unwrap_or([0.0, 0.0, 1.0, 1.0])
            }
        };
        for i in 0..6 {
            face_uvs.insert((key.clone(), i), uv);
        }
    }

    if !extra_tiles.is_empty() {
        tracing::info!(
            "Filled {} block(s) missing from terrainExt.png using textures from the client JAR: {}",
            extra_tiles.len(),
            extra_tiles
                .iter()
                .map(|(b, t, _)| format!("{b} -> {t}"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    still_missing.sort_unstable();
    still_missing.dedup();
    if !still_missing.is_empty() {
        tracing::warn!(
            "{} block type(s) have no texture in either terrainExt.png or the \
             client JAR and were drawn as plain stone: {}",
            still_missing.len(),
            still_missing.join(", ")
        );
    }

    // The face_uvs map holds all the UV data; the tile set just carries the
    // atlas image (terrainExt.png plus any JAR-sourced rows appended above).
    let atlas = TileSet::from_raw(pixels, width, final_height);

    // Direct texture-name -> UV lookup, for model-driven geometry: a vanilla
    // block model names the texture on each face rather than relying on the
    // block's face order.
    let mut texture_uvs = std::collections::HashMap::new();
    for (index, &(col, row, name)) in TILE_TABLE.iter().enumerate() {
        let _ = index;
        texture_uvs.entry(name.to_owned()).or_insert_with(|| uv_for(col, row));
    }
    for (i, (_, tex_name, _)) in extra_tiles.iter().enumerate() {
        texture_uvs.insert(
            tex_name.clone(),
            uv_for(i as u32 % 16, base_rows + i as u32 / 16),
        );
    }

    FaceAwareTileSet { atlas, face_uvs, texture_uvs }
}

/// Load a 16x16 RGBA tile per block name from the installed client JAR.
///
/// Returns `(block_name, texture_name, tile_pixels)`. Blocks with no matching
/// texture are simply absent from the result.
/// Further texture names to try for a block whose own name finds nothing.
///
/// The JAR does not ship a texture per *block*; it ships one per *image*, and
/// several block families borrow another block's image rather than owning one.
/// Without these the blocks fall through to plain stone, which is both wrong
/// and silent unless you read the log.
fn texture_aliases(block: &str) -> Vec<String> {
    let mut out = Vec::new();

    // `*_wood` and `*_hyphae` are the all-bark variants of a log or stem, and
    // reuse that block's side texture rather than shipping their own.
    if let Some(base) = block.strip_suffix("_wood") {
        out.push(format!("{base}_log"));
    }
    if let Some(base) = block.strip_suffix("_hyphae") {
        out.push(format!("{base}_stem"));
    }
    // Stripped variants follow the same rule one level down.
    if let Some(base) = block.strip_suffix("_wood").and_then(|b| b.strip_prefix("stripped_")) {
        out.push(format!("stripped_{base}_log"));
    }

    // Signs, banners and heads are drawn by the game from entity textures, so
    // there is no block image at all. Fall back to the material they are made
    // of: wrong in detail, but the right colour and far better than stone.
    if let Some(base) = block
        .strip_suffix("_wall_hanging_sign")
        .or_else(|| block.strip_suffix("_hanging_sign"))
        .or_else(|| block.strip_suffix("_wall_sign"))
        .or_else(|| block.strip_suffix("_sign"))
    {
        out.push(format!("{base}_planks"));
    }
    if block.ends_with("_banner") {
        out.push("white_wool".to_owned());
    }

    match block {
        // A bubble column is water with rising bubbles; the water surface is
        // the only part with an image.
        "bubble_column" => out.push("water_still".to_owned()),
        // Mob heads are entity-rendered. Approximate with the mob's block.
        "piglin_head" | "piglin_wall_head" => out.push("nether_bricks".to_owned()),
        "player_head" | "player_wall_head" => out.push("dirt".to_owned()),
        "skeleton_skull" | "skeleton_wall_skull" | "wither_skeleton_skull"
        | "wither_skeleton_wall_skull" => out.push("bone_block_side".to_owned()),
        "zombie_head" | "zombie_wall_head" => out.push("moss_block".to_owned()),
        "creeper_head" | "creeper_wall_head" => out.push("green_wool".to_owned()),
        "dragon_head" | "dragon_wall_head" => out.push("black_wool".to_owned()),
        _ => {}
    }

    // A block cut from another one borrows its texture. `_planks` is included
    // because the wooden families name their material that way: an
    // `acacia_pressure_plate` is cut from `acacia_planks`, and stripping the
    // suffix alone only gets as far as `acacia`, which is not a texture.
    for suffix in [
        "_slab",
        "_stairs",
        "_wall",
        "_fence",
        "_fence_gate",
        "_pressure_plate",
        "_button",
        "_trapdoor",
    ] {
        if let Some(base) = block.strip_suffix(suffix) {
            out.push(base.to_owned());
            out.push(format!("{base}_planks"));
            out.push(format!("{base}_block"));
            out.push(format!("{base}s"));
        }
    }

    out
}

fn collect_jar_tiles(blocks: &[&str]) -> Vec<(String, String, Vec<u8>)> {
    let loader = match crate::jar_textures::JarTextureLoader::load() {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("No client JAR available to fill missing block textures: {e}");
            return Vec::new();
        }
    };

    let mut out = Vec::new();
    for &block in blocks {
        // Exact name, then the usual per-face spellings, then any texture
        // whose name starts with the block's (multi-part blocks such as
        // `sulfur_spike` only ship `sulfur_spike_up_base` and friends).
        let mut direct = vec![
            block.to_owned(),
            format!("{block}_top"),
            format!("{block}_side"),
            format!("{block}_still"),
        ];
        direct.extend(texture_aliases(block));
        let found = direct
            .iter()
            .find_map(|n| loader.get(n).map(|b| (n.clone(), b)))
            .or_else(|| {
                loader
                    .find_prefixed(&format!("{block}_"))
                    .map(|(n, b)| (n.to_owned(), b))
            });

        let Some((tex_name, png)) = found else {
            continue;
        };
        let Some(mut tile) = decode_jar_tile(png) else {
            tracing::warn!("could not decode JAR texture {tex_name} for {block}");
            continue;
        };
        // Grayscale foliage from the JAR needs the same biome tint the
        // baked-in atlas swatches get.
        if let Some(tint) = tint_for_atlas_swatch(&tex_name) {
            for px in tile.chunks_exact_mut(4) {
                for c in 0..3 {
                    px[c] = (u16::from(px[c]) * u16::from(tint[c]) / 255) as u8;
                }
            }
        }
        out.push((block.to_owned(), tex_name, tile));
    }
    out
}

/// Decode a JAR block PNG into a 16x16 RGBA tile.
///
/// Animated textures (water, lava, fire, ...) are stored as a vertical strip
/// of square frames rather than one image, so a naive resize would squash the
/// whole animation into one tile; take the first frame instead.
fn decode_jar_tile(png: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory(png).ok()?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return None;
    }
    let frame = if h > w { img.crop_imm(0, 0, w, w) } else { img };
    let raw = frame
        .resize_exact(16, 16, image::imageops::FilterType::Nearest)
        .to_rgba8()
        .into_raw();
    (raw.len() == 16 * 16 * 4).then_some(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The atlas and the prototype exporter each decide tinting from their own
    /// list, and the two drifted: `bush` and `leaf_litter` were added to the
    /// atlas list only, so the same block came out green when drawn from the
    /// atlas and colourless when drawn as a prototype. Any name either list
    /// tints, both must tint, and to the same colour.
    #[test]
    fn both_tint_tables_agree() {
        // Every name mentioned by either table, checked in both directions.
        const NAMES: &[&str] = &[
            "grass_block_top", "grass_block_side_overlay", "grass_block_side",
            "short_grass", "fern", "tall_grass_top", "tall_grass_bottom",
            "large_fern_top", "large_fern_bottom", "melon_stem", "pumpkin_stem",
            "attached_melon_stem", "attached_pumpkin_stem",
            "oak_leaves", "birch_leaves", "acacia_leaves", "dark_oak_leaves",
            "mangrove_leaves", "spruce_leaves", "jungle_leaves", "cherry_leaves",
            "vine", "bush", "leaf_litter", "lily_pad", "firefly_bush", "pale_oak_leaves", "azalea_leaves",
            "moss_block", "sugar_cane", "dirt", "stone",
        ];
        // The two tables read different images, so they are allowed to differ
        // where those images differ. Every entry needs a measured reason.
        //
        // `pale_oak_leaves`: grayscale in the client JAR (spread 10/255) but
        // already coloured in this atlas (spread 0.029), so only the prototype
        // path, which uses the JAR, may tint it.
        //
        // `jungle_leaves`: the same split. The vanilla PNG measures 0.07 mean
        // saturation -- grayscale, next to `oak_leaves` at 0.01 and genuinely
        // coloured `cherry_leaves` at 0.72 -- so the JAR path must tint it or
        // the canopy renders grey, while this atlas ships the swatch already
        // coloured and must not.
        const MEASURED_EXCEPTIONS: &[&str] = &["pale_oak_leaves", "jungle_leaves"];

        let mut disagree = Vec::new();
        for name in NAMES {
            if MEASURED_EXCEPTIONS.contains(name) {
                continue;
            }
            let atlas = tint_for_atlas_swatch(name);
            let proto = crate::texture::biome_tint(name);
            if atlas != proto {
                disagree.push(format!("{name}: atlas={atlas:?} prototype={proto:?}"));
            }
        }
        assert!(
            disagree.is_empty(),
            "tint tables disagree, so the same block renders differently \
             depending on which path draws it:\n  {}",
            disagree.join("\n  "),
        );
    }

    /// Tinting multiplies a grayscale tile by a biome colour. Applying it to a
    /// tile that already ships coloured double-darkens it, and *skipping* a
    /// grayscale one leaves it rendering as flat gray (this is why `bush` and
    /// `leaf_litter` came out as white blobs). Neither is visible in the code
    /// itself, so derive the expectation from the atlas pixels: every tinted
    /// tile must actually be grayscale.
    /// Filling a block from the client JAR appends rows to the atlas, which
    /// changes its height — and therefore the V coordinate of *every* tile,
    /// including ones resolved from the baked-in table. The UVs must be
    /// computed against the final height, not the table's fixed-height
    /// constant, or adding one new block silently slides every existing
    /// texture up the image.
    #[test]
    fn appending_jar_tiles_keeps_existing_uvs_pointing_at_the_same_pixels() {
        let base = build_mineways_tileset(&["minecraft:stone".to_owned()]);
        let base_h = base.atlas.height;
        let stone_uv = base.face_uv("minecraft:stone", 0);

        // `sulfur` is absent from terrainExt.png, so this run has to append.
        let extended = build_mineways_tileset(&[
            "minecraft:stone".to_owned(),
            "minecraft:sulfur".to_owned(),
        ]);
        if extended.atlas.height == base_h {
            return; // no client JAR on this machine; nothing was appended
        }
        assert!(extended.atlas.height > base_h, "atlas should have grown");

        // Same tile, taller image => same pixel row, so V must have shrunk in
        // exact proportion to the height increase.
        let scale = f64::from(base_h) / f64::from(extended.atlas.height);
        let stone_uv2 = extended.face_uv("minecraft:stone", 0);
        for i in [1usize, 3] {
            let expected = f64::from(stone_uv[i]) * scale;
            assert!(
                (f64::from(stone_uv2[i]) - expected).abs() < 1e-6,
                "stone V[{i}] became {} but should be {expected} after the atlas \
                 grew from {base_h} to {} — existing UVs were not rescaled",
                stone_uv2[i],
                extended.atlas.height
            );
        }

        // And the appended block must land on a real, non-empty tile.
        let sulfur_uv = extended.face_uv("minecraft:sulfur", 0);
        assert!(
            sulfur_uv[3] > f32::EPSILON && sulfur_uv != [0.0, 0.0, 1.0, 1.0],
            "sulfur did not get its own tile: {sulfur_uv:?}"
        );
    }

    #[test]
    fn every_tinted_swatch_is_actually_grayscale() {
        let (pixels, width, height) = load_terrain_atlas().unwrap();
        for &(col, row, name) in TILE_TABLE.iter() {
            if tint_for_atlas_swatch(name).is_none() {
                continue;
            }
            let (mut acc, mut n) = ([0f32; 3], 0f32);
            for y in row * 16..row * 16 + 16 {
                for x in col * 16..col * 16 + 16 {
                    if x >= width || y >= height {
                        continue;
                    }
                    let i = ((y * width + x) * 4) as usize;
                    if pixels[i + 3] < 128 {
                        continue;
                    }
                    for c in 0..3 {
                        acc[c] += f32::from(pixels[i + c]);
                    }
                    n += 1.0;
                }
            }
            assert!(n > 0.0, "{name}: tinted tile ({col},{row}) is empty");
            let avg = acc.map(|v| v / n / 255.0);
            let spread = (avg[0] - avg[1])
                .abs()
                .max((avg[1] - avg[2]).abs())
                .max((avg[0] - avg[2]).abs());
            assert!(
                spread < 0.02,
                "{name} at ({col},{row}) averages {avg:?} (spread {spread:.3}) — it already \
                 carries colour, so multiplying a biome tint over it double-darkens the tile"
            );
        }
    }

    #[test]
    fn tint_only_touches_grayscale_swatches_not_grass_side() {
        let (mut pixels, width, height) = load_terrain_atlas().unwrap();

        let tile_avg = |pixels: &[u8], col: u32, row: u32| -> [u8; 3] {
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in (row * 16)..(row * 16 + 16) {
                for x in (col * 16)..(col * 16 + 16) {
                    let idx = ((y * width + x) * 4) as usize;
                    r += u32::from(pixels[idx]);
                    g += u32::from(pixels[idx + 1]);
                    b += u32::from(pixels[idx + 2]);
                    n += 1;
                }
            }
            [(r / n) as u8, (g / n) as u8, (b / n) as u8]
        };

        // grass_block_side (3, 0) is a pre-coloured dirt+strip composite,
        // not grayscale — must be untouched by tinting.
        let side_before = tile_avg(&pixels, 3, 0);
        // grass_block_top (0, 0) IS grayscale — must change (darken/green).
        let top_before = tile_avg(&pixels, 0, 0);

        tint_biome_swatches(&mut pixels, width, height);

        let side_after = tile_avg(&pixels, 3, 0);
        let top_after = tile_avg(&pixels, 0, 0);

        assert_eq!(side_before, side_after, "grass_block_side must not be tinted");
        assert_ne!(top_before, top_after, "grass_block_top must be tinted");
    }

    /// The swatch table is positional, so it only means anything paired with
    /// the `terrainExt.png` revision it was generated from. When the two drift
    /// apart, lookups still succeed but land on whatever tile now occupies
    /// that slot — deepslate came out white, tuff diamond-blue, calcite slate,
    /// while low-index blocks like stone and dirt stayed correct and hid the
    /// problem. Checks a few high-index swatches against an independent colour
    /// source so a mismatched regeneration fails here instead of in a render.
    #[test]
    fn swatch_table_matches_the_bundled_atlas() {
        let (pixels, width, height) = load_terrain_atlas().unwrap();
        assert_eq!(
            height as usize,
            (TILE_TABLE.iter().map(|t| t.1).max().unwrap() as usize + 1) * 16,
            "atlas height and table row count disagree — regenerate with tools/gen_mineways_data.py"
        );

        let tile_avg = |col: u32, row: u32| -> [f32; 3] {
            let (mut acc, mut n) = ([0f32; 3], 0f32);
            for y in row * 16..row * 16 + 16 {
                for x in col * 16..col * 16 + 16 {
                    let i = ((y * width + x) * 4) as usize;
                    if pixels[i + 3] < 128 {
                        continue;
                    }
                    for c in 0..3 {
                        acc[c] += f32::from(pixels[i + c]);
                    }
                    n += 1.0;
                }
            }
            acc.map(|v| if n == 0.0 { 0.0 } else { v / n / 255.0 })
        };

        // High-index blocks: these are the ones a stale table gets wrong.
        for block in ["deepslate", "tuff", "calcite", "andesite", "granite"] {
            let faces = get_swatch_for_block(block, None)
                .unwrap_or_else(|| panic!("{block} has no swatch"));
            let (col, row, _) = TILE_TABLE[faces[2]];
            let got = tile_avg(col, row);
            let want = crate::blocks::block_color(block);
            let delta: f32 = (0..3).map(|c| (got[c] - want[c]).abs()).sum();
            assert!(
                delta < 0.45,
                "{block}: atlas tile ({col},{row}) is {got:?} but should look like \
                 {want:?} (delta {delta:.2}) — swatch table and terrainExt.png are \
                 out of sync; regenerate with tools/gen_mineways_data.py"
            );
        }
    }

    #[test]
    fn pillar_face_remap_is_identity_for_vertical_axis() {
        for face in 0..6 {
            assert_eq!(remap_pillar_face(face, "y"), face);
            assert_eq!(remap_pillar_face(face, "unknown"), face);
        }
    }

    #[test]
    fn pillar_face_remap_moves_end_grain_for_horizontal_axis() {
        // axis=x: end-grain belongs on the east/west faces (2, 3), not top/bottom (0, 1).
        assert_eq!(remap_pillar_face(2, "x"), 0);
        assert_eq!(remap_pillar_face(3, "x"), 1);
        for bark_face in [0, 1, 4, 5] {
            assert_eq!(remap_pillar_face(bark_face, "x"), 2);
        }

        // axis=z: end-grain belongs on the south/north faces (4, 5).
        assert_eq!(remap_pillar_face(4, "z"), 0);
        assert_eq!(remap_pillar_face(5, "z"), 1);
        for bark_face in [0, 1, 2, 3] {
            assert_eq!(remap_pillar_face(bark_face, "z"), 2);
        }
    }

    #[test]
    fn atlas_loads_successfully() {
        let (pixels, w, h) = load_terrain_atlas().unwrap();
        assert_eq!(w, 256);
        assert_eq!(h, 1264);
        assert_eq!(pixels.len(), 256 * 1264 * 4);
        // Check a few pixels - first pixel should not be black
        assert!(pixels[0] > 0 || pixels[1] > 0 || pixels[2] > 0);
    }

    #[test]
    fn common_blocks_have_uvs() {
        let blocks = &[
            "minecraft:stone",
            "minecraft:dirt",
            "minecraft:grass_block",
            "minecraft:oak_log",
            "minecraft:oak_planks",
            "minecraft:cobblestone",
            "minecraft:bedrock",
            "minecraft:water",
            "minecraft:sand",
            "minecraft:gravel",
            "minecraft:iron_ore",
            "minecraft:deepslate",
            "minecraft:tuff",
            "minecraft:andesite",
            "minecraft:diorite",
            "minecraft:granite",
        ];
        let names: Vec<String> = blocks.iter().map(|s| s.to_string()).collect();
        let tiles = build_mineways_tileset(&names);
        for name in blocks {
            for face in 0..6usize {
                let uv = tiles.face_uv(name, face);
                assert!(
                    uv[0] >= 0.0 && uv[0] <= 1.0,
                    "{name} face {face} u0={}",
                    uv[0]
                );
                assert!(
                    uv[1] >= 0.0 && uv[1] <= 1.0,
                    "{name} face {face} v0={}",
                    uv[1]
                );
                assert!(
                    uv[2] >= 0.0 && uv[2] <= 1.0,
                    "{name} face {face} u1={}",
                    uv[2]
                );
                assert!(
                    uv[3] >= 0.0 && uv[3] <= 1.0,
                    "{name} face {face} v1={}",
                    uv[3]
                );
                assert!(uv[2] > uv[0], "{name} face {face} u1 <= u0");
                assert!(uv[3] > uv[1], "{name} face {face} v1 <= v0");
            }
        }
    }

    #[test]
    fn print_unknown_block_swatches() {
        // Test blocks that might be in a real world but not explicitly handled
        let unknowns = &[
            "minecraft:polished_deepslate",
            "minecraft:cobbled_deepslate",
            "minecraft:deepslate_bricks",
            "minecraft:deepslate_tiles",
            "minecraft:calcite",
            "minecraft:smooth_basalt",
            "minecraft:dripstone_block",
            "minecraft:raw_iron_block",
            "minecraft:raw_copper_block",
            "minecraft:copper_ore",
            "minecraft:deepslate_iron_ore",
            "minecraft:deepslate_copper_ore",
            "minecraft:moss_block",
            "minecraft:mud",
            "minecraft:mud_bricks",
            "minecraft:packed_mud",
            "minecraft:rooted_dirt",
            "minecraft:azalea_leaves",
        ];
        for name in unknowns {
            let short = strip_namespace(name);
            let swatches = get_swatch_for_block(short, None);
            match swatches {
                Some(s) => println!(
                    "  {name}: swatches=[{},{},{},{},{},{}]",
                    swatch_uv(s[0]).unwrap_or([-1.0; 4])[0],
                    swatch_uv(s[0]).unwrap_or([-1.0; 4])[1],
                    swatch_uv(s[0]).unwrap_or([-1.0; 4])[2],
                    swatch_uv(s[0]).unwrap_or([-1.0; 4])[3],
                    s[0],
                    s[1]
                ),
                None => println!("  {name}: NO SWATCH FOUND ❌"),
            }
        }
    }

    #[test]
    fn grass_block_has_correct_faces() {
        let names = vec!["minecraft:grass_block".to_string()];
        let tiles = build_mineways_tileset(&names);
        // top face (0)
        let top = tiles.face_uv("minecraft:grass_block", 0);
        println!("grass_block top UV: {top:?}");
        // bottom face (1)
        let bot = tiles.face_uv("minecraft:grass_block", 1);
        println!("grass_block bottom UV: {bot:?}");
        // side face (2 = east)
        let side = tiles.face_uv("minecraft:grass_block", 2);
        println!("grass_block side UV: {side:?}");
        // All three should be different
        assert!(top != side, "grass_block top and side UVs should differ");
        assert!(bot != side, "grass_block bottom and side UVs should differ");
    }
}
