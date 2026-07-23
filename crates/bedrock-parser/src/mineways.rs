//! Mineways-compatible block texture mapping.
//!
//! Uses Mineways' `terrainExt.png` texture atlas and `gTilesTable[]`
//! tile-position data to assign correct per-face textures to any block.
//! This replaces the homegrown `block_model.rs` + `jar_textures.rs` pipeline
//! with Mineways' battle-tested 15-year block-definition database.

use crate::block_definitions::G_BLOCK_DEFINITIONS;
use crate::chunk::strip_namespace;
use crate::mineways_data::{swatch_by_filename, swatch_uv};

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

    let default_swatch = find_swatch(tex_name).or_else(|| swatch_from_definitions(short))?;

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

    let (pixels, width, height) = load_terrain_atlas().unwrap_or_else(|e| {
        tracing::warn!("Failed to load Mineways atlas: {e}, falling back to procedural");
        // Return a 16x16 placeholder
        (vec![0u8; 16 * 16 * 4], 16, 16)
    });

    // Build per-face UV mappings for all requested blocks
    // The atlas is a single large image (terrainExt.png), not a packed set of tiles.
    // We just need pixels/width/height for the atlas image.
    let mut face_uvs = std::collections::HashMap::new();
    for key in texture_keys {
        let (name, color) = split_texture_key(key);
        let short = strip_namespace(name);
        let faces = get_swatch_for_block(short, color).unwrap_or([0; 6]);
        for (i, &swatch) in faces.iter().enumerate() {
            if let Some(uv) = swatch_uv(swatch) {
                face_uvs.insert((key.clone(), i), uv);
            }
        }
    }

    // Build a minimal tile set — the face_uvs map has all the UV data.
    // The atlas pixel buffer is terrainExt.png itself.
    let atlas = TileSet::from_raw(pixels, width, height);

    FaceAwareTileSet { atlas, face_uvs }
}

#[cfg(test)]
mod tests {
    use super::*;

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
