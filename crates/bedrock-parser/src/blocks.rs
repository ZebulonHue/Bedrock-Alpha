//! Block presentation knowledge shared by the renderer and the exporter:
//! which blocks are air, and a flat display color per block id.
//!
//! Colors are rough averages of the vanilla textures; unknown (modded)
//! blocks get a stable, muted color derived from their name. The real
//! texture atlas replaces this in a later milestone.

use crate::chunk::strip_namespace;

/// True for blocks that occupy no geometry at all.
pub fn is_air(name: &str) -> bool {
    matches!(strip_namespace(name), "air" | "cave_air" | "void_air")
}

/// Flat display color for a block id (namespace optional), in 0..1 RGB.
pub fn block_color(name: &str) -> [f32; 3] {
    let short = strip_namespace(name);
    if let Some(color) = known_color(short) {
        return color;
    }
    // Modded or unmapped block: deterministic muted fallback.
    let mut hash: u32 = 0x811c_9dc5;
    for byte in short.bytes() {
        hash = (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193);
    }
    let hue = (hash % 360) as f32 / 360.0;
    hsv(hue, 0.35, 0.6)
}

/// Colors for the most common vanilla blocks (rough averages of their
/// textures, in 0..1 RGB).
fn known_color(short: &str) -> Option<[f32; 3]> {
    let rgb = |r: u8, g: u8, b: u8| {
        Some([
            f32::from(r) / 255.0,
            f32::from(g) / 255.0,
            f32::from(b) / 255.0,
        ])
    };
    match short {
        "stone" => rgb(125, 125, 125),
        "granite" => rgb(149, 103, 86),
        "diorite" => rgb(188, 188, 190),
        "andesite" => rgb(136, 136, 138),
        "deepslate" => rgb(80, 82, 87),
        "cobblestone" | "mossy_cobblestone" => rgb(110, 110, 110),
        "bedrock" => rgb(56, 56, 56),
        "dirt" | "coarse_dirt" | "rooted_dirt" => rgb(134, 96, 67),
        "grass_block" => rgb(112, 168, 70),
        "podzol" => rgb(120, 85, 55),
        "mycelium" => rgb(111, 99, 105),
        "sand" => rgb(219, 207, 163),
        "red_sand" => rgb(190, 102, 33),
        "gravel" => rgb(131, 127, 126),
        "clay" => rgb(159, 164, 177),
        "mud" => rgb(60, 57, 60),
        "snow_block" | "snow" => rgb(240, 251, 251),
        "ice" | "packed_ice" | "blue_ice" | "frosted_ice" => rgb(145, 183, 253),
        "water" => rgb(52, 95, 218),
        "lava" => rgb(212, 90, 18),
        "sandstone" | "cut_sandstone" | "chiseled_sandstone" | "smooth_sandstone" => {
            rgb(216, 203, 155)
        }
        "red_sandstone" | "cut_red_sandstone" | "smooth_red_sandstone" => rgb(186, 99, 29),
        "netherrack" => rgb(111, 54, 52),
        "soul_sand" | "soul_soil" => rgb(81, 62, 51),
        "basalt" | "polished_basalt" => rgb(73, 72, 78),
        "blackstone" => rgb(42, 36, 41),
        "obsidian" | "crying_obsidian" => rgb(15, 11, 25),
        "end_stone" => rgb(219, 223, 158),
        "end_stone_bricks" => rgb(226, 231, 171),
        "oak_log" | "spruce_log" | "birch_log" | "jungle_log" | "acacia_log" | "dark_oak_log"
        | "mangrove_log" | "cherry_log" => rgb(106, 85, 52),
        "oak_planks" | "spruce_planks" | "birch_planks" | "jungle_planks" | "acacia_planks"
        | "dark_oak_planks" | "mangrove_planks" | "cherry_planks" => rgb(162, 130, 78),
        "oak_leaves"
        | "spruce_leaves"
        | "birch_leaves"
        | "jungle_leaves"
        | "acacia_leaves"
        | "dark_oak_leaves"
        | "mangrove_leaves"
        | "azalea_leaves"
        | "flowering_azalea_leaves" => rgb(60, 120, 35),
        "cherry_leaves" => rgb(228, 165, 197),
        "glass" | "tinted_glass" | "glass_pane" => rgb(190, 220, 225),
        "bricks" => rgb(150, 97, 83),
        "stone_bricks"
        | "cracked_stone_bricks"
        | "mossy_stone_bricks"
        | "chiseled_stone_bricks" => rgb(122, 122, 122),
        "coal_ore" | "deepslate_coal_ore" => rgb(104, 104, 104),
        "iron_ore" | "deepslate_iron_ore" => rgb(148, 125, 110),
        "gold_ore" | "deepslate_gold_ore" | "nether_gold_ore" => rgb(165, 140, 70),
        "diamond_ore" | "deepslate_diamond_ore" => rgb(115, 145, 150),
        "redstone_ore" | "deepslate_redstone_ore" => rgb(140, 90, 90),
        "lapis_ore" | "deepslate_lapis_ore" => rgb(100, 110, 150),
        "emerald_ore" | "deepslate_emerald_ore" => rgb(100, 150, 110),
        "copper_ore" | "deepslate_copper_ore" => rgb(140, 110, 90),
        "quartz_ore" | "nether_quartz_ore" => rgb(130, 80, 75),
        "glowstone" => rgb(247, 205, 110),
        "sea_lantern" => rgb(178, 210, 200),
        "terracotta" => rgb(150, 92, 66),
        "white_terracotta" => rgb(209, 178, 161),
        "white_wool" => rgb(233, 236, 236),
        "black_wool" | "black_concrete" => rgb(21, 21, 26),
        "black_terracotta" => rgb(37, 23, 16),
        "prismarine" | "prismarine_bricks" | "dark_prismarine" => rgb(99, 156, 151),
        "tuff" => rgb(108, 109, 102),
        "calcite" => rgb(223, 224, 220),
        "dripstone_block" | "pointed_dripstone" => rgb(134, 107, 92),
        "moss_block" => rgb(90, 110, 45),
        _ => None,
    }
}

/// HSV → RGB, all components 0..1.
fn hsv(h: f32, s: f32, v: f32) -> [f32; 3] {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    match i as i32 % 6 {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_are_in_range() {
        for name in [
            "minecraft:stone",
            "minecraft:grass_block",
            "minecraft:totally_modded_block",
        ] {
            let color = block_color(name);
            assert!(color.iter().all(|c| (0.0..=1.0).contains(c)), "{name}");
        }
        // The namespace must not affect the color.
        assert_eq!(block_color("mod:foo"), block_color("other:foo"));
    }

    #[test]
    fn air_detection() {
        assert!(is_air("minecraft:air"));
        assert!(is_air("minecraft:cave_air"));
        assert!(!is_air("minecraft:stone"));
    }
}
