//! Per-face texture resolution for Minecraft blocks.
//!
//! Maps a block name (without namespace) to the six texture names used on its
//! faces. Most blocks are uniform (same texture on all six faces), but some
//! important ones — grass, logs, TNT, crafting tables, pistons — use
//! different textures on the top, bottom, and sides.
//!
//! This is a **static lookup table** covering the most common blocks. Blocks
//! not in the table fall back to a single texture derived from the block name.
//! This is intentionally simpler than parsing blockstate/model JSON — it gets
//! 95 % of visual results for 5 % of the complexity.

use crate::chunk::strip_namespace;

/// The six texture names for one block (without path prefix or `.png`).
///
/// Each field is a Minecraft texture name such as `"grass_block_top"` or
/// `"oak_log"`. These match the file names inside the client JAR under
/// `assets/minecraft/textures/block/`.
#[derive(Debug, Clone)]
pub struct FaceTextures {
    /// +Y face (the top).
    pub top: String,
    /// -Y face (the bottom).
    pub bottom: String,
    /// +Z face (south / front).
    pub south: String,
    /// -Z face (north / back).
    pub north: String,
    /// +X face (east / right).
    pub east: String,
    /// -X face (west / left).
    pub west: String,
}

impl FaceTextures {
    /// A block that uses the same texture on every face.
    fn uniform(name: &str) -> Self {
        Self {
            top: name.into(),
            bottom: name.into(),
            south: name.into(),
            north: name.into(),
            east: name.into(),
            west: name.into(),
        }
    }

    /// A block with different top/bottom and uniform sides.
    fn column(top: &str, bottom: &str, side: &str) -> Self {
        Self {
            top: top.into(),
            bottom: bottom.into(),
            south: side.into(),
            north: side.into(),
            east: side.into(),
            west: side.into(),
        }
    }

    /// Unique textures on all three axes (top, bottom, sides).
    fn tri(top: &str, bottom: &str, side: &str) -> Self {
        Self::column(top, bottom, side)
    }

    /// Return the texture name for a face given its outward normal index.
    ///
    /// Normal index matches the `FACES` constant in `mesh.rs`:
    /// `0` = +Y, `1` = -Y, `2` = +X, `3` = -X, `4` = +Z, `5` = -Z.
    pub fn for_face_index(&self, face: usize) -> &str {
        match face {
            0 => &self.top,
            1 => &self.bottom,
            2 => &self.east,
            3 => &self.west,
            4 => &self.south,
            5 => &self.north,
            _ => &self.top,
        }
    }
}

static RESOLVER: std::sync::OnceLock<Option<crate::json_model::BlockModelResolver>> =
    std::sync::OnceLock::new();

pub fn get_resolver() -> Option<&'static crate::json_model::BlockModelResolver> {
    RESOLVER
        .get_or_init(|| {
            crate::assets_extractor::VanillaAssets::load()
                .ok()
                .map(|assets| crate::json_model::BlockModelResolver::new(&assets))
        })
        .as_ref()
}

/// Resolve the six face textures for a block name (namespace optional).
///
/// Unknown blocks fall back to a single texture named after the block itself.
pub fn face_textures(block: &str) -> FaceTextures {
    let short = strip_namespace(block);

    // Try dynamic JSON definitions first
    if let Some(res) = get_resolver() {
        if res.block_textures.contains_key(short) {
            return res.face_textures(short);
        }
    }

    if let Some(ft) = bed_face_textures(short) {
        return ft;
    }
    if let Some(ft) = static_face_textures(short) {
        return ft;
    }
    // Fallback: assume a uniform texture named after the block.
    FaceTextures::uniform(short)
}

/// Bed face textures: the mattress top/sides read as coloured wool (the
/// closest ordinary block texture to the actual dyed blanket colour), and
/// the underside as oak planks (frame colour). This is a deliberate
/// approximation — real beds sample a dedicated entity-style multi-region
/// atlas (`textures/entity/bed/<color>.png`, see PRD §4 item 3) that this
/// exporter's one-texture-per-face-per-block system doesn't support yet —
/// but it reads clearly as "a bed" and is a large step up from the previous
/// fallback (an unmatched "<color>_bed" name that resolved to a flat,
/// wrong-coloured procedural tile).
fn bed_face_textures(short: &str) -> Option<FaceTextures> {
    let color = short.strip_suffix("_bed")?;
    let wool = format!("{color}_wool");
    Some(FaceTextures {
        top: wool.clone(),
        bottom: "oak_planks".into(),
        south: wool.clone(),
        north: wool.clone(),
        east: wool.clone(),
        west: wool,
    })
}

/// Static lookup table for the most common vanilla blocks.
#[allow(clippy::too_many_lines)]
fn static_face_textures(short: &str) -> Option<FaceTextures> {
    let u = |n: &str| FaceTextures::uniform(n);
    let c = |top: &str, bot: &str, side: &str| FaceTextures::tri(top, bot, side);

    Some(match short {
        // ─── Dirt / Ground ───
        "grass_block" => c("grass_block_top", "dirt", "grass_block_side"),
        "dirt" => u("dirt"),
        "coarse_dirt" => u("coarse_dirt"),
        "rooted_dirt" => c("rooted_dirt", "rooted_dirt", "rooted_dirt"),
        "podzol" => c("podzol_top", "dirt", "podzol_side"),
        "mycelium" => c("mycelium_top", "dirt", "mycelium_side"),
        "mud" => u("mud"),
        "muddy_mangrove_roots" => c(
            "muddy_mangrove_roots_top",
            "muddy_mangrove_roots_top",
            "muddy_mangrove_roots_side",
        ),

        // ─── Stone / Rock ───
        "stone" => u("stone"),
        "cobblestone" => u("cobblestone"),
        "mossy_cobblestone" => u("mossy_cobblestone"),
        "granite" => u("granite"),
        "polished_granite" => u("polished_granite"),
        "diorite" => u("diorite"),
        "polished_diorite" => u("polished_diorite"),
        "andesite" => u("andesite"),
        "polished_andesite" => u("polished_andesite"),
        "deepslate" => c("deepslate_top", "deepslate_top", "deepslate"),
        "cobbled_deepslate" => u("cobbled_deepslate"),
        "polished_deepslate" => u("polished_deepslate"),
        "tuff" => u("tuff"),
        "calcite" => u("calcite"),
        "dripstone_block" => u("dripstone_block"),
        "bedrock" => u("bedrock"),
        "gravel" => u("gravel"),
        "sand" => u("sand"),
        "red_sand" => u("red_sand"),

        // ─── Sandstone ───
        "sandstone" => c("sandstone_top", "sandstone_bottom", "sandstone"),
        "chiseled_sandstone" => c("sandstone_top", "sandstone_bottom", "chiseled_sandstone"),
        "cut_sandstone" => c("sandstone_top", "sandstone_bottom", "cut_sandstone"),
        "smooth_sandstone" => u("sandstone_top"),
        "red_sandstone" => c("red_sandstone_top", "red_sandstone_bottom", "red_sandstone"),
        "chiseled_red_sandstone" => c(
            "red_sandstone_top",
            "red_sandstone_bottom",
            "chiseled_red_sandstone",
        ),
        "cut_red_sandstone" => c(
            "red_sandstone_top",
            "red_sandstone_bottom",
            "cut_red_sandstone",
        ),
        "smooth_red_sandstone" => u("red_sandstone_top"),

        // ─── Wood Logs ───
        "oak_log" | "stripped_oak_log" => {
            let (top, side) = if short.starts_with("stripped") {
                ("stripped_oak_log_top", "stripped_oak_log")
            } else {
                ("oak_log_top", "oak_log")
            };
            c(top, top, side)
        }
        "spruce_log" | "stripped_spruce_log" => {
            let (top, side) = if short.starts_with("stripped") {
                ("stripped_spruce_log_top", "stripped_spruce_log")
            } else {
                ("spruce_log_top", "spruce_log")
            };
            c(top, top, side)
        }
        "birch_log" | "stripped_birch_log" => {
            let (top, side) = if short.starts_with("stripped") {
                ("stripped_birch_log_top", "stripped_birch_log")
            } else {
                ("birch_log_top", "birch_log")
            };
            c(top, top, side)
        }
        "jungle_log" | "stripped_jungle_log" => {
            let (top, side) = if short.starts_with("stripped") {
                ("stripped_jungle_log_top", "stripped_jungle_log")
            } else {
                ("jungle_log_top", "jungle_log")
            };
            c(top, top, side)
        }
        "acacia_log" | "stripped_acacia_log" => {
            let (top, side) = if short.starts_with("stripped") {
                ("stripped_acacia_log_top", "stripped_acacia_log")
            } else {
                ("acacia_log_top", "acacia_log")
            };
            c(top, top, side)
        }
        "dark_oak_log" | "stripped_dark_oak_log" => {
            let (top, side) = if short.starts_with("stripped") {
                ("stripped_dark_oak_log_top", "stripped_dark_oak_log")
            } else {
                ("dark_oak_log_top", "dark_oak_log")
            };
            c(top, top, side)
        }
        "mangrove_log" | "stripped_mangrove_log" => {
            let (top, side) = if short.starts_with("stripped") {
                ("stripped_mangrove_log_top", "stripped_mangrove_log")
            } else {
                ("mangrove_log_top", "mangrove_log")
            };
            c(top, top, side)
        }
        "cherry_log" | "stripped_cherry_log" => {
            let (top, side) = if short.starts_with("stripped") {
                ("stripped_cherry_log_top", "stripped_cherry_log")
            } else {
                ("cherry_log_top", "cherry_log")
            };
            c(top, top, side)
        }

        // ─── Wood Planks ───
        "oak_planks" => u("oak_planks"),
        "spruce_planks" => u("spruce_planks"),
        "birch_planks" => u("birch_planks"),
        "jungle_planks" => u("jungle_planks"),
        "acacia_planks" => u("acacia_planks"),
        "dark_oak_planks" => u("dark_oak_planks"),
        "mangrove_planks" => u("mangrove_planks"),
        "cherry_planks" => u("cherry_planks"),
        "bamboo_planks" => u("bamboo_planks"),
        "crimson_planks" => u("crimson_planks"),
        "warped_planks" => u("warped_planks"),

        // ─── Leaves ───
        "oak_leaves" => u("oak_leaves"),
        "spruce_leaves" => u("spruce_leaves"),
        "birch_leaves" => u("birch_leaves"),
        "jungle_leaves" => u("jungle_leaves"),
        "acacia_leaves" => u("acacia_leaves"),
        "dark_oak_leaves" => u("dark_oak_leaves"),
        "mangrove_leaves" => u("mangrove_leaves"),
        "cherry_leaves" => u("cherry_leaves"),
        "azalea_leaves" => u("azalea_leaves"),
        "flowering_azalea_leaves" => u("flowering_azalea_leaves"),

        // ─── Glass ───
        "glass" => u("glass"),
        "tinted_glass" => u("tinted_glass"),

        // ─── Brick / Masonry ───
        "bricks" => u("bricks"),
        "stone_bricks" => u("stone_bricks"),
        "cracked_stone_bricks" => u("cracked_stone_bricks"),
        "mossy_stone_bricks" => u("mossy_stone_bricks"),
        "chiseled_stone_bricks" => u("chiseled_stone_bricks"),
        "nether_bricks" => u("nether_bricks"),
        "red_nether_bricks" => u("red_nether_bricks"),
        "chiseled_nether_bricks" => u("chiseled_nether_bricks"),
        "cracked_nether_bricks" => u("cracked_nether_bricks"),
        "end_stone_bricks" => u("end_stone_bricks"),
        "mud_bricks" => u("mud_bricks"),
        "deepslate_bricks" => u("deepslate_bricks"),
        "cracked_deepslate_bricks" => u("cracked_deepslate_bricks"),
        "deepslate_tiles" => u("deepslate_tiles"),
        "cracked_deepslate_tiles" => u("cracked_deepslate_tiles"),
        "chiseled_deepslate" => u("chiseled_deepslate"),

        // ─── Ore ───
        "coal_ore" => u("coal_ore"),
        "iron_ore" => u("iron_ore"),
        "gold_ore" => u("gold_ore"),
        "diamond_ore" => u("diamond_ore"),
        "redstone_ore" => u("redstone_ore"),
        "lapis_ore" => u("lapis_ore"),
        "emerald_ore" => u("emerald_ore"),
        "copper_ore" => u("copper_ore"),
        "nether_gold_ore" => u("nether_gold_ore"),
        "nether_quartz_ore" => u("nether_quartz_ore"),
        "deepslate_coal_ore" => u("deepslate_coal_ore"),
        "deepslate_iron_ore" => u("deepslate_iron_ore"),
        "deepslate_gold_ore" => u("deepslate_gold_ore"),
        "deepslate_diamond_ore" => u("deepslate_diamond_ore"),
        "deepslate_redstone_ore" => u("deepslate_redstone_ore"),
        "deepslate_lapis_ore" => u("deepslate_lapis_ore"),
        "deepslate_emerald_ore" => u("deepslate_emerald_ore"),
        "deepslate_copper_ore" => u("deepslate_copper_ore"),

        // ─── Metal Blocks ───
        "iron_block" => u("iron_block"),
        "gold_block" => u("gold_block"),
        "diamond_block" => u("diamond_block"),
        "emerald_block" => u("emerald_block"),
        "lapis_block" => u("lapis_block"),
        "redstone_block" => u("redstone_block"),
        "copper_block" => u("copper_block"),
        "exposed_copper" => u("exposed_copper"),
        "weathered_copper" => u("weathered_copper"),
        "oxidized_copper" => u("oxidized_copper"),
        "netherite_block" => u("netherite_block"),

        // ─── Nether / End ───
        "netherrack" => u("netherrack"),
        "soul_sand" => u("soul_sand"),
        "soul_soil" => u("soul_soil"),
        "basalt" => c("basalt_top", "basalt_top", "basalt_side"),
        "polished_basalt" => c(
            "polished_basalt_top",
            "polished_basalt_top",
            "polished_basalt_side",
        ),
        "blackstone" => c("blackstone_top", "blackstone_top", "blackstone"),
        "polished_blackstone" => u("polished_blackstone"),
        "obsidian" => u("obsidian"),
        "crying_obsidian" => u("crying_obsidian"),
        "glowstone" => u("glowstone"),
        "shroomlight" => u("shroomlight"),
        "magma_block" => u("magma"),
        "nether_wart_block" => u("nether_wart_block"),
        "warped_wart_block" => u("warped_wart_block"),
        "end_stone" => u("end_stone"),

        // ─── Water / Lava / Ice ───
        "water" => u("water_still"),
        "lava" => u("lava_still"),
        "ice" => u("ice"),
        "packed_ice" => u("packed_ice"),
        "blue_ice" => u("blue_ice"),
        "snow_block" => u("snow"),

        // ─── Clay / Terracotta / Concrete ───
        "clay" => u("clay"),
        "terracotta" => u("terracotta"),
        "white_terracotta" => u("white_terracotta"),
        "orange_terracotta" => u("orange_terracotta"),
        "magenta_terracotta" => u("magenta_terracotta"),
        "light_blue_terracotta" => u("light_blue_terracotta"),
        "yellow_terracotta" => u("yellow_terracotta"),
        "lime_terracotta" => u("lime_terracotta"),
        "pink_terracotta" => u("pink_terracotta"),
        "gray_terracotta" => u("gray_terracotta"),
        "light_gray_terracotta" => u("light_gray_terracotta"),
        "cyan_terracotta" => u("cyan_terracotta"),
        "purple_terracotta" => u("purple_terracotta"),
        "blue_terracotta" => u("blue_terracotta"),
        "brown_terracotta" => u("brown_terracotta"),
        "green_terracotta" => u("green_terracotta"),
        "red_terracotta" => u("red_terracotta"),
        "black_terracotta" => u("black_terracotta"),
        "white_concrete" => u("white_concrete"),
        "orange_concrete" => u("orange_concrete"),
        "magenta_concrete" => u("magenta_concrete"),
        "light_blue_concrete" => u("light_blue_concrete"),
        "yellow_concrete" => u("yellow_concrete"),
        "lime_concrete" => u("lime_concrete"),
        "pink_concrete" => u("pink_concrete"),
        "gray_concrete" => u("gray_concrete"),
        "light_gray_concrete" => u("light_gray_concrete"),
        "cyan_concrete" => u("cyan_concrete"),
        "purple_concrete" => u("purple_concrete"),
        "blue_concrete" => u("blue_concrete"),
        "brown_concrete" => u("brown_concrete"),
        "green_concrete" => u("green_concrete"),
        "red_concrete" => u("red_concrete"),
        "black_concrete" => u("black_concrete"),

        // ─── Wool ───
        "white_wool" => u("white_wool"),
        "orange_wool" => u("orange_wool"),
        "magenta_wool" => u("magenta_wool"),
        "light_blue_wool" => u("light_blue_wool"),
        "yellow_wool" => u("yellow_wool"),
        "lime_wool" => u("lime_wool"),
        "pink_wool" => u("pink_wool"),
        "gray_wool" => u("gray_wool"),
        "light_gray_wool" => u("light_gray_wool"),
        "cyan_wool" => u("cyan_wool"),
        "purple_wool" => u("purple_wool"),
        "blue_wool" => u("blue_wool"),
        "brown_wool" => u("brown_wool"),
        "green_wool" => u("green_wool"),
        "red_wool" => u("red_wool"),
        "black_wool" => u("black_wool"),

        // ─── Crafting / Utility ───
        "crafting_table" => FaceTextures {
            top: "crafting_table_top".into(),
            bottom: "oak_planks".into(),
            south: "crafting_table_front".into(),
            north: "crafting_table_side".into(),
            east: "crafting_table_side".into(),
            west: "crafting_table_front".into(),
        },
        "furnace" => FaceTextures {
            top: "furnace_top".into(),
            bottom: "furnace_top".into(),
            south: "furnace_front_off".into(),
            north: "furnace_side".into(),
            east: "furnace_side".into(),
            west: "furnace_side".into(),
        },
        "blast_furnace" => FaceTextures {
            top: "blast_furnace_top".into(),
            bottom: "blast_furnace_top".into(),
            south: "blast_furnace_front_off".into(),
            north: "blast_furnace_side".into(),
            east: "blast_furnace_side".into(),
            west: "blast_furnace_side".into(),
        },
        "smoker" => FaceTextures {
            top: "smoker_top".into(),
            bottom: "smoker_bottom".into(),
            south: "smoker_front_off".into(),
            north: "smoker_side".into(),
            east: "smoker_side".into(),
            west: "smoker_side".into(),
        },
        "tnt" => c("tnt_top", "tnt_bottom", "tnt_side"),
        "bookshelf" => c("oak_planks", "oak_planks", "bookshelf"),
        "chiseled_bookshelf" => c(
            "chiseled_bookshelf_top",
            "chiseled_bookshelf_top",
            "chiseled_bookshelf",
        ),
        "pumpkin" => c("pumpkin_top", "pumpkin_top", "pumpkin_side"),
        "carved_pumpkin" => c("pumpkin_top", "pumpkin_top", "carved_pumpkin"),
        "jack_o_lantern" => FaceTextures {
            top: "pumpkin_top".into(),
            bottom: "pumpkin_top".into(),
            south: "jack_o_lantern".into(),
            north: "pumpkin_side".into(),
            east: "pumpkin_side".into(),
            west: "pumpkin_side".into(),
        },
        "melon" => c("melon_top", "melon_top", "melon_side"),
        "hay_block" => c("hay_block_top", "hay_block_top", "hay_block_side"),
        "bone_block" => c("bone_block_top", "bone_block_top", "bone_block_side"),

        // ─── Misc ───
        "moss_block" => u("moss_block"),
        "sponge" => u("sponge"),
        "wet_sponge" => u("wet_sponge"),
        "prismarine" => u("prismarine"),
        "prismarine_bricks" => u("prismarine_bricks"),
        "dark_prismarine" => u("dark_prismarine"),
        "sea_lantern" => u("sea_lantern"),
        "purpur_pillar" => c(
            "purpur_pillar_top",
            "purpur_pillar_top",
            "purpur_pillar_side",
        ),

        // ─── Paths & Farmland ───
        "grass_path" | "dirt_path" => c("dirt_path_top", "dirt", "dirt_path_side"),
        "farmland" => c("farmland_moist", "dirt", "dirt"),

        // ─── Quartz ───
        "quartz_block" => c(
            "quartz_block_top",
            "quartz_block_bottom",
            "quartz_block_side",
        ),
        "chiseled_quartz_block" => c(
            "chiseled_quartz_block_top",
            "chiseled_quartz_block_top",
            "chiseled_quartz_block",
        ),
        "quartz_pillar" => c("quartz_pillar_top", "quartz_pillar_top", "quartz_pillar"),
        "smooth_quartz" => u("quartz_block_bottom"),
        "quartz_bricks" => u("quartz_bricks"),

        // ─── Stairs (all mapped to their base material, rendered as full cube) ───
        "oak_stairs" => u("oak_planks"),
        "spruce_stairs" => u("spruce_planks"),
        "birch_stairs" => u("birch_planks"),
        "jungle_stairs" => u("jungle_planks"),
        "acacia_stairs" => u("acacia_planks"),
        "dark_oak_stairs" => u("dark_oak_planks"),
        "mangrove_stairs" => u("mangrove_planks"),
        "cherry_stairs" => u("cherry_planks"),
        "bamboo_stairs" => u("bamboo_planks"),
        "crimson_stairs" => u("crimson_planks"),
        "warped_stairs" => u("warped_planks"),
        "stone_stairs" => u("stone"),
        "cobblestone_stairs" => u("cobblestone"),
        "mossy_cobblestone_stairs" => u("mossy_cobblestone"),
        "stone_brick_stairs" => u("stone_bricks"),
        "mossy_stone_brick_stairs" => u("mossy_stone_bricks"),
        "sandstone_stairs" => c("sandstone_top", "sandstone_bottom", "sandstone"),
        "red_sandstone_stairs" => c("red_sandstone_top", "red_sandstone_bottom", "red_sandstone"),
        "granite_stairs" => u("granite"),
        "polished_granite_stairs" => u("polished_granite"),
        "diorite_stairs" => u("diorite"),
        "polished_diorite_stairs" => u("polished_diorite"),
        "andesite_stairs" => u("andesite"),
        "polished_andesite_stairs" => u("polished_andesite"),
        "brick_stairs" => u("bricks"),
        "nether_brick_stairs" => u("nether_bricks"),
        "red_nether_brick_stairs" => u("red_nether_bricks"),
        "end_stone_brick_stairs" => u("end_stone_bricks"),
        "purpur_stairs" => u("purpur_block"),
        "quartz_stairs" => c(
            "quartz_block_top",
            "quartz_block_bottom",
            "quartz_block_side",
        ),
        "smooth_quartz_stairs" => u("quartz_block_bottom"),
        "prismarine_stairs" => u("prismarine"),
        "prismarine_brick_stairs" => u("prismarine_bricks"),
        "dark_prismarine_stairs" => u("dark_prismarine"),
        "blackstone_stairs" => c("blackstone_top", "blackstone_top", "blackstone"),
        "polished_blackstone_stairs" => u("polished_blackstone"),
        "polished_blackstone_brick_stairs" => u("polished_blackstone_bricks"),
        "cobbled_deepslate_stairs" => u("cobbled_deepslate"),
        "polished_deepslate_stairs" => u("polished_deepslate"),
        "deepslate_brick_stairs" => u("deepslate_bricks"),
        "deepslate_tile_stairs" => u("deepslate_tiles"),
        "tuff_stairs" => u("tuff"),
        "mud_brick_stairs" => u("mud_bricks"),

        // ─── Slabs ───
        "oak_slab" => u("oak_planks"),
        "spruce_slab" => u("spruce_planks"),
        "birch_slab" => u("birch_planks"),
        "jungle_slab" => u("jungle_planks"),
        "acacia_slab" => u("acacia_planks"),
        "dark_oak_slab" => u("dark_oak_planks"),
        "mangrove_slab" => u("mangrove_planks"),
        "cherry_slab" => u("cherry_planks"),
        "bamboo_slab" => u("bamboo_planks"),
        "crimson_slab" => u("crimson_planks"),
        "warped_slab" => u("warped_planks"),
        "stone_slab" => u("stone"),
        "smooth_stone_slab" => c("smooth_stone", "smooth_stone", "smooth_stone_slab_side"),
        "cobblestone_slab" => u("cobblestone"),
        "mossy_cobblestone_slab" => u("mossy_cobblestone"),
        "stone_brick_slab" => u("stone_bricks"),
        "mossy_stone_brick_slab" => u("mossy_stone_bricks"),
        "sandstone_slab" => c("sandstone_top", "sandstone_bottom", "sandstone"),
        "cut_sandstone_slab" => c("sandstone_top", "sandstone_bottom", "cut_sandstone"),
        "red_sandstone_slab" => c("red_sandstone_top", "red_sandstone_bottom", "red_sandstone"),
        "cut_red_sandstone_slab" => c(
            "red_sandstone_top",
            "red_sandstone_bottom",
            "cut_red_sandstone",
        ),
        "granite_slab" => u("granite"),
        "polished_granite_slab" => u("polished_granite"),
        "diorite_slab" => u("diorite"),
        "polished_diorite_slab" => u("polished_diorite"),
        "andesite_slab" => u("andesite"),
        "polished_andesite_slab" => u("polished_andesite"),
        "brick_slab" => u("bricks"),
        "nether_brick_slab" => u("nether_bricks"),
        "red_nether_brick_slab" => u("red_nether_bricks"),
        "end_stone_brick_slab" => u("end_stone_bricks"),
        "purpur_slab" => u("purpur_block"),
        "quartz_slab" => c(
            "quartz_block_top",
            "quartz_block_bottom",
            "quartz_block_side",
        ),
        "smooth_quartz_slab" => u("quartz_block_bottom"),
        "prismarine_slab" => u("prismarine"),
        "prismarine_brick_slab" => u("prismarine_bricks"),
        "dark_prismarine_slab" => u("dark_prismarine"),
        "blackstone_slab" => c("blackstone_top", "blackstone_top", "blackstone"),
        "polished_blackstone_slab" => u("polished_blackstone"),
        "polished_blackstone_brick_slab" => u("polished_blackstone_bricks"),
        "cobbled_deepslate_slab" => u("cobbled_deepslate"),
        "polished_deepslate_slab" => u("polished_deepslate"),
        "deepslate_brick_slab" => u("deepslate_bricks"),
        "deepslate_tile_slab" => u("deepslate_tiles"),
        "tuff_slab" => u("tuff"),
        "mud_brick_slab" => u("mud_bricks"),
        "petrified_oak_slab" => u("oak_planks"),

        // ─── Fences ───
        "oak_fence" => u("oak_planks"),
        "spruce_fence" => u("spruce_planks"),
        "birch_fence" => u("birch_planks"),
        "jungle_fence" => u("jungle_planks"),
        "acacia_fence" => u("acacia_planks"),
        "dark_oak_fence" => u("dark_oak_planks"),
        "mangrove_fence" => u("mangrove_planks"),
        "cherry_fence" => u("cherry_planks"),
        "bamboo_fence" => u("bamboo_planks"),
        "crimson_fence" => u("crimson_planks"),
        "warped_fence" => u("warped_planks"),
        "nether_brick_fence" => u("nether_bricks"),

        // ─── Fence Gates ───
        "oak_fence_gate" => u("oak_planks"),
        "spruce_fence_gate" => u("spruce_planks"),
        "birch_fence_gate" => u("birch_planks"),
        "jungle_fence_gate" => u("jungle_planks"),
        "acacia_fence_gate" => u("acacia_planks"),
        "dark_oak_fence_gate" => u("dark_oak_planks"),
        "mangrove_fence_gate" => u("mangrove_planks"),
        "cherry_fence_gate" => u("cherry_planks"),
        "bamboo_fence_gate" => u("bamboo_planks"),
        "crimson_fence_gate" => u("crimson_planks"),
        "warped_fence_gate" => u("warped_planks"),

        // ─── Walls ───
        "cobblestone_wall" => u("cobblestone"),
        "mossy_cobblestone_wall" => u("mossy_cobblestone"),
        "stone_brick_wall" => u("stone_bricks"),
        "mossy_stone_brick_wall" => u("mossy_stone_bricks"),
        "andesite_wall" => u("andesite"),
        "diorite_wall" => u("diorite"),
        "granite_wall" => u("granite"),
        "brick_wall" => u("bricks"),
        "nether_brick_wall" => u("nether_bricks"),
        "red_nether_brick_wall" => u("red_nether_bricks"),
        "sandstone_wall" => c("sandstone_top", "sandstone_bottom", "sandstone"),
        "red_sandstone_wall" => c("red_sandstone_top", "red_sandstone_bottom", "red_sandstone"),
        "end_stone_brick_wall" => u("end_stone_bricks"),
        "prismarine_wall" => u("prismarine"),
        "blackstone_wall" => c("blackstone_top", "blackstone_top", "blackstone"),
        "polished_blackstone_wall" => u("polished_blackstone"),
        "polished_blackstone_brick_wall" => u("polished_blackstone_bricks"),
        "cobbled_deepslate_wall" => u("cobbled_deepslate"),
        "polished_deepslate_wall" => u("polished_deepslate"),
        "deepslate_brick_wall" => u("deepslate_bricks"),
        "deepslate_tile_wall" => u("deepslate_tiles"),
        "tuff_wall" => u("tuff"),
        "mud_brick_wall" => u("mud_bricks"),

        // ─── Doors ───
        "oak_door" => c("oak_door_top", "oak_door_bottom", "oak_door_bottom"),
        "spruce_door" => c(
            "spruce_door_top",
            "spruce_door_bottom",
            "spruce_door_bottom",
        ),
        "birch_door" => c("birch_door_top", "birch_door_bottom", "birch_door_bottom"),
        "jungle_door" => c(
            "jungle_door_top",
            "jungle_door_bottom",
            "jungle_door_bottom",
        ),
        "acacia_door" => c(
            "acacia_door_top",
            "acacia_door_bottom",
            "acacia_door_bottom",
        ),
        "dark_oak_door" => c(
            "dark_oak_door_top",
            "dark_oak_door_bottom",
            "dark_oak_door_bottom",
        ),
        "mangrove_door" => c(
            "mangrove_door_top",
            "mangrove_door_bottom",
            "mangrove_door_bottom",
        ),
        "cherry_door" => c(
            "cherry_door_top",
            "cherry_door_bottom",
            "cherry_door_bottom",
        ),
        "crimson_door" => c(
            "crimson_door_top",
            "crimson_door_bottom",
            "crimson_door_bottom",
        ),
        "warped_door" => c(
            "warped_door_top",
            "warped_door_bottom",
            "warped_door_bottom",
        ),
        "iron_door" => c("iron_door_top", "iron_door_bottom", "iron_door_bottom"),
        "copper_door" => c(
            "copper_door_top",
            "copper_door_bottom",
            "copper_door_bottom",
        ),

        // ─── Trapdoors ───
        "oak_trapdoor" => u("oak_trapdoor"),
        "spruce_trapdoor" => u("spruce_trapdoor"),
        "birch_trapdoor" => u("birch_trapdoor"),
        "jungle_trapdoor" => u("jungle_trapdoor"),
        "acacia_trapdoor" => u("acacia_trapdoor"),
        "dark_oak_trapdoor" => u("dark_oak_trapdoor"),
        "mangrove_trapdoor" => u("mangrove_trapdoor"),
        "cherry_trapdoor" => u("cherry_trapdoor"),
        "crimson_trapdoor" => u("crimson_trapdoor"),
        "warped_trapdoor" => u("warped_trapdoor"),
        "iron_trapdoor" => u("iron_trapdoor"),
        "copper_trapdoor" => u("copper_trapdoor"),

        // ─── Flowers & Small Plants ───
        "dandelion" => u("dandelion"),
        "poppy" => u("poppy"),
        "blue_orchid" => u("blue_orchid"),
        "allium" => u("allium"),
        "azure_bluet" => u("azure_bluet"),
        "red_tulip" => u("red_tulip"),
        "orange_tulip" => u("orange_tulip"),
        "white_tulip" => u("white_tulip"),
        "pink_tulip" => u("pink_tulip"),
        "oxeye_daisy" => u("oxeye_daisy"),
        "cornflower" => u("cornflower"),
        "lily_of_the_valley" => u("lily_of_the_valley"),
        "wither_rose" => u("wither_rose"),
        "sunflower" => u("sunflower_front"),
        "lilac" => u("lilac_top"),
        "peony" => u("peony_top"),
        "rose_bush" => u("rose_bush_top"),
        "spore_blossom" => u("spore_blossom"),
        "pink_petals" => u("pink_petals"),
        "torchflower" => u("torchflower"),
        "pitcher_plant" => u("pitcher_plant_top"),
        "brown_mushroom" => u("brown_mushroom"),
        "red_mushroom" => u("red_mushroom"),
        "crimson_fungus" => u("crimson_fungus"),
        "warped_fungus" => u("warped_fungus"),

        // ─── Mushroom Blocks ───
        "brown_mushroom_block" => u("mushroom_block_skin_brown"),
        "red_mushroom_block" => u("mushroom_block_skin_red"),
        "mushroom_stem" => c(
            "mushroom_block_skin_stem",
            "mushroom_block_skin_stem",
            "mushroom_block_inside",
        ),

        // ─── Tall Plants / Grasses (biome tint applied in texture.rs) ───
        "short_grass" | "grass" => u("short_grass"),
        "tall_grass" => u("tall_grass_top"),
        "fern" => u("fern"),
        "large_fern" => u("large_fern_top"),
        "dead_bush" => u("dead_bush"),
        "seagrass" => u("seagrass"),
        "tall_seagrass" => u("tall_seagrass_top"),
        "kelp" => u("kelp"),
        "kelp_plant" => u("kelp_plant"),
        "hanging_roots" => u("hanging_roots"),
        "azalea" => c("azalea_top", "azalea_top", "azalea_side"),
        "flowering_azalea" => c(
            "flowering_azalea_top",
            "flowering_azalea_top",
            "flowering_azalea_side",
        ),
        "moss_carpet" => u("moss_block"),
        "crimson_roots" => u("crimson_roots"),
        "warped_roots" => u("warped_roots"),
        "nether_sprouts" => u("nether_sprouts"),

        // ─── Crops ───
        "wheat" => u("wheat_stage7"),
        "carrots" => u("carrots_stage3"),
        "potatoes" => u("potatoes_stage3"),
        "beetroots" => u("beetroots_stage3"),
        "pumpkin_stem" => u("pumpkin_stem"),
        "melon_stem" => u("melon_stem"),
        "nether_wart" => u("nether_wart_stage2"),
        "cocoa" => u("cocoa_stage2"),
        "sweet_berry_bush" => u("sweet_berry_bush_stage3"),
        "torchflower_crop" => u("torchflower_crop_stage1"),

        // ─── Cactus, Sugar Cane, Vines, etc. ───
        "cactus" => c("cactus_top", "cactus_bottom", "cactus_side"),
        "lily_pad" => u("lily_pad"),
        "vine" => u("vine"),
        "cobweb" => u("cobweb"),
        "ladder" => u("ladder"),
        "bamboo" => u("bamboo_stalk"),

        // ─── Torches & Lights ───
        "torch" | "wall_torch" => u("torch"),
        "soul_torch" | "soul_wall_torch" => u("soul_torch"),
        "redstone_torch" | "redstone_wall_torch" => u("redstone_torch"),
        "lantern" => u("lantern"),
        "soul_lantern" => u("soul_lantern"),
        "sea_pickle" => u("sea_pickle"),
        "end_rod" => u("end_rod"),
        "conduit" => u("conduit"),
        "beacon" => u("beacon"),
        "redstone_lamp" => u("redstone_lamp"),

        // ─── Chests & Storage ───
        // Chest model textures live in entity/, not block/; plank fallback keeps
        // the color recognisable without a texture load failure.
        "chest" | "trapped_chest" => c("oak_planks", "oak_planks", "oak_planks"),
        "ender_chest" => u("obsidian"),
        "barrel" => c("barrel_top", "barrel_bottom", "barrel_side"),
        "shulker_box" => u("shulker_top"),
        "white_shulker_box"
        | "orange_shulker_box"
        | "magenta_shulker_box"
        | "light_blue_shulker_box"
        | "yellow_shulker_box"
        | "lime_shulker_box"
        | "pink_shulker_box"
        | "gray_shulker_box"
        | "light_gray_shulker_box"
        | "cyan_shulker_box"
        | "purple_shulker_box"
        | "blue_shulker_box"
        | "brown_shulker_box"
        | "green_shulker_box"
        | "red_shulker_box"
        | "black_shulker_box" => u("shulker_top"),

        // ─── Village Workstations ───
        "cartography_table" => FaceTextures {
            top: "cartography_table_top".into(),
            bottom: "oak_planks".into(),
            south: "cartography_table_side1".into(),
            north: "cartography_table_side2".into(),
            east: "cartography_table_side3".into(),
            west: "cartography_table_side3".into(),
        },
        "fletching_table" => FaceTextures {
            top: "fletching_table_top".into(),
            bottom: "birch_planks".into(),
            south: "fletching_table_front".into(),
            north: "fletching_table_side".into(),
            east: "fletching_table_side".into(),
            west: "fletching_table_front".into(),
        },
        "loom" => FaceTextures {
            top: "loom_top".into(),
            bottom: "oak_planks".into(),
            south: "loom_front".into(),
            north: "loom_side".into(),
            east: "loom_side".into(),
            west: "loom_side".into(),
        },
        "smithing_table" => FaceTextures {
            top: "smithing_table_top".into(),
            bottom: "oak_planks".into(),
            south: "smithing_table_front".into(),
            north: "smithing_table_side".into(),
            east: "smithing_table_side".into(),
            west: "smithing_table_front".into(),
        },
        "lectern" => c("lectern_top", "oak_planks", "lectern_front"),
        "composter" => c("composter_top", "composter_bottom", "composter_side"),
        "grindstone" => u("grindstone_side"),
        "stonecutter" => c("stonecutter_top", "stone", "stonecutter_side"),
        "bell" => u("bell_body"),
        "beehive" => c("beehive_end", "beehive_end", "beehive_side"),
        "bee_nest" => c("bee_nest_end", "bee_nest_end", "bee_nest_side"),
        "cauldron" => c("cauldron_top", "cauldron_bottom", "cauldron_side"),
        "lava_cauldron" => c("cauldron_top", "cauldron_bottom", "cauldron_side"),
        "water_cauldron" => c("cauldron_top", "cauldron_bottom", "cauldron_side"),
        "powder_snow_cauldron" => c("cauldron_top", "cauldron_bottom", "cauldron_side"),
        "jukebox" => c("jukebox_top", "oak_planks", "jukebox_side"),
        "note_block" => u("note_block"),
        "enchanting_table" => c(
            "enchanting_table_top",
            "enchanting_table_bottom",
            "enchanting_table_side",
        ),
        "brewing_stand" => c("brewing_stand_base", "brewing_stand_base", "brewing_stand"),
        "campfire" => c("campfire_log_lit", "campfire_log_lit", "campfire_log_lit"),
        "soul_campfire" => c(
            "soul_campfire_log_lit",
            "soul_campfire_log_lit",
            "soul_campfire_log_lit",
        ),
        "anvil" => c("anvil_top", "anvil_base", "anvil_base"),
        "chipped_anvil" => c("chipped_anvil_top", "anvil_base", "anvil_base"),
        "damaged_anvil" => c("damaged_anvil_top", "anvil_base", "anvil_base"),
        "flower_pot" => u("flower_pot"),

        // ─── Redstone & Mechanisms ───
        "observer" => c("observer_top", "observer_back", "observer_side"),
        "piston" => c("piston_top_normal", "piston_bottom", "piston_side"),
        "sticky_piston" => c("piston_top_sticky", "piston_bottom", "piston_side"),
        "piston_head" => u("piston_top_normal"),
        "dispenser" => c("furnace_top", "furnace_top", "dispenser_front_horizontal"),
        "dropper" => c("furnace_top", "furnace_top", "dropper_front_horizontal"),
        "hopper" => c("hopper_top", "hopper_outside", "hopper_outside"),
        "daylight_detector" => c(
            "daylight_detector_top",
            "oak_planks",
            "daylight_detector_side",
        ),
        "target" => c("target_top", "target_top", "target_side"),
        "slime_block" => u("slime_block"),
        "honey_block" => c("honey_block_top", "honey_block_bottom", "honey_block_side"),
        "honeycomb_block" => u("honeycomb_block"),

        // ─── Pressure Plates & Buttons (rendered as full cube with base texture) ───
        "stone_pressure_plate" | "stone_button" => u("stone"),
        "oak_pressure_plate" | "oak_button" => u("oak_planks"),
        "spruce_pressure_plate" | "spruce_button" => u("spruce_planks"),
        "birch_pressure_plate" | "birch_button" => u("birch_planks"),
        "jungle_pressure_plate" | "jungle_button" => u("jungle_planks"),
        "acacia_pressure_plate" | "acacia_button" => u("acacia_planks"),
        "dark_oak_pressure_plate" | "dark_oak_button" => u("dark_oak_planks"),
        "mangrove_pressure_plate" | "mangrove_button" => u("mangrove_planks"),
        "cherry_pressure_plate" | "cherry_button" => u("cherry_planks"),
        "crimson_pressure_plate" | "crimson_button" => u("crimson_planks"),
        "warped_pressure_plate" | "warped_button" => u("warped_planks"),
        "polished_blackstone_pressure_plate" | "polished_blackstone_button" => {
            u("polished_blackstone")
        }
        "heavy_weighted_pressure_plate" => u("iron_block"),
        "light_weighted_pressure_plate" => u("gold_block"),

        // ─── Rails ───
        "rail" => u("rail"),
        "powered_rail" => u("powered_rail"),
        "detector_rail" => u("detector_rail"),
        "activator_rail" => u("activator_rail"),

        // ─── Bamboo Block ───
        "bamboo_block" => c("bamboo_block_top", "bamboo_block_top", "bamboo_block"),
        "stripped_bamboo_block" => c(
            "stripped_bamboo_block_top",
            "stripped_bamboo_block_top",
            "stripped_bamboo_block",
        ),

        // ─── Wood (bark-all-sides variant) ───
        "oak_wood" | "stripped_oak_wood" => u("oak_log"),
        "spruce_wood" | "stripped_spruce_wood" => u("spruce_log"),
        "birch_wood" | "stripped_birch_wood" => u("birch_log"),
        "jungle_wood" | "stripped_jungle_wood" => u("jungle_log"),
        "acacia_wood" | "stripped_acacia_wood" => u("acacia_log"),
        "dark_oak_wood" | "stripped_dark_oak_wood" => u("dark_oak_log"),
        "mangrove_wood" | "stripped_mangrove_wood" => u("mangrove_log"),
        "cherry_wood" | "stripped_cherry_wood" => u("cherry_log"),
        "crimson_hyphae" | "stripped_crimson_hyphae" => u("crimson_log"),
        "warped_hyphae" | "stripped_warped_hyphae" => u("warped_log"),

        // ─── Nether Blocks ───
        "warped_nylium" => c("warped_nylium", "netherrack", "warped_nylium_side"),
        "crimson_nylium" => c("crimson_nylium", "netherrack", "crimson_nylium_side"),
        "ancient_debris" => c(
            "ancient_debris_top",
            "ancient_debris_top",
            "ancient_debris_side",
        ),
        "respawn_anchor" => c(
            "respawn_anchor_top",
            "respawn_anchor_bottom",
            "respawn_anchor_side0",
        ),
        "lodestone" => c("lodestone_top", "lodestone_top", "lodestone_side"),
        "gilded_blackstone" => u("gilded_blackstone"),
        "chiseled_polished_blackstone" => u("chiseled_polished_blackstone"),
        "polished_blackstone_bricks" => u("polished_blackstone_bricks"),
        "cracked_polished_blackstone_bricks" => u("cracked_polished_blackstone_bricks"),

        // ─── End Blocks ───
        "end_portal_frame" => c("end_portal_frame_top", "end_stone", "end_portal_frame_side"),
        "chorus_flower" => u("chorus_flower"),
        "chorus_plant" => u("chorus_plant"),

        // ─── Concrete Powder ───
        "white_concrete_powder" => u("white_concrete_powder"),
        "orange_concrete_powder" => u("orange_concrete_powder"),
        "magenta_concrete_powder" => u("magenta_concrete_powder"),
        "light_blue_concrete_powder" => u("light_blue_concrete_powder"),
        "yellow_concrete_powder" => u("yellow_concrete_powder"),
        "lime_concrete_powder" => u("lime_concrete_powder"),
        "pink_concrete_powder" => u("pink_concrete_powder"),
        "gray_concrete_powder" => u("gray_concrete_powder"),
        "light_gray_concrete_powder" => u("light_gray_concrete_powder"),
        "cyan_concrete_powder" => u("cyan_concrete_powder"),
        "purple_concrete_powder" => u("purple_concrete_powder"),
        "blue_concrete_powder" => u("blue_concrete_powder"),
        "brown_concrete_powder" => u("brown_concrete_powder"),
        "green_concrete_powder" => u("green_concrete_powder"),
        "red_concrete_powder" => u("red_concrete_powder"),
        "black_concrete_powder" => u("black_concrete_powder"),

        // ─── Glazed Terracotta ───
        "white_glazed_terracotta" => u("white_glazed_terracotta"),
        "orange_glazed_terracotta" => u("orange_glazed_terracotta"),
        "magenta_glazed_terracotta" => u("magenta_glazed_terracotta"),
        "light_blue_glazed_terracotta" => u("light_blue_glazed_terracotta"),
        "yellow_glazed_terracotta" => u("yellow_glazed_terracotta"),
        "lime_glazed_terracotta" => u("lime_glazed_terracotta"),
        "pink_glazed_terracotta" => u("pink_glazed_terracotta"),
        "gray_glazed_terracotta" => u("gray_glazed_terracotta"),
        "light_gray_glazed_terracotta" => u("light_gray_glazed_terracotta"),
        "cyan_glazed_terracotta" => u("cyan_glazed_terracotta"),
        "purple_glazed_terracotta" => u("purple_glazed_terracotta"),
        "blue_glazed_terracotta" => u("blue_glazed_terracotta"),
        "brown_glazed_terracotta" => u("brown_glazed_terracotta"),
        "green_glazed_terracotta" => u("green_glazed_terracotta"),
        "red_glazed_terracotta" => u("red_glazed_terracotta"),
        "black_glazed_terracotta" => u("black_glazed_terracotta"),

        // ─── Sculk (1.19+) ───
        "sculk" => u("sculk"),
        "sculk_catalyst" => c(
            "sculk_catalyst_top",
            "sculk_catalyst_bottom",
            "sculk_catalyst_side",
        ),
        "sculk_shrieker" => c(
            "sculk_shrieker_top",
            "sculk_shrieker_bottom",
            "sculk_shrieker_side",
        ),
        "sculk_sensor" => c(
            "sculk_sensor_top",
            "sculk_sensor_bottom",
            "sculk_sensor_side",
        ),
        "sculk_vein" => u("sculk_vein"),

        // ─── Mangrove Swamp ───
        "mangrove_roots" => c(
            "mangrove_roots_top",
            "mangrove_roots_top",
            "mangrove_roots_side",
        ),

        // ─── Glass Panes & Bars ───
        "glass_pane" => u("glass"),
        "iron_bars" => u("iron_bars"),
        "white_stained_glass_pane" => u("white_stained_glass"),
        "orange_stained_glass_pane" => u("orange_stained_glass"),
        "magenta_stained_glass_pane" => u("magenta_stained_glass"),
        "light_blue_stained_glass_pane" => u("light_blue_stained_glass"),
        "yellow_stained_glass_pane" => u("yellow_stained_glass"),
        "lime_stained_glass_pane" => u("lime_stained_glass"),
        "pink_stained_glass_pane" => u("pink_stained_glass"),
        "gray_stained_glass_pane" => u("gray_stained_glass"),
        "light_gray_stained_glass_pane" => u("light_gray_stained_glass"),
        "cyan_stained_glass_pane" => u("cyan_stained_glass"),
        "purple_stained_glass_pane" => u("purple_stained_glass"),
        "blue_stained_glass_pane" => u("blue_stained_glass"),
        "brown_stained_glass_pane" => u("brown_stained_glass"),
        "green_stained_glass_pane" => u("green_stained_glass"),
        "red_stained_glass_pane" => u("red_stained_glass"),
        "black_stained_glass_pane" => u("black_stained_glass"),

        // ─── Stained Glass ───
        "white_stained_glass" => u("white_stained_glass"),
        "orange_stained_glass" => u("orange_stained_glass"),
        "magenta_stained_glass" => u("magenta_stained_glass"),
        "light_blue_stained_glass" => u("light_blue_stained_glass"),
        "yellow_stained_glass" => u("yellow_stained_glass"),
        "lime_stained_glass" => u("lime_stained_glass"),
        "pink_stained_glass" => u("pink_stained_glass"),
        "gray_stained_glass" => u("gray_stained_glass"),
        "light_gray_stained_glass" => u("light_gray_stained_glass"),
        "cyan_stained_glass" => u("cyan_stained_glass"),
        "purple_stained_glass" => u("purple_stained_glass"),
        "blue_stained_glass" => u("blue_stained_glass"),
        "brown_stained_glass" => u("brown_stained_glass"),
        "green_stained_glass" => u("green_stained_glass"),
        "red_stained_glass" => u("red_stained_glass"),
        "black_stained_glass" => u("black_stained_glass"),

        // ─── Signs & Hanging Signs (show as plank texture) ───
        "oak_sign" | "oak_wall_sign" | "oak_hanging_sign" | "oak_wall_hanging_sign" => {
            u("oak_planks")
        }
        "spruce_sign" | "spruce_wall_sign" | "spruce_hanging_sign" | "spruce_wall_hanging_sign" => {
            u("spruce_planks")
        }
        "birch_sign" | "birch_wall_sign" | "birch_hanging_sign" | "birch_wall_hanging_sign" => {
            u("birch_planks")
        }
        "jungle_sign" | "jungle_wall_sign" | "jungle_hanging_sign" | "jungle_wall_hanging_sign" => {
            u("jungle_planks")
        }
        "acacia_sign" | "acacia_wall_sign" | "acacia_hanging_sign" | "acacia_wall_hanging_sign" => {
            u("acacia_planks")
        }
        "dark_oak_sign"
        | "dark_oak_wall_sign"
        | "dark_oak_hanging_sign"
        | "dark_oak_wall_hanging_sign" => u("dark_oak_planks"),
        "mangrove_sign"
        | "mangrove_wall_sign"
        | "mangrove_hanging_sign"
        | "mangrove_wall_hanging_sign" => u("mangrove_planks"),
        "cherry_sign" | "cherry_wall_sign" | "cherry_hanging_sign" | "cherry_wall_hanging_sign" => {
            u("cherry_planks")
        }
        "bamboo_sign" | "bamboo_wall_sign" | "bamboo_hanging_sign" | "bamboo_wall_hanging_sign" => {
            u("bamboo_planks")
        }
        "crimson_sign"
        | "crimson_wall_sign"
        | "crimson_hanging_sign"
        | "crimson_wall_hanging_sign" => u("crimson_planks"),
        "warped_sign" | "warped_wall_sign" | "warped_hanging_sign" | "warped_wall_hanging_sign" => {
            u("warped_planks")
        }

        // ─── Misc Utility ───
        "dried_kelp_block" => c("dried_kelp_top", "dried_kelp_bottom", "dried_kelp_side"),
        "powder_snow" => u("powder_snow"),

        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grass_block_has_distinct_faces() {
        let ft = face_textures("minecraft:grass_block");
        assert_eq!(ft.top, "grass_block_top");
        assert_eq!(ft.bottom, "dirt");
        assert_eq!(ft.south, "grass_block_side");
    }

    #[test]
    fn oak_log_has_distinct_top() {
        let ft = face_textures("minecraft:oak_log");
        assert_eq!(ft.top, "oak_log_top");
        assert_eq!(ft.south, "oak_log");
    }

    #[test]
    fn unknown_block_falls_back() {
        let ft = face_textures("minecraft:mystery_block_xyz");
        assert_eq!(ft.top, "mystery_block_xyz");
        assert_eq!(ft.south, "mystery_block_xyz");
    }

    #[test]
    fn stone_is_uniform() {
        let ft = face_textures("stone");
        assert_eq!(ft.top, "stone");
        assert_eq!(ft.bottom, "stone");
        assert_eq!(ft.east, "stone");
    }

    #[test]
    fn face_index_mapping() {
        let ft = face_textures("minecraft:grass_block");
        assert_eq!(ft.for_face_index(0), "grass_block_top"); // +Y
        assert_eq!(ft.for_face_index(1), "dirt"); // -Y
        assert_eq!(ft.for_face_index(2), "grass_block_side"); // +X (east)
    }
}
