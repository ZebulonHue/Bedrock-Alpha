//! Block appearance table — ported from the ChunkForge webapp's
//! `src/lib/mc/appearance.ts`.
//!
//! Maps common vanilla block names to a fallback color, a jar texture path and
//! an `opaque` flag. `opaque` is `true` ONLY for full opaque cubes — it drives
//! exterior culling, so a wrongly-opaque entry would cull visible blocks
//! (e.g. torches must never occlude). When in doubt, the TS table marks
//! `opaque: false`; unknown names fall back to `opaque: true` unless a
//! substring hint (slab/stair/torch/leaves/…) says otherwise.
//!
//! The 350-entry table lives in `appearance_table.inc` (machine-translated
//! from the TS source by a throwaway script, then committed). It is sorted by
//! name and queried with a binary search.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Visual properties of one block type.
pub struct BlockAppearance {
    /// Fallback color (matches the TS CSS hex, e.g. `#7d7d7d` -> `[0x7d, 0x7d, 0x7d]`).
    pub color: [u8; 3],
    /// Path inside the minecraft jar, e.g. `assets/minecraft/textures/block/stone.png`.
    pub texture_path: Option<&'static str>,
    /// `true` only for full opaque cubes — drives occlusion culling.
    pub opaque: bool,
}

include!("appearance_table.inc");

/// Binary-search the sorted table for `short` (name without namespace).
fn table_lookup(short: &str) -> Option<&'static (&'static str, [u8; 3], bool, &'static str)> {
    TABLE
        .binary_search_by(|(name, _, _, _)| (*name).cmp(short))
        .ok()
        .map(|i| &TABLE[i])
}

/// JS `String.prototype.charCodeAt`-compatible 31-multiply hash.
/// Matches TS: `h = (h * 31 + charCode) | 0` over UTF-16 code units.
fn hash_hue(name: &str) -> i32 {
    let mut h: i32 = 0;
    for unit in name.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(unit as i32);
    }
    // TS: ((h % 360) + 360) % 360 — rem_euclid is identical for a positive modulus.
    h.rem_euclid(360)
}

/// HSL -> RGB, ported from the TS `hslToHex(hue, 0.35, 0.45)`.
fn hsl_to_rgb(h: i32, s: f64, l: f64) -> [u8; 3] {
    let a = s * l.min(1.0 - l);
    let f = |n: i32| -> u8 {
        let k = ((n as f64 + h as f64 / 30.0) % 12.0 + 12.0) % 12.0;
        let c = l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0);
        (255.0 * c).round() as u8
    };
    [f(0), f(8), f(4)]
}

/// Cache for runtime-guessed texture paths of unknown blocks, so repeated
/// lookups of the same name leak at most one boxed str per distinct name.
/// (Parsing interns at most 65,534 distinct names, so this stays bounded.)
fn guessed_texture_path(short: &str) -> &'static str {
    static CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(p) = guard.get(short) {
        return p;
    }
    let path: &'static str =
        Box::leak(format!("assets/minecraft/textures/block/{short}.png").into_boxed_str());
    guard.insert(short.to_string(), path);
    path
}

/// Look up the appearance of a block. Accepts names with or without the
/// `minecraft:` namespace. Unknown blocks get a deterministic muted color,
/// a guessed texture path, and `opaque: true` unless the name hints at a
/// non-occluding shape (slab/stairs/torch/leaves/...). Table entries always
/// win over hints.
pub fn appearance(name: &str) -> BlockAppearance {
    let short = match name.split_once(':') {
        Some((_, s)) => s,
        None => name,
    };
    if let Some((_, color, opaque, texture)) = table_lookup(short) {
        return BlockAppearance {
            color: *color,
            texture_path: Some(texture),
            opaque: *opaque,
        };
    }
    let opaque = !NON_OPAQUE_HINTS.iter().any(|hint| short.contains(hint));
    BlockAppearance {
        color: hsl_to_rgb(hash_hue(short), 0.35, 0.45),
        texture_path: Some(guessed_texture_path(short)),
        opaque,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_entries() {
        assert!(appearance("minecraft:stone").opaque);
        assert!(appearance("minecraft:bedrock").opaque);
        assert!(
            !appearance("minecraft:torch").opaque,
            "torch must NOT occlude"
        );
        assert!(!appearance("minecraft:glass").opaque);
        assert!(!appearance("minecraft:oak_leaves").opaque);
        assert_eq!(
            appearance("minecraft:torch").texture_path,
            Some("assets/minecraft/textures/block/torch.png")
        );
        assert_eq!(
            appearance("minecraft:grass_block").texture_path,
            Some("assets/minecraft/textures/block/grass_block_top.png")
        );
        assert_eq!(
            appearance("minecraft:oak_log").texture_path,
            Some("assets/minecraft/textures/block/oak_log.png")
        );
        assert_eq!(appearance("stone").color, [0x7d, 0x7d, 0x7d]);
    }

    #[test]
    fn hint_fallbacks() {
        // Not in the table — hints drive opaque:false.
        assert!(!appearance("minecraft:oak_stairs").opaque);
        assert!(!appearance("minecraft:stone_slab").opaque);
        assert!(!appearance("minecraft:modded_copper_fence").opaque);
        // Table wins over hints: snow_block/ice contain "snow"/"ice" but are opaque cubes.
        assert!(appearance("minecraft:snow_block").opaque);
        assert!(appearance("minecraft:packed_ice").opaque);
    }

    #[test]
    fn unknown_block() {
        let a = appearance("minecraft:unobtainium_block");
        assert!(a.opaque, "unknown blocks default to opaque");
        assert_eq!(
            a.texture_path,
            Some("assets/minecraft/textures/block/unobtainium_block.png")
        );
        assert_eq!(
            a.color,
            appearance("minecraft:unobtainium_block").color,
            "deterministic"
        );
        assert!(
            a.texture_path == appearance("unobtainium_block").texture_path,
            "namespace-insensitive"
        );
    }

    #[test]
    fn table_is_sorted() {
        for w in TABLE.windows(2) {
            assert!(
                w[0].0 < w[1].0,
                "table not sorted at {} >= {}",
                w[0].0,
                w[1].0
            );
        }
    }
}
