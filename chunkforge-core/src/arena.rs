//! Section-arena block storage — the memory design that cut peak RSS ~3.5x
//! vs per-block hash maps in the TS parser (1639 MB -> ~470 MB on a
//! 23.4M-block region).
//!
//! Blocks live in ONE growable `Vec<u16>` arena of 4096-cell sections; each
//! cell holds `name_id + 1` (0 = air/absent). Sections are keyed numerically:
//!
//! ```text
//! key = ((chunkX + 2^21) * 2^22 + (chunkZ + 2^21)) * 64 + (sectionY + 32)
//! ```
//!
//! Chunk coords are bounded by the ±30M world border (±1.875M chunks < 2^21)
//! and sectionY fits in 6 bits, so the key stays below 2^50 and neighbor
//! sections are pure arithmetic: y±1 -> key±1, z±1 -> key±64, x±1 -> key±2^28.

use crate::appearance::appearance;
use crate::error::ParseError;
use std::collections::HashMap;

pub const SECTION_CELLS: usize = 4096;

const AXIS_OFF: i64 = 1 << 21; // 2^21
const COL_STRIDE: i64 = 1 << 22; // 2^22
const SEC_Y_OFF: i64 = 32;
const Y_STRIDE: i64 = 64; // 2^6

/// Neighbor-section key deltas (derived from the packing above).
pub const D_SEC_Y: i64 = 1;
pub const D_SEC_Z: i64 = Y_STRIDE; // 64
pub const D_SEC_X: i64 = COL_STRIDE * Y_STRIDE; // 2^28

/// Numeric section key — see module docs.
pub fn section_key(chunk_x: i32, chunk_z: i32, section_y: i32) -> i64 {
    ((chunk_x as i64 + AXIS_OFF) * COL_STRIDE + (chunk_z as i64 + AXIS_OFF)) * Y_STRIDE
        + (section_y as i64 + SEC_Y_OFF)
}

/// Invert a section key back to (chunk_x, chunk_z, section_y).
/// Exact: keys are always positive (< 2^50), matching the TS `Math.floor` math.
pub fn unkey(key: i64) -> (i32, i32, i32) {
    let col_key = key / Y_STRIDE;
    let sx = col_key / COL_STRIDE - AXIS_OFF;
    let sz = col_key - (sx + AXIS_OFF) * COL_STRIDE - AXIS_OFF;
    let sy = key - col_key * Y_STRIDE - SEC_Y_OFF;
    (sx as i32, sz as i32, sy as i32)
}

/// Sparse world: section key -> slot in one growable arena. Iteration follows
/// insertion order (like the TS `Map`), keeping output ordering deterministic.
pub struct SectionArena {
    arena: Vec<u16>,
    slots: HashMap<i64, u32>,
    order: Vec<(i64, u32)>,
    size: u32,
}

impl SectionArena {
    pub fn new() -> Self {
        SectionArena {
            arena: vec![0; SECTION_CELLS * 64],
            slots: HashMap::new(),
            order: Vec::new(),
            size: 0,
        }
    }

    /// Slot index for `key`, allocating a zeroed section if absent.
    pub fn alloc(&mut self, key: i64) -> u32 {
        if let Some(&slot) = self.slots.get(&key) {
            return slot;
        }
        let slot = self.size;
        self.size += 1;
        if slot as usize * SECTION_CELLS >= self.arena.len() {
            let next = self.arena.len() * 2;
            self.arena.resize(next, 0);
        }
        self.slots.insert(key, slot);
        self.order.push((key, slot));
        slot
    }

    /// Slot index for `key`, or `None` if the section has no non-air cells.
    pub fn get(&self, key: i64) -> Option<u32> {
        self.slots.get(&key).copied()
    }

    /// The raw arena — index cells as `arena[slot * SECTION_CELLS + i]`.
    pub fn cells(&self) -> &[u16] {
        &self.arena
    }

    pub fn cells_mut(&mut self) -> &mut [u16] {
        &mut self.arena
    }

    /// Iterate `(section_key, slot)` pairs in insertion order.
    pub fn entries(&self) -> impl Iterator<Item = (i64, u32)> + '_ {
        self.order.iter().copied()
    }

    /// Drop excess capacity before the culling pass (one copy).
    pub fn shrink(&mut self) {
        self.arena.truncate(self.size as usize * SECTION_CELLS);
    }

    /// Release everything (after culling) so the arena memory is freed BEFORE
    /// the per-type output vectors grow the heap.
    pub fn clear(&mut self) {
        self.arena = Vec::new();
        self.slots.clear();
        self.order.clear();
        self.size = 0;
    }
}

/// Interned block names: section cells store `name_id + 1` in a `u16`, so the
/// table is bounded at 65,534 names (vanilla + any modpack stays in the low
/// thousands; the guard only trips on a deliberately hostile file).
pub const MAX_NAME_IDS: usize = 65534;

pub struct NameTable {
    pub names: Vec<String>,
    ids: HashMap<String, u32>,
    pub opaque: Vec<bool>,
}

impl NameTable {
    pub fn new() -> Self {
        NameTable {
            names: Vec::new(),
            ids: HashMap::new(),
            opaque: Vec::new(),
        }
    }

    pub fn intern(&mut self, name: &str) -> Result<u32, ParseError> {
        if let Some(&id) = self.ids.get(name) {
            return Ok(id);
        }
        if self.names.len() >= MAX_NAME_IDS {
            return Err(ParseError::CorruptNbt(format!(
                "ChunkForge: too many distinct block names (>{MAX_NAME_IDS})"
            )));
        }
        let id = self.names.len() as u32;
        self.names.push(name.to_string());
        self.ids.insert(name.to_string(), id);
        self.opaque.push(appearance(name).opaque);
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_roundtrip() {
        for (cx, cz, sy) in [
            (0, 0, 0),
            (1, 0, -4),
            (-100, 200, 19),
            (31, 31, 3),
            (-1, -1, -1),
        ] {
            assert_eq!(unkey(section_key(cx, cz, sy)), (cx, cz, sy));
        }
    }

    #[test]
    fn neighbor_keys() {
        let k = section_key(3, 4, 5);
        assert_eq!(unkey(k + D_SEC_Y), (3, 4, 6));
        assert_eq!(unkey(k - D_SEC_Z), (3, 3, 5));
        assert_eq!(unkey(k + D_SEC_X), (4, 4, 5));
    }

    #[test]
    fn alloc_get_grow() {
        let mut a = SectionArena::new();
        let s0 = a.alloc(section_key(0, 0, 0));
        let s1 = a.alloc(section_key(0, 0, 1));
        assert_eq!(s0, 0);
        assert_eq!(s1, 1);
        assert_eq!(
            a.alloc(section_key(0, 0, 0)),
            0,
            "re-alloc returns same slot"
        );
        assert_eq!(a.get(section_key(9, 9, 9)), None);
        a.cells_mut()[s1 as usize * SECTION_CELLS + 5] = 42;
        assert_eq!(a.cells()[SECTION_CELLS + 5], 42);
        // grow past the initial 64-section capacity
        for i in 0..200 {
            a.alloc(section_key(i + 10, 0, 0));
        }
        a.shrink();
        assert_eq!(a.cells().len(), 202 * SECTION_CELLS);
        a.clear();
        assert!(a.cells().is_empty());
    }

    #[test]
    fn intern_ids() {
        let mut t = NameTable::new();
        let a = t.intern("minecraft:stone").unwrap();
        let b = t.intern("minecraft:torch").unwrap();
        assert_eq!(a, 0);
        assert_eq!(b, 1);
        assert_eq!(t.intern("minecraft:stone").unwrap(), 0);
        assert!(t.opaque[0]);
        assert!(!t.opaque[1]);
    }
}
