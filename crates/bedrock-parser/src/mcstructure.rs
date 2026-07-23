//! Parser for Minecraft Bedrock Edition `.mcstructure` files.
//!
//! Structure blocks in Bedrock Edition can export regions as `.mcstructure`
//! files. This module reads those files and converts them into the same
//! `Vec<Chunk>` the rest of the pipeline already understands.
//!
//! ## File Format
//!
//! `.mcstructure` files are **Little-Endian NBT** (the same format decoded by
//! [`crate::nbt_le`]).  The root compound contains:
//!
//! ```text
//! format_version: int (always 1)
//! size:           list[int; 3]  — [X, Y, Z] in blocks
//! structure:
//!   block_palette: list[compound]  — each has "name" string
//!   block_indices: list[list[int]] — two layers (terrain, waterlogged)
//!                  flat index = z*size_y*size_x + y*size_x + x
//!   entities:    list[compound]  (ignored)
//! structure_world_origin: list[int; 3]  (ignored for mesh purposes)
//! ```
//!
//! Block indices reference the `block_palette` list by position.
//! Layer 0 is terrain; layer 1 contains waterlogged / extra blocks.

use crate::chunk::{Chunk, SectionData};
use crate::nbt_le::{NbtCursor, NbtValue};
use std::collections::HashMap;
use std::fmt;

/// Why reading a `.mcstructure` file failed.
#[derive(Debug)]
pub enum McStructureError {
    /// The NBT payload was malformed.
    Nbt(String),
    /// An expected field was absent or had the wrong type.
    Missing(String),
    /// The format version is not supported (we only understand v1).
    Unsupported(String),
}

impl fmt::Display for McStructureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            McStructureError::Nbt(e) => write!(f, "malformed NBT: {e}"),
            McStructureError::Missing(field) => write!(f, "missing field: {field}"),
            McStructureError::Unsupported(what) => write!(f, "unsupported: {what}"),
        }
    }
}

impl std::error::Error for McStructureError {}

/// A decoded `.mcstructure` file, ready to be converted to chunks.
pub struct McStructure {
    /// Bounding box width (X axis).
    pub size_x: i32,
    /// Bounding box height (Y axis).
    pub size_y: i32,
    /// Bounding box depth (Z axis).
    pub size_z: i32,
    /// Block palette: namespaced block names, e.g. `"minecraft:stone"`.
    pub palette: Vec<String>,
    /// Flat ZYX-order palette indices. Length = size_x * size_y * size_z.
    /// Index -1 means "no block" (air).
    pub indices: Vec<i32>,
}

impl McStructure {
    /// Parse a `.mcstructure` file from raw bytes.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, McStructureError> {
        let mut cursor = NbtCursor::new(bytes);
        let root = cursor
            .read_root()
            .map_err(|e| McStructureError::Nbt(e.to_string()))?;
        let root = root
            .as_compound()
            .ok_or_else(|| McStructureError::Missing("root compound".into()))?;

        // format_version (optional; we only understand 1).
        if let Some(NbtValue::Int(v)) = root.get("format_version") {
            if *v != 1 {
                return Err(McStructureError::Unsupported(format!(
                    "format_version {v} (expected 1)"
                )));
            }
        }

        // size: list of 3 ints.
        let size = match root.get("size") {
            Some(NbtValue::List(items)) => items,
            _ => return Err(McStructureError::Missing("size (list)".into())),
        };
        if size.len() != 3 {
            return Err(McStructureError::Missing(
                "size must have exactly 3 elements".into(),
            ));
        }
        let int_at = |list: &Vec<NbtValue>, i: usize| -> Result<i32, McStructureError> {
            match &list[i] {
                NbtValue::Int(v) => Ok(*v),
                _ => Err(McStructureError::Missing(format!("size[{i}] not int"))),
            }
        };
        let size_x = int_at(size, 0)?;
        let size_y = int_at(size, 1)?;
        let size_z = int_at(size, 2)?;
        let volume = (size_x * size_y * size_z) as usize;

        // structure compound.
        let structure = match root.get("structure") {
            Some(NbtValue::Compound(c)) => c,
            _ => return Err(McStructureError::Missing("structure (compound)".into())),
        };

        // structure.block_palette — list of compounds with "name".
        let block_palette_list = match structure.get("block_palette") {
            Some(NbtValue::List(items)) => items,
            _ => {
                return Err(McStructureError::Missing(
                    "structure.block_palette (list)".into(),
                ))
            }
        };

        let mut palette = Vec::with_capacity(block_palette_list.len());
        for entry in block_palette_list {
            let compound = entry.as_compound().ok_or_else(|| {
                McStructureError::Missing("block_palette entry must be compound".into())
            })?;
            let name = compound
                .get("name")
                .and_then(NbtValue::as_str)
                .ok_or_else(|| McStructureError::Missing("block_palette entry.name".into()))?;
            palette.push(name.to_owned());
        }

        // structure.block_indices — list of two lists.
        let block_indices_outer = match structure.get("block_indices") {
            Some(NbtValue::List(items)) => items,
            _ => {
                return Err(McStructureError::Missing(
                    "structure.block_indices (list)".into(),
                ))
            }
        };

        if block_indices_outer.is_empty() {
            return Err(McStructureError::Missing(
                "block_indices has no layers".into(),
            ));
        }

        // Layer 0 = terrain.
        let layer0 = match &block_indices_outer[0] {
            NbtValue::List(items) => items,
            _ => {
                return Err(McStructureError::Missing(
                    "block_indices[0] must be a list".into(),
                ))
            }
        };

        if layer0.len() != volume {
            return Err(McStructureError::Nbt(format!(
                "block_indices[0] length {} != volume {volume}",
                layer0.len()
            )));
        }

        let indices: Vec<i32> = layer0
            .iter()
            .map(|v| match v {
                NbtValue::Int(i) => *i,
                _ => -1,
            })
            .collect();

        Ok(Self {
            size_x,
            size_y,
            size_z,
            palette,
            indices,
        })
    }

    /// Convert the structure to a `Vec<Chunk>` compatible with the rest of the
    /// pipeline.
    ///
    /// The structure is mapped to world coordinates with its lower-west-south
    /// corner at the origin `(0, 0, 0)`. Sections of 16 blocks are created as
    /// needed. Blocks outside the palette or with index -1 are treated as air.
    pub fn to_chunks(&self) -> Vec<Chunk> {
        let is_air_idx = |idx: i32| -> bool {
            if idx < 0 {
                return true;
            }
            match self.palette.get(idx as usize) {
                None => true,
                Some(name) => {
                    let short = name.rsplit(':').next().unwrap_or(name);
                    matches!(short, "air" | "cave_air" | "void_air")
                }
            }
        };

        let max_cx = (self.size_x - 1).div_euclid(16);
        let max_cz = (self.size_z - 1).div_euclid(16);
        let max_section_y = (self.size_y - 1).div_euclid(16);

        let mut chunks: Vec<Chunk> = Vec::new();

        for cx in 0..=max_cx {
            for cz in 0..=max_cz {
                let mut sections: Vec<SectionData> = Vec::new();
                for sy in 0..=max_section_y {
                    let mut palette_map: HashMap<String, usize> = HashMap::new();
                    let mut section_palette: Vec<crate::chunk::BlockState> =
                        vec![crate::chunk::BlockState::new("minecraft:air".to_owned())];
                    let mut section_indices = vec![0u16; 4096];
                    let mut has_blocks = false;

                    palette_map.insert("minecraft:air".to_owned(), 0);

                    for local_y in 0..16usize {
                        let wy = sy * 16 + local_y as i32;
                        if wy >= self.size_y {
                            continue;
                        }
                        for local_z in 0..16usize {
                            let wz = cz * 16 + local_z as i32;
                            if wz >= self.size_z {
                                continue;
                            }
                            for local_x in 0..16usize {
                                let wx = cx * 16 + local_x as i32;
                                if wx >= self.size_x {
                                    continue;
                                }
                                // Flat index: wy * size_z * size_x + wz * size_x + wx
                                let flat = (wy * self.size_z * self.size_x + wz * self.size_x + wx)
                                    as usize;
                                let palette_idx = self.indices.get(flat).copied().unwrap_or(-1);
                                if is_air_idx(palette_idx) {
                                    continue;
                                }
                                has_blocks = true;
                                let name = self
                                    .palette
                                    .get(palette_idx as usize)
                                    .cloned()
                                    .unwrap_or_else(|| "minecraft:air".to_owned());

                                let entry = if let Some(&id) = palette_map.get(&name) {
                                    id
                                } else {
                                    let id = section_palette.len();
                                    section_palette
                                        .push(crate::chunk::BlockState::new(name.clone()));
                                    palette_map.insert(name, id);
                                    id
                                };

                                // Section index layout: y<<8 | z<<4 | x
                                let section_pos = (local_y << 8) | (local_z << 4) | local_x;
                                section_indices[section_pos] = entry as u16;
                            }
                        }
                    }

                    if has_blocks {
                        sections.push(SectionData {
                            y: sy as i8,
                            palette: section_palette,
                            indices: section_indices,
                        });
                    }
                }

                if !sections.is_empty() {
                    chunks.push(Chunk::from_sections(cx, cz, sections));
                }
            }
        }

        chunks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_chunks_maps_blocks_correctly() {
        // 4×4×4 structure, all stone except corner (0,0,0).
        let mut indices = vec![1i32; 4 * 4 * 4];
        // flat(0,0,0) = 0*4*4 + 0*4 + 0 = 0
        indices[0] = 0; // air at (0,0,0)

        let mc = McStructure {
            size_x: 4,
            size_y: 4,
            size_z: 4,
            palette: vec!["minecraft:air".to_owned(), "minecraft:stone".to_owned()],
            indices,
        };

        let chunks = mc.to_chunks();
        assert!(!chunks.is_empty(), "should have at least one chunk");

        let chunk = &chunks[0];
        // (0,0,0) should be air (palette index 0).
        let block_000 = chunk.block_at(0, 0, 0).unwrap_or("minecraft:air");
        assert!(
            block_000.contains("air"),
            "corner (0,0,0) should be air, got: {block_000}"
        );
        // (1,0,0) should be stone.
        let block_100 = chunk.block_at(1, 0, 0).unwrap_or("");
        assert!(
            block_100.contains("stone"),
            "block (1,0,0) should be stone, got: {block_100}"
        );
    }

    #[test]
    fn empty_palette_gives_no_chunks() {
        let mc = McStructure {
            size_x: 4,
            size_y: 4,
            size_z: 4,
            palette: vec!["minecraft:air".to_owned()],
            indices: vec![0i32; 4 * 4 * 4], // all air
        };
        let chunks = mc.to_chunks();
        // All air → no sections → no chunks.
        assert!(
            chunks.is_empty(),
            "all-air structure should produce no chunks"
        );
    }
}
