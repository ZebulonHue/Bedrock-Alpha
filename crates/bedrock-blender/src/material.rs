//! Material conventions for Project Bedrock exports.
//!
//! This module defines:
//! - How block names map to Blender material names (compatible with MCprep
//!   and Mineways conventions).
//! - PBR presets for common Minecraft block categories (roughness, metallic,
//!   emissive, transparency).
//! - Utility functions for generating MTL entries and Blender node setups.

/// How strongly a block category participates in PBR rendering.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AlphaMode {
    /// Fully opaque — no transparency.
    Opaque,
    /// Binary transparency — pixels are either fully opaque or invisible
    /// (e.g. leaves, iron bars).
    Mask,
    /// Gradual transparency — alpha blended (e.g. glass, ice, water).
    Blend,
}

/// PBR material preset for a Minecraft-like block.
///
/// These values target Cycles/Eevee Principled BSDF parameters and are meant
/// as a starting point — artists can tweak them after import.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PbrPreset {
    /// Roughness (0 = mirror, 1 = diffuse).
    pub roughness: f32,
    /// Metallic (0 = dielectric, 1 = metal).
    pub metallic: f32,
    /// Emissive colour (RGB, 0-1 range) — zero means no emission.
    pub emissive: [f32; 3],
    /// Emissive strength multiplier.
    pub emissive_strength: f32,
    /// Alpha mode for transparency handling.
    pub alpha_mode: AlphaMode,
    /// Optional alpha cut-off threshold for `Mask` mode (typically 0.5).
    pub alpha_threshold: Option<f32>,
}

impl Default for PbrPreset {
    fn default() -> Self {
        Self {
            roughness: 0.8,
            metallic: 0.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_threshold: None,
        }
    }
}

impl PbrPreset {
    /// A fully diffuse, rough, non-metallic, opaque block (most common).
    pub const fn solid() -> Self {
        Self {
            roughness: 0.85,
            metallic: 0.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_threshold: None,
        }
    }

    /// Smooth block like stone or polished variants.
    pub const fn smooth() -> Self {
        Self {
            roughness: 0.4,
            metallic: 0.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_threshold: None,
        }
    }

    /// Metal block (iron, gold, copper, netherite).
    pub const fn metal() -> Self {
        Self {
            roughness: 0.3,
            metallic: 1.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_threshold: None,
        }
    }

    /// Emissive block (glowstone, shroomlight, lantern, sea lantern).
    pub const fn emissive() -> Self {
        Self {
            roughness: 0.7,
            metallic: 0.0,
            emissive: [1.0; 3],
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_threshold: None,
        }
    }

    /// Strongly emissive block (beacon, respawn anchor charged).
    pub const fn glowing() -> Self {
        Self {
            roughness: 0.5,
            metallic: 0.0,
            emissive: [2.0; 3],
            emissive_strength: 2.0,
            alpha_mode: AlphaMode::Opaque,
            alpha_threshold: None,
        }
    }

    /// Glass / ice: transparent with blend alpha.
    pub const fn glass() -> Self {
        Self {
            roughness: 0.05,
            metallic: 0.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Blend,
            alpha_threshold: None,
        }
    }

    /// Leaves / tinted glass: masked transparency.
    pub const fn masked() -> Self {
        Self {
            roughness: 0.8,
            metallic: 0.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Mask,
            alpha_threshold: Some(0.5),
        }
    }

    /// Water / lava: liquid appearance.
    pub const fn liquid() -> Self {
        Self {
            roughness: 0.1,
            metallic: 0.0,
            emissive: [0.0; 3],
            emissive_strength: 1.0,
            alpha_mode: AlphaMode::Blend,
            alpha_threshold: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Material naming conventions
// ─────────────────────────────────────────────────────────────────────────────

/// Naming convention for exported materials.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MaterialConvention {
    /// Short names like `stone`, `oak_planks`, `grass_block` (default).
    #[default]
    Short,
    /// Namespaced names like `minecraft:stone`, `minecraft:oak_planks`.
    Namespaced,
    /// MCprep-compatible naming (matches MCprep's sanitised convention).
    Mcprep,
}

/// Sanitise a block name so it is safe to use as a Blender material name.
///
/// Replaces any character that is not ASCII alphanumeric or underscore with
/// an underscore. This is the same transform that `bedrock-export` applies
/// when writing OBJ/MTL files.
///
/// # Example
///
/// ```
/// use bedrock_blender::material::sanitise_name;
/// assert_eq!(sanitise_name("stone"), "stone");
/// assert_eq!(sanitise_name("grass_block"), "grass_block");
/// assert_eq!(sanitise_name("weird-block!"), "weird_block_");
/// ```
pub fn sanitise_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Reconstruct a short block name from a sanitised material name.
///
/// This is the inverse of [`sanitise_name`] in the sense that underscores
/// that were originally other characters cannot be recovered — but the
/// function at least returns a human-readable approximation.
///
/// # Example
///
/// ```
/// use bedrock_blender::material::desanitise_name;
/// assert_eq!(desanitise_name("stone"), "stone");
/// assert_eq!(desanitise_name("grass_block"), "grass_block");
/// ```
pub fn desanitise_name(sanitised: &str) -> String {
    sanitised.to_owned()
}

/// Format a block name according to the given [`MaterialConvention`].
pub fn format_name(name: &str, convention: MaterialConvention) -> String {
    match convention {
        MaterialConvention::Short | MaterialConvention::Mcprep => sanitise_name(name),
        MaterialConvention::Namespaced => {
            if name.contains(':') {
                sanitise_name(name)
            } else {
                format!("minecraft:{}", sanitise_name(name))
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PBR preset lookup
// ─────────────────────────────────────────────────────────────────────────────

/// Look up the PBR preset for a short (namespaceless) block name.
///
/// Returns `None` for unknown blocks, in which case you should use
/// [`PbrPreset::solid()`] as a fallback.
///
/// # Example
///
/// ```
/// use bedrock_blender::material::{pbr_preset, AlphaMode, PbrPreset};
/// let preset = pbr_preset("glass").unwrap();
/// assert_eq!(preset.alpha_mode, AlphaMode::Blend);
/// assert!(preset.roughness < 0.1);
/// ```
pub fn pbr_preset(short_name: &str) -> Option<PbrPreset> {
    // Map from short block name → PBR preset.
    // Organised by category for maintainability.
    match short_name {
        // ── Metals ────────────────────────────────────────────────────────
        "iron_block"
        | "gold_block"
        | "copper_block"
        | "cut_copper"
        | "exposed_cut_copper"
        | "weathered_cut_copper"
        | "oxidized_cut_copper"
        | "waxed_copper_block"
        | "waxed_cut_copper"
        | "waxed_exposed_cut_copper"
        | "waxed_weathered_cut_copper"
        | "waxed_oxidized_cut_copper"
        | "netherite_block"
        | "raw_iron_block"
        | "raw_gold_block"
        | "raw_copper_block"
        | "iron_door"
        | "iron_trapdoor"
        | "iron_bars"
        | "heavy_weighted_pressure_plate"
        | "lantern"
        | "soul_lantern" => Some(PbrPreset::metal()),

        // ── Glass / transparent ───────────────────────────────────────────
        "glass"
        | "white_stained_glass"
        | "orange_stained_glass"
        | "magenta_stained_glass"
        | "light_blue_stained_glass"
        | "yellow_stained_glass"
        | "lime_stained_glass"
        | "pink_stained_glass"
        | "gray_stained_glass"
        | "light_gray_stained_glass"
        | "cyan_stained_glass"
        | "purple_stained_glass"
        | "blue_stained_glass"
        | "brown_stained_glass"
        | "green_stained_glass"
        | "red_stained_glass"
        | "black_stained_glass"
        | "glass_pane"
        | "white_stained_glass_pane"
        | "orange_stained_glass_pane"
        | "magenta_stained_glass_pane"
        | "light_blue_stained_glass_pane"
        | "yellow_stained_glass_pane"
        | "lime_stained_glass_pane"
        | "pink_stained_glass_pane"
        | "gray_stained_glass_pane"
        | "light_gray_stained_glass_pane"
        | "cyan_stained_glass_pane"
        | "purple_stained_glass_pane"
        | "blue_stained_glass_pane"
        | "brown_stained_glass_pane"
        | "green_stained_glass_pane"
        | "red_stained_glass_pane"
        | "black_stained_glass_pane"
        | "ice"
        | "packed_ice"
        | "blue_ice"
        | "frosted_ice" => Some(PbrPreset::glass()),

        // ── Tinted glass (masked) ─────────────────────────────────────────
        "tinted_glass" => Some(PbrPreset::masked()),

        // ── Emissive ──────────────────────────────────────────────────────
        "glowstone"
        | "shroomlight"
        | "sea_lantern"
        | "jack_o_lantern"
        | "redstone_lamp"
        | "ochre_froglight"
        | "verdant_froglight"
        | "pearlescent_froglight"
        | "crying_obsidian"
        | "magma_block" => Some(PbrPreset::emissive()),

        // ── Strongly emissive ─────────────────────────────────────────────
        "beacon" | "respawn_anchor" => Some(PbrPreset::glowing()),

        // ── Smooth stone variants ─────────────────────────────────────────
        "stone"
        | "smooth_stone"
        | "stone_slab"
        | "polished_granite"
        | "polished_diorite"
        | "polished_andesite"
        | "polished_blackstone"
        | "polished_basalt"
        | "polished_deepslate"
        | "smooth_quartz"
        | "smooth_sandstone"
        | "smooth_red_sandstone"
        | "chiseled_quartz_block"
        | "chiseled_stone_bricks"
        | "chiseled_deepslate"
        | "chiseled_nether_bricks"
        | "chiseled_polished_blackstone"
        | "calcite"
        | "dripstone_block" => Some(PbrPreset::smooth()),

        // ── Leaves (masked) ───────────────────────────────────────────────
        "oak_leaves"
        | "spruce_leaves"
        | "birch_leaves"
        | "jungle_leaves"
        | "acacia_leaves"
        | "dark_oak_leaves"
        | "mangrove_leaves"
        | "cherry_leaves"
        | "azalea_leaves"
        | "flowering_azalea_leaves" => Some(PbrPreset::masked()),

        // ── Liquids ───────────────────────────────────────────────────────
        "water" | "lava" => Some(PbrPreset::liquid()),

        // ── Everything else defaults to solid ─────────────────────────────
        _ => None,
    }
}

/// Return an iterator over all known block names that have PBR presets.
pub fn known_pbr_blocks() -> Vec<&'static str> {
    // This is a convenience for the add-on generator so it can include a
    // comprehensive preset table. We just reuse the match arms from
    // `pbr_preset` by listing all known names.
    vec![
        // Metals
        "iron_block",
        "gold_block",
        "copper_block",
        "cut_copper",
        "exposed_cut_copper",
        "weathered_cut_copper",
        "oxidized_cut_copper",
        "waxed_copper_block",
        "waxed_cut_copper",
        "waxed_exposed_cut_copper",
        "waxed_weathered_cut_copper",
        "waxed_oxidized_cut_copper",
        "netherite_block",
        "raw_iron_block",
        "raw_gold_block",
        "raw_copper_block",
        "iron_door",
        "iron_trapdoor",
        "iron_bars",
        "heavy_weighted_pressure_plate",
        "lantern",
        "soul_lantern",
        // Glass
        "glass",
        "white_stained_glass",
        "orange_stained_glass",
        "magenta_stained_glass",
        "light_blue_stained_glass",
        "yellow_stained_glass",
        "lime_stained_glass",
        "pink_stained_glass",
        "gray_stained_glass",
        "light_gray_stained_glass",
        "cyan_stained_glass",
        "purple_stained_glass",
        "blue_stained_glass",
        "brown_stained_glass",
        "green_stained_glass",
        "red_stained_glass",
        "black_stained_glass",
        "glass_pane",
        "white_stained_glass_pane",
        "orange_stained_glass_pane",
        "magenta_stained_glass_pane",
        "light_blue_stained_glass_pane",
        "yellow_stained_glass_pane",
        "lime_stained_glass_pane",
        "pink_stained_glass_pane",
        "gray_stained_glass_pane",
        "light_gray_stained_glass_pane",
        "cyan_stained_glass_pane",
        "purple_stained_glass_pane",
        "blue_stained_glass_pane",
        "brown_stained_glass_pane",
        "green_stained_glass_pane",
        "red_stained_glass_pane",
        "black_stained_glass_pane",
        "ice",
        "packed_ice",
        "blue_ice",
        "frosted_ice",
        // Tinted
        "tinted_glass",
        // Emissive
        "glowstone",
        "shroomlight",
        "sea_lantern",
        "jack_o_lantern",
        "redstone_lamp",
        "ochre_froglight",
        "verdant_froglight",
        "pearlescent_froglight",
        "crying_obsidian",
        "magma_block",
        // Strongly emissive
        "beacon",
        "respawn_anchor",
        // Smooth
        "stone",
        "smooth_stone",
        "stone_slab",
        "polished_granite",
        "polished_diorite",
        "polished_andesite",
        "polished_blackstone",
        "polished_basalt",
        "polished_deepslate",
        "smooth_quartz",
        "smooth_sandstone",
        "smooth_red_sandstone",
        "chiseled_quartz_block",
        "chiseled_stone_bricks",
        "chiseled_deepslate",
        "chiseled_nether_bricks",
        "chiseled_polished_blackstone",
        "calcite",
        "dripstone_block",
        // Leaves
        "oak_leaves",
        "spruce_leaves",
        "birch_leaves",
        "jungle_leaves",
        "acacia_leaves",
        "dark_oak_leaves",
        "mangrove_leaves",
        "cherry_leaves",
        "azalea_leaves",
        "flowering_azalea_leaves",
        // Liquids
        "water",
        "lava",
    ]
}

/// Check if a block name is known to be supported by MCprep's MeshSwap library.
pub fn is_mcprep_meshswap_block(short_name: &str) -> bool {
    matches!(
        short_name,
        "grass_block"
            | "dandelion"
            | "poppy"
            | "blue_orchid"
            | "allium"
            | "azure_bluet"
            | "red_tulip"
            | "orange_tulip"
            | "white_tulip"
            | "pink_tulip"
            | "oxeye_daisy"
            | "cornflower"
            | "lily_of_the_valley"
            | "sunflower"
            | "lilac"
            | "rose_bush"
            | "peony"
            | "wheat"
            | "carrots"
            | "potatoes"
            | "beetroots"
            | "torch"
            | "soul_torch"
            | "redstone_torch"
            | "lantern"
            | "soul_lantern"
            | "chest"
            | "trapped_chest"
            | "ender_chest"
            | "crafting_table"
            | "furnace"
            | "blast_furnace"
            | "smoker"
            | "oak_leaves"
            | "spruce_leaves"
            | "birch_leaves"
            | "jungle_leaves"
            | "acacia_leaves"
            | "dark_oak_leaves"
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// MTL generation helpers
// ─────────────────────────────────────────────────────────────────────────────

/// A generated MTL material entry.
#[derive(Debug, Clone)]
pub struct MtlEntry {
    /// Material name (matches `newmtl` line).
    pub name: String,
    /// Diffuse colour (Kd) — RGB 0-1.
    pub diffuse: [f32; 3],
    /// Ambient colour (Ka) — typically black.
    pub ambient: [f32; 3],
    /// Specular colour (Ks) — typically black for non-metals.
    pub specular: [f32; 3],
    /// Shininess (Ns) — 0-1000.
    pub shininess: f32,
    /// Opacity (d / Tr) — 1.0 = fully opaque.
    pub opacity: f32,
    /// Path to the diffuse texture map.
    pub map_kd: Option<String>,
    /// Path to the normal map.
    pub map_ka: Option<String>,
    /// Path to the specular map.
    pub map_ks: Option<String>,
}

impl MtlEntry {
    /// Build an MTL entry for a block material, using the atlas PNG as the
    /// sole texture map and optionally applying PBR-derived defaults.
    pub fn new(name: String, atlas_path: &str, preset: Option<&PbrPreset>) -> Self {
        let p = preset.copied().unwrap_or_default();
        Self {
            name,
            diffuse: [1.0; 3],
            ambient: [0.0; 3],
            specular: if p.metallic > 0.5 { [0.5; 3] } else { [0.0; 3] },
            shininess: (1.0 - p.roughness).max(0.0) * 100.0,
            opacity: if matches!(p.alpha_mode, AlphaMode::Opaque) {
                1.0
            } else {
                0.9
            },
            map_kd: Some(atlas_path.to_owned()),
            map_ka: None,
            map_ks: None,
        }
    }

    /// Render this entry as lines in a `.mtl` file.
    pub fn to_mtl_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!("newmtl {}", self.name));
        lines.push(format!(
            "Kd {:.4} {:.4} {:.4}",
            self.diffuse[0], self.diffuse[1], self.diffuse[2]
        ));
        lines.push(format!(
            "Ka {:.4} {:.4} {:.4}",
            self.ambient[0], self.ambient[1], self.ambient[2]
        ));
        lines.push(format!(
            "Ks {:.4} {:.4} {:.4}",
            self.specular[0], self.specular[1], self.specular[2]
        ));
        lines.push(format!("Ns {:.1}", self.shininess));
        if self.opacity < 1.0 {
            lines.push(format!("d {:.4}", self.opacity));
            lines.push(String::from("Tr 0.0000"));
        }
        if let Some(ref path) = self.map_kd {
            lines.push(format!("map_Kd {}", path));
        }
        if let Some(ref path) = self.map_ka {
            lines.push(format!("map_Ka {}", path));
        }
        if let Some(ref path) = self.map_ks {
            lines.push(format!("map_Ks {}", path));
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitise_removes_colons_and_hyphens() {
        assert_eq!(sanitise_name("minecraft:stone"), "minecraft_stone");
        assert_eq!(sanitise_name("weird-block!"), "weird_block_");
    }

    #[test]
    fn sanitise_preserves_underscores() {
        assert_eq!(sanitise_name("grass_block"), "grass_block");
    }

    #[test]
    fn format_short_removes_namespace() {
        assert_eq!(
            format_name("minecraft:stone", MaterialConvention::Short),
            "minecraft_stone"
        );
        assert_eq!(format_name("stone", MaterialConvention::Short), "stone");
    }

    #[test]
    fn format_namespaced_adds_prefix() {
        assert_eq!(
            format_name("stone", MaterialConvention::Namespaced),
            "minecraft:stone"
        );
    }

    #[test]
    fn glass_has_blend_alpha() {
        let preset = pbr_preset("glass").unwrap();
        assert_eq!(preset.alpha_mode, AlphaMode::Blend);
        assert!(preset.roughness < 0.1);
        assert_eq!(preset.metallic, 0.0);
    }

    #[test]
    fn iron_block_is_metallic() {
        let preset = pbr_preset("iron_block").unwrap();
        assert_eq!(preset.metallic, 1.0);
        assert_eq!(preset.alpha_mode, AlphaMode::Opaque);
    }

    #[test]
    fn glowstone_is_emissive() {
        let preset = pbr_preset("glowstone").unwrap();
        assert!(preset.emissive[0] > 0.0);
        assert!(preset.emissive[1] > 0.0);
        assert!(preset.emissive[2] > 0.0);
    }

    #[test]
    fn unknown_block_returns_none() {
        assert!(pbr_preset("nonexistent_block").is_none());
    }

    #[test]
    fn leaf_blocks_are_masked() {
        let preset = pbr_preset("oak_leaves").unwrap();
        assert_eq!(preset.alpha_mode, AlphaMode::Mask);
        assert_eq!(preset.alpha_threshold, Some(0.5));
    }

    #[test]
    fn water_is_liquid() {
        let preset = pbr_preset("water").unwrap();
        assert_eq!(preset.alpha_mode, AlphaMode::Blend);
    }

    #[test]
    fn mtl_entry_uses_preset_roughness() {
        let preset = PbrPreset::metal();
        let entry = MtlEntry::new("iron_block".into(), "atlas.png", Some(&preset));
        assert_eq!(entry.shininess, 70.0); // (1.0 - 0.3) * 100
        assert!(entry.specular[0] > 0.0); // metallic → specular
    }

    #[test]
    fn mtl_entry_default_is_opaque() {
        let entry = MtlEntry::new("stone".into(), "atlas.png", None);
        assert_eq!(entry.opacity, 1.0);
        assert_eq!(entry.map_kd, Some("atlas.png".into()));
    }

    #[test]
    fn mtl_to_lines_produces_valid_entry() {
        let entry = MtlEntry::new("test".into(), "tex.png", None);
        let lines = entry.to_mtl_lines();
        assert!(lines.iter().any(|l| l == "newmtl test"));
        assert!(lines.iter().any(|l| l.starts_with("Kd ")));
        assert!(lines.iter().any(|l| l.starts_with("map_Kd ")));
    }

    #[test]
    fn known_pbr_blocks_are_not_empty() {
        let count = known_pbr_blocks().len();
        assert!(count > 50, "expected at least 50 known blocks, got {count}");
    }

    #[test]
    fn mcprep_meshswap_blocks_detected() {
        assert!(is_mcprep_meshswap_block("grass_block"));
        assert!(is_mcprep_meshswap_block("torch"));
        assert!(is_mcprep_meshswap_block("dandelion"));
        assert!(!is_mcprep_meshswap_block("unknown_cube_x"));
    }
}
