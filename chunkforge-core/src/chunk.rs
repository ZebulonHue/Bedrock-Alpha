//! 1.18+ chunk section decoding — palette + bit-packed data (port of
//! `readSection` in `src/lib/mc/parseCore.ts`).
//!
//! Per section: `Y`, `block_states.palette[]` (`Name` + optional
//! `Properties`), `block_states.data` (long array). Bits per index =
//! `max(4, ceil(log2(palette.len())))`; indices are packed LSB-first into
//! 64-bit longs and NEVER cross long boundaries (`floor(64/bits)` per long),
//! so plain `u64` shifts extract them (the TS 32-bit-halves trick is only a
//! performance dodge for BigInt-less math). Block index = `(y<<8)|(z<<4)|x`.

use crate::arena::{section_key, NameTable, SectionArena, SECTION_CELLS};
use crate::error::ParseError;
use crate::nbt::Nbt;

/// Air block names are dropped entirely (both namespaced and bare forms).
fn is_air(name: &str) -> bool {
    matches!(
        name,
        "minecraft:air"
            | "minecraft:cave_air"
            | "minecraft:void_air"
            | "air"
            | "cave_air"
            | "void_air"
    )
}

/// Add the `minecraft:` namespace when missing (TS `normalizeName`).
fn normalize_name(name: &str) -> String {
    if name.contains(':') {
        name.to_string()
    } else {
        format!("minecraft:{name}")
    }
}

/// Bits per palette index: `max(4, ceil(log2(n)))` computed exactly
/// (no float `log2` rounding traps).
pub fn bits_per_index(n_pal: usize) -> u32 {
    if n_pal <= 2 {
        // ceil(log2(1)) = 0, ceil(log2(2)) = 1 -> both clamp to 4
        return 4;
    }
    let ceil_log2 = usize::BITS - (n_pal - 1).leading_zeros();
    ceil_log2.max(4)
}

/// Decode one section's blocks into the section arena. Returns the number of
/// non-air blocks added. Corrupt-but-recoverable cells (palette index out of
/// range, truncated data) are treated as air, exactly like the TS parser.
pub fn read_section(
    section: &Nbt,
    chunk_x: i32,
    chunk_z: i32,
    arena: &mut SectionArena,
    names: &mut NameTable,
) -> Result<u64, ParseError> {
    let sy = match section.get("Y").and_then(Nbt::as_i32) {
        Some(v) => v,
        None => return Ok(0),
    };
    let bs = match section.get("block_states") {
        Some(bs @ Nbt::Compound(_)) => bs,
        _ => return Ok(0),
    };
    let palette = match bs.get("palette").and_then(Nbt::as_list) {
        Some(p) if !p.is_empty() => p,
        _ => return Ok(0),
    };
    let data = bs.get("data").and_then(Nbt::as_long_array);

    // Intern the palette once: ids[i] = name id, or -1 for air/invalid.
    let n_pal = palette.len();
    let mut ids = vec![-1i32; n_pal];
    let mut non_air = 0usize;
    for (i, entry) in palette.iter().enumerate() {
        let name = entry
            .get("Name")
            .and_then(Nbt::as_str)
            .map(normalize_name)
            .unwrap_or_default();
        if name.is_empty() || is_air(&name) {
            ids[i] = -1;
        } else {
            ids[i] = names.intern(&name)? as i32;
            non_air += 1;
        }
    }
    if non_air == 0 {
        return Ok(0);
    }

    // Section cells are stored as nameId + 1 so 0 means air/absent. The slot
    // is allocated lazily on the first non-air cell — air-only sections never
    // touch the store. Duplicate sections (corrupt files) overwrite only their
    // non-air cells, exactly like the old Map.set behavior.
    let key = section_key(chunk_x, chunk_z, sy);
    let mut sec_off: Option<usize> = None; // slot * SECTION_CELLS, latched on first write

    macro_rules! write_cell {
        ($i:expr, $id:expr) => {{
            let off = match sec_off {
                Some(o) => o,
                None => {
                    let o = arena.alloc(key) as usize * SECTION_CELLS;
                    sec_off = Some(o);
                    o
                }
            };
            arena.cells_mut()[off + $i] = ($id as u16) + 1;
        }};
    }

    let data = match data {
        Some(d) if !d.is_empty() => d,
        _ => {
            // Whole section is palette[0].
            let only = ids[0];
            if only < 0 {
                return Ok(0);
            }
            let off = match sec_off {
                Some(o) => o,
                None => arena.alloc(key) as usize * SECTION_CELLS,
            };
            arena.cells_mut()[off..off + SECTION_CELLS].fill((only as u16) + 1);
            return Ok(4096);
        }
    };

    let bits = bits_per_index(n_pal) as usize;
    let per_long = 64 / bits;
    let mut added = 0u64;

    if bits > 30 {
        // Absurd palette size (corrupt file) — careful extraction fallback.
        // (In TS this path uses exact BigInt math; u64 shifts suffice here
        // because indices never cross 64-bit boundaries.)
        if per_long == 0 {
            return Ok(0); // TS: li = Infinity >= data.length -> immediate stop
        }
        let mask: u64 = if bits >= 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        let mut cur_long = usize::MAX;
        let mut cur_word = 0u64;
        for i in 0..4096usize {
            let li = i / per_long;
            if li >= data.len() {
                break; // truncated data — stop, don't invent blocks
            }
            if li != cur_long {
                cur_long = li;
                cur_word = data[li] as u64;
            }
            let shift = ((i % per_long) * bits) as u32;
            let idx = (cur_word >> shift) & mask;
            if idx >= n_pal as u64 {
                continue; // corrupt entry — treat as air
            }
            let id = ids[idx as usize];
            if id < 0 {
                continue;
            }
            write_cell!(i, id);
            added += 1;
        }
        return Ok(added);
    }

    // Hot path: plain u64 shifts. `off` stays < 64 because an index never
    // crosses a long boundary (per_long * bits <= 64 by construction).
    let mask = (1u64 << bits) - 1;
    let mut i = 0usize;
    'long_loop: for &word in data {
        let word = word as u64;
        let mut off = 0usize;
        for _ in 0..per_long {
            if i >= 4096 {
                break 'long_loop;
            }
            let idx = ((word >> off) & mask) as usize;
            off += bits;
            if idx < n_pal {
                let id = ids[idx];
                if id >= 0 {
                    write_cell!(i, id);
                    added += 1;
                }
            }
            i += 1;
        }
    }
    // Truncated data (fewer longs than 4096 indices need) just stops — the
    // missing tail is treated as air, matching the TS behavior.
    Ok(added)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::unkey;

    fn pal(names: &[&str]) -> Nbt {
        Nbt::List(
            names
                .iter()
                .map(|n| Nbt::Compound(vec![("Name".into(), Nbt::String(n.to_string()))]))
                .collect(),
        )
    }

    fn section(y: i8, palette: Nbt, data: Option<Vec<i64>>) -> Nbt {
        let mut bs = vec![("palette".into(), palette)];
        if let Some(d) = data {
            bs.push(("data".into(), Nbt::LongArray(d)));
        }
        Nbt::Compound(vec![
            ("Y".into(), Nbt::Byte(y)),
            ("block_states".into(), Nbt::Compound(bs)),
        ])
    }

    /// Pack palette indices LSB-first, floor(64/bits) per long (never crossing).
    fn pack(indices: &[usize], bits: usize) -> Vec<i64> {
        let per_long = 64 / bits;
        let mut out = Vec::new();
        for chunk in indices.chunks(per_long) {
            let mut w = 0u64;
            for (k, &idx) in chunk.iter().enumerate() {
                w |= (idx as u64) << (k * bits);
            }
            out.push(w as i64);
        }
        out
    }

    fn decode(sec: &Nbt) -> (SectionArena, NameTable, u64) {
        let mut arena = SectionArena::new();
        let mut names = NameTable::new();
        let added = read_section(sec, 5, -7, &mut arena, &mut names).unwrap();
        (arena, names, added)
    }

    #[test]
    fn width4_all_stone() {
        // 16-entry palette -> 4 bits/index, 16 per long.
        let mut names = vec!["minecraft:air"];
        names.extend(vec!["minecraft:stone"; 15]);
        let data = pack(&[1usize; 4096], 4);
        assert_eq!(data.len(), 256);
        let (arena, _names, added) = decode(&section(2, pal(&names), Some(data)));
        assert_eq!(added, 4096);
        let key = section_key(5, -7, 2);
        let slot = arena.get(key).expect("allocated");
        let cells = arena.cells();
        assert!(
            cells[slot as usize * SECTION_CELLS..(slot as usize + 1) * SECTION_CELLS]
                .iter()
                .all(|&v| v == 1),
            "every cell is name_id 0 + 1"
        );
        assert_eq!(unkey(key), (5, -7, 2));
    }

    #[test]
    fn width5_no_long_crossing() {
        // 17-entry palette -> 5 bits/index, 12 per long (12*5=60; bits 60..63 unused).
        // 16 DISTINCT names so palette index k maps to interned name id k-1.
        let mut names: Vec<&str> = vec!["minecraft:air"];
        names.extend_from_slice(&[
            "minecraft:stone",
            "minecraft:dirt",
            "minecraft:granite",
            "minecraft:diorite",
            "minecraft:andesite",
            "minecraft:cobblestone",
            "minecraft:sand",
            "minecraft:gravel",
            "minecraft:clay",
            "minecraft:bricks",
            "minecraft:bedrock",
            "minecraft:obsidian",
            "minecraft:netherrack",
            "minecraft:tuff",
            "minecraft:calcite",
            "minecraft:deepslate",
        ]);
        // cells 0..=13 -> palette indices 1..=14 (name ids 0..=13 -> cell values 1..=14).
        let mut indices: Vec<usize> = (1..=14).collect();
        indices.resize(4096, 0);
        let mut data = pack(&indices, 5);
        assert_eq!(data.len(), 342); // ceil(4096/12)
                                     // Set the unused top 4 bits of every long to garbage — a decoder that
                                     // lets indices cross long boundaries would misread the next cell.
        for w in data.iter_mut() {
            *w |= (0xFu64 << 60) as i64;
        }
        let (arena, names_t, added) = decode(&section(0, pal(&names), Some(data)));
        assert_eq!(added, 14);
        let slot = arena.get(section_key(5, -7, 0)).unwrap() as usize * SECTION_CELLS;
        let cells = arena.cells();
        for i in 0..14usize {
            assert_eq!(cells[slot + i], (i + 1) as u16, "cell {i}");
        }
        assert_eq!(cells[slot + 14], 0, "garbage bits must not become a block");
        assert_eq!(names_t.names[0], "minecraft:stone");
        assert_eq!(names_t.names[1], "minecraft:dirt");
    }

    #[test]
    fn truncated_data_tail_is_air() {
        let names = vec!["minecraft:stone"];
        // palette len 1 -> 4 bits... but TS: max(4, ceil(log2(1)))=4; data with 1 long = 16 cells.
        let data = vec![-1i64]; // all 4-bit groups = 15 -> idx 15 >= nPal(1) -> air!
        let (_a, _n, added) = decode(&section(0, pal(&names), Some(data)));
        assert_eq!(added, 0, "out-of-range palette indices are air");
        // now idx 0 in first long only, then truncated
        let data = vec![0i64];
        let (arena, _n, added) = decode(&section(0, pal(&names), Some(data)));
        assert_eq!(added, 16, "one long = 16 cells; the remaining 4080 are air");
        let slot = arena.get(section_key(5, -7, 0)).unwrap() as usize * SECTION_CELLS;
        assert!(arena.cells()[slot..slot + 16].iter().all(|&v| v == 1));
        assert_eq!(arena.cells()[slot + 16], 0);
    }

    #[test]
    fn no_data_fills_palette0() {
        let names = vec!["minecraft:stone"];
        let (arena, _n, added) = decode(&section(-3, pal(&names), None));
        assert_eq!(added, 4096);
        let slot = arena.get(section_key(5, -7, -3)).unwrap() as usize * SECTION_CELLS;
        assert!(arena.cells()[slot..slot + 4096].iter().all(|&v| v == 1));
        // palette[0] = air, no data -> nothing allocated
        let names = vec!["minecraft:air"];
        let (arena, _n, added) = decode(&section(0, pal(&names), None));
        assert_eq!(added, 0);
        assert_eq!(arena.get(section_key(5, -7, 0)), None);
    }

    #[test]
    fn air_only_and_missing_fields() {
        // palette of only air with data -> no allocation, no blocks
        let names = vec!["minecraft:air", "minecraft:cave_air"];
        let data = pack(&[1usize; 4096], 4);
        let (arena, _n, added) = decode(&section(0, pal(&names), Some(data)));
        assert_eq!(added, 0);
        assert_eq!(arena.get(section_key(5, -7, 0)), None);
        // missing Y / block_states / palette -> no-op
        let mut arena = SectionArena::new();
        let mut names_t = NameTable::new();
        assert_eq!(
            read_section(&Nbt::Compound(vec![]), 0, 0, &mut arena, &mut names_t).unwrap(),
            0
        );
        let no_palette = Nbt::Compound(vec![
            ("Y".into(), Nbt::Byte(0)),
            ("block_states".into(), Nbt::Compound(vec![])),
        ]);
        assert_eq!(
            read_section(&no_palette, 0, 0, &mut arena, &mut names_t).unwrap(),
            0
        );
    }

    #[test]
    fn bits_rule() {
        assert_eq!(bits_per_index(1), 4);
        assert_eq!(bits_per_index(2), 4);
        assert_eq!(bits_per_index(13), 4); // SPEC's worked example
        assert_eq!(bits_per_index(16), 4);
        assert_eq!(bits_per_index(17), 5);
        assert_eq!(bits_per_index(256), 8);
        assert_eq!(bits_per_index(257), 9);
    }
}
