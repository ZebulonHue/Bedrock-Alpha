//! Chunk decoding: turns chunk NBT into an addressable block grid.
//!
//! Supports both the modern layout (1.18+, `sections` with `block_states`)
//! and the legacy one (`Level.Sections` with `Palette`/`BlockStates`).

use fastnbt::Value;
use std::collections::HashMap;
use std::fmt;

/// Blocks per section edge (sections are 16×16×16).
const SECTION_EDGE: usize = 16;
/// Blocks per section.
const SECTION_VOLUME: usize = SECTION_EDGE * SECTION_EDGE * SECTION_EDGE;

/// A fully decoded block state: namespaced block ID plus optional key-value properties.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BlockState {
    /// Namespaced block ID (`minecraft:oak_stairs`).
    pub name: String,
    /// Property key-value pairs (`facing -> north`).
    pub properties: HashMap<String, String>,
}

impl BlockState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            properties: HashMap::new(),
        }
    }

    /// Short block ID without namespace (`oak_stairs`).
    pub fn short_name(&self) -> &str {
        strip_namespace(&self.name)
    }

    pub fn cache_key(&self) -> String {
        let mut keys: Vec<_> = self.properties.keys().collect();
        keys.sort_unstable();
        let mut s = self.name.clone();
        for k in keys {
            s.push('|');
            s.push_str(k);
            s.push('=');
            s.push_str(&self.properties[k]);
        }
        s
    }

    /// Facing direction property (`north`, `south`, `east`, `west`). Default: `south`.
    pub fn facing(&self) -> &str {
        self.properties
            .get("facing")
            .map(String::as_str)
            .unwrap_or("south")
    }

    /// Vertical half property (`top`, `bottom`). Default: `bottom`.
    pub fn half(&self) -> &str {
        self.properties
            .get("half")
            .or_else(|| self.properties.get("type"))
            .map(String::as_str)
            .unwrap_or("bottom")
    }

    /// Log/pillar axis property (`x`, `y`, `z`). Default: `y`.
    pub fn axis(&self) -> &str {
        self.properties
            .get("axis")
            .map(String::as_str)
            .unwrap_or("y")
    }

    /// Bed half property (`head`, `foot`). Default: `foot`.
    ///
    /// Matches vanilla's own default (a bed placed with no explicit `part`
    /// property, e.g. in a hand-built test fixture, is the foot half).
    pub fn part(&self) -> &str {
        self.properties
            .get("part")
            .map(String::as_str)
            .unwrap_or("foot")
    }

    /// Dye colour property (`white`, `red`, `light_blue`, ...). Default:
    /// `white`, matching the legacy Bedrock data value of 0.
    ///
    /// Pre-flattening block IDs (`wool`, `carpet`, `concretepowder`,
    /// `stained_hardened_clay`, `glazedterracotta`, `stained_glass`,
    /// `stained_glass_pane`) all share one block *name* across their 16 dye
    /// colours and carry the actual colour in this separate state instead.
    pub fn color(&self) -> &str {
        self.properties
            .get("color")
            .map(String::as_str)
            .unwrap_or("white")
    }

    /// Texture-lookup key: the block name, plus its colour when the block
    /// carries a `color` property.
    ///
    /// `name` alone is not a unique texture key — two `minecraft:wool`
    /// palette entries with different `color` values must resolve to
    /// different atlas swatches (red wool vs. blue wool), so callers that
    /// build a per-block-name UV map need this instead of `name` to avoid
    /// collapsing every colour onto whichever one was inserted first.
    pub fn texture_key(&self) -> String {
        match self.properties.get("color") {
            Some(color) => format!("{}|{color}", self.name),
            None => self.name.clone(),
        }
    }
}

/// One 16×16×16 chunk section: a palette plus a palette index per block.
#[derive(Clone)]
struct Section {
    /// Section Y coordinate (world Y = `y * 16`).
    y: i8,
    /// Namespaced block states (`minecraft:oak_stairs`).
    palette: Vec<BlockState>,
    /// 4096 palette indices in `y << 8 | z << 4 | x` order. Empty when the
    /// palette has a single entry (every block is that entry).
    indices: Vec<u16>,
}

/// A decoded chunk column (16 blocks × world height × 16 blocks).
#[derive(Clone)]
pub struct Chunk {
    /// Chunk X coordinate.
    pub x: i32,
    /// Chunk Z coordinate.
    pub z: i32,
    sections: Vec<Section>,
}

/// Section data for [`Chunk::from_sections`] (used by the Bedrock reader,
/// which decodes subchunks outside the NBT chunk format).
pub struct SectionData {
    /// Section Y coordinate (world Y = `y * 16`).
    pub y: i8,
    /// Namespaced block states (`minecraft:stone`).
    pub palette: Vec<BlockState>,
    /// 4096 palette indices in `y << 8 | z << 4 | x` order (empty when the
    /// palette has a single entry).
    pub indices: Vec<u16>,
}

/// Why decoding a chunk failed.
#[derive(Debug)]
pub enum ChunkError {
    /// The NBT payload was malformed.
    Nbt(fastnbt::error::Error),
    /// An expected tag was missing or had the wrong type.
    Missing(String),
}

impl fmt::Display for ChunkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ChunkError::Nbt(err) => write!(f, "invalid NBT: {err}"),
            ChunkError::Missing(tag) => write!(f, "missing or invalid tag: {tag}"),
        }
    }
}

impl std::error::Error for ChunkError {}

impl From<fastnbt::error::Error> for ChunkError {
    fn from(err: fastnbt::error::Error) -> Self {
        ChunkError::Nbt(err)
    }
}

impl Chunk {
    /// Build a chunk directly from decoded sections (Bedrock subchunks).
    pub fn from_sections(x: i32, z: i32, sections: Vec<SectionData>) -> Self {
        let sections = sections
            .into_iter()
            .map(|s| Section {
                y: s.y,
                palette: s.palette,
                indices: s.indices,
            })
            .collect();
        Self { x, z, sections }
    }

    /// Decode a chunk from decompressed NBT bytes.
    pub fn from_nbt(bytes: &[u8]) -> Result<Self, ChunkError> {
        let root = fastnbt::from_bytes::<Value>(bytes)?;
        let root = as_compound(&root)?;

        // Modern chunks store xPos/zPos/sections at the root; legacy chunks
        // nest them under "Level" (which also uses "Sections"/"Palette").
        let container: &HashMap<String, Value> = match compound_get(root, "Level") {
            Some(level) => level,
            None => root,
        };
        let x = get_int(container, "xPos")?;
        let z = get_int(container, "zPos")?;
        let sections_value = container
            .get("sections")
            .or_else(|| container.get("Sections"))
            .ok_or(ChunkError::Missing("sections".into()))?;
        let sections_list = as_list(sections_value)?;

        let mut sections = Vec::new();
        for section_value in sections_list {
            if let Some(section) = parse_section(section_value)? {
                sections.push(section);
            }
        }
        Ok(Self { x, z, sections })
    }

    /// World-Y range covered by this chunk's sections, `(min_y, max_y)`.
    /// `None` when the chunk has no sections with block data.
    pub fn y_range(&self) -> Option<(i32, i32)> {
        let mut ys = self.sections.iter().map(|s| s.y as i32);
        let first = ys.next()?;
        let (min, max) = ys.fold((first, first), |(lo, hi), y| (lo.min(y), hi.max(y)));
        Some((min * SECTION_EDGE as i32, (max + 1) * SECTION_EDGE as i32))
    }

    /// Every distinct block id in this chunk's palettes (air included).
    pub fn block_names(&self) -> Vec<&str> {
        let mut names: Vec<&str> = Vec::new();
        for section in &self.sections {
            for state in &section.palette {
                if !names.contains(&state.name.as_str()) {
                    names.push(&state.name);
                }
            }
        }
        names
    }

    /// Every distinct [`BlockState::texture_key`] in this chunk's palettes
    /// (air included).
    ///
    /// Unlike [`Self::block_names`], this distinguishes colour variants of
    /// legacy blocks — `minecraft:wool` with `color=red` and
    /// `minecraft:wool` with `color=blue` yield two different keys, since
    /// they need two different atlas swatches. Texture-atlas builders
    /// (e.g. `build_mineways_tileset`) should collect keys via this method,
    /// not `block_names`, or every colour of wool/concrete/carpet/glass in
    /// the world collapses onto whichever colour happened to load first.
    pub fn texture_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = Vec::new();
        for section in &self.sections {
            for state in &section.palette {
                let key = state.texture_key();
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
        keys
    }

    /// BlockState at local `(x, z)` (0..16) and world `y`.
    pub fn block_state_at(&self, x: usize, y: i32, z: usize) -> Option<&BlockState> {
        debug_assert!(x < SECTION_EDGE && z < SECTION_EDGE);
        let section_y = y.div_euclid(SECTION_EDGE as i32) as i8;
        let section = self.sections.iter().find(|s| s.y == section_y)?;
        if section.palette.is_empty() {
            return None;
        }
        let palette_index = if section.indices.is_empty() {
            0
        } else {
            let local_y = y.rem_euclid(SECTION_EDGE as i32) as usize;
            let block_index = (local_y << 8) | (z << 4) | x;
            *section.indices.get(block_index)? as usize
        };
        section.palette.get(palette_index)
    }

    /// Namespaced block id at local `(x, z)` (0..16) and world `y`,
    /// or `None` when outside the world's height.
    pub fn block_at(&self, x: usize, y: i32, z: usize) -> Option<&str> {
        self.block_state_at(x, y, z).map(|s| s.name.as_str())
    }
}

/// Strip the `minecraft:` namespace from a block id.
pub fn strip_namespace(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Rough opacity classification used for face culling. Unknown blocks are
/// treated as opaque — culling stays conservative and correct-looking.
pub fn is_opaque_cube(name: &str) -> bool {
    !matches!(
        strip_namespace(name),
        "air"
            | "cave_air"
            | "void_air"
            | "water"
            | "lava"
            | "glass"
            | "glass_pane"
            | "tinted_glass"
            | "grass"
            | "short_grass"
            | "tall_grass"
            | "fern"
            | "large_fern"
            | "oak_leaves"
            | "spruce_leaves"
            | "birch_leaves"
            | "jungle_leaves"
            | "acacia_leaves"
            | "dark_oak_leaves"
            | "mangrove_leaves"
            | "cherry_leaves"
            | "azalea_leaves"
            | "flowering_azalea_leaves"
            | "torch"
            | "wall_torch"
            | "soul_torch"
            | "soul_wall_torch"
            | "lantern"
            | "soul_lantern"
            | "redstone_torch"
            | "redstone_wall_torch"
            | "dandelion"
            | "poppy"
            | "blue_orchid"
            | "allium"
            | "azure_bluet"
            | "oxeye_daisy"
            | "cornflower"
            | "lily_of_the_valley"
            | "red_tulip"
            | "orange_tulip"
            | "white_tulip"
            | "pink_tulip"
            | "sunflower"
            | "lilac"
            | "rose_bush"
            | "peony"
            | "snow"
            | "vine"
            | "ladder"
            | "rail"
            | "powered_rail"
            | "detector_rail"
            | "activator_rail"
            | "lever"
            | "tripwire"
            | "fire"
            | "soul_fire"
            | "cobweb"
            | "dead_bush"
            | "seagrass"
    )
}

/// Parse one section entry, or `None` for sections with no block data.
fn parse_section(value: &Value) -> Result<Option<Section>, ChunkError> {
    let compound = as_compound(value)?;
    let y = match compound.get("Y") {
        Some(Value::Byte(y)) => *y,
        _ => return Ok(None),
    };

    // Modern: block_states { palette, data }. Legacy: Palette / BlockStates.
    let is_legacy = compound_get(compound, "block_states").is_none();
    let (palette_value, data_value) = if !is_legacy {
        let block_states = compound_get(compound, "block_states").unwrap();
        (block_states.get("palette"), block_states.get("data"))
    } else {
        (compound.get("Palette"), compound.get("BlockStates"))
    };
    let Some(palette_value) = palette_value else {
        return Ok(None); // no block data at all — treat as empty
    };

    let mut palette = Vec::new();
    for entry in as_list(palette_value)? {
        let entry = as_compound(entry)?;
        let name = match entry.get("Name") {
            Some(Value::String(name)) => name.clone(),
            _ => return Err(ChunkError::Missing("palette Name".into())),
        };
        let mut properties = HashMap::new();
        if let Some(Value::Compound(props)) = entry.get("Properties") {
            for (k, v) in props {
                if let Value::String(s) = v {
                    properties.insert(k.clone(), s.clone());
                }
            }
        }
        palette.push(BlockState { name, properties });
    }

    let indices = match data_value {
        Some(value) => unpack_indices(&as_long_array(value)?, palette.len(), is_legacy),
        None => Vec::new(), // single-entry palette: no data array
    };
    Ok(Some(Section {
        y,
        palette,
        indices,
    }))
}

/// Unpack bit-packed palette indices. Modern chunks (1.16+) never span
/// longs. Legacy chunks (pre-1.16) bit-pack continuously across longs.
fn unpack_indices(data: &[i64], palette_len: usize, spans_longs: bool) -> Vec<u16> {
    if palette_len <= 1 {
        return Vec::new();
    }
    let bits = (usize::BITS - (palette_len - 1).leading_zeros()).max(4) as usize;
    let mask = (1u64 << bits) - 1;
    let mut out = Vec::with_capacity(SECTION_VOLUME);

    if spans_longs {
        let mut bit_index = 0;
        let mut current_long_idx = 0;

        while out.len() < SECTION_VOLUME && current_long_idx < data.len() {
            let bit_offset = bit_index % 64;
            let mut val = (data[current_long_idx] as u64) >> bit_offset;

            let end_bit_offset = bit_offset + bits;
            if end_bit_offset > 64 && current_long_idx + 1 < data.len() {
                val |= (data[current_long_idx + 1] as u64) << (64 - bit_offset);
            }

            out.push((val & mask) as u16);
            bit_index += bits;
            current_long_idx = bit_index / 64;
        }
    } else {
        let per_long = 64 / bits;
        for &long in data {
            let packed = long as u64;
            for i in 0..per_long {
                if out.len() == SECTION_VOLUME {
                    return out;
                }
                out.push(((packed >> (i * bits)) & mask) as u16);
            }
        }
    }
    out
}

fn as_compound(value: &Value) -> Result<&HashMap<String, Value>, ChunkError> {
    match value {
        Value::Compound(compound) => Ok(compound),
        _ => Err(ChunkError::Missing("compound".into())),
    }
}

fn compound_get<'a>(
    compound: &'a HashMap<String, Value>,
    key: &str,
) -> Option<&'a HashMap<String, Value>> {
    match compound.get(key) {
        Some(Value::Compound(inner)) => Some(inner),
        _ => None,
    }
}

fn as_list(value: &Value) -> Result<&[Value], ChunkError> {
    match value {
        Value::List(list) => Ok(list),
        _ => Err(ChunkError::Missing("list".into())),
    }
}

fn as_long_array(value: &Value) -> Result<Vec<i64>, ChunkError> {
    match value {
        Value::LongArray(array) => Ok(array.iter().copied().collect()),
        _ => Err(ChunkError::Missing("long array".into())),
    }
}

fn get_int(compound: &HashMap<String, Value>, key: &str) -> Result<i32, ChunkError> {
    match compound.get(key) {
        Some(Value::Int(value)) => Ok(*value),
        _ => Err(ChunkError::Missing(key.to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Serialize;

    #[derive(Serialize)]
    struct TestChunk {
        sections: Vec<TestSection>,
        #[serde(rename = "xPos")]
        x: i32,
        #[serde(rename = "zPos")]
        z: i32,
    }

    #[derive(Serialize)]
    struct TestSection {
        #[serde(rename = "Y")]
        y: i8,
        block_states: TestBlockStates,
    }

    #[derive(Serialize)]
    struct TestBlockStates {
        palette: Vec<TestPaletteEntry>,
        data: fastnbt::LongArray,
    }

    #[derive(Serialize)]
    struct TestPaletteEntry {
        #[serde(rename = "Name")]
        name: String,
    }

    /// Pack indices with the same 1.16+ scheme the game uses.
    fn pack(indices: &[u16], bits: usize) -> Vec<i64> {
        let per_long = 64 / bits;
        let mut out = vec![0i64; indices.len().div_ceil(per_long)];
        for (i, &index) in indices.iter().enumerate() {
            let slot = i / per_long;
            let shift = (i % per_long) * bits;
            out[slot] |= (index as i64) << shift;
        }
        out
    }

    /// Block index in y << 8 | z << 4 | x order.
    fn at(x: usize, y: usize, z: usize) -> usize {
        (y << 8) | (z << 4) | x
    }

    fn test_chunk_bytes() -> Vec<u8> {
        // Three-entry palette → 4 bits per index.
        let mut indices = vec![0u16; SECTION_VOLUME];
        indices[at(3, 0, 5)] = 1; // stone at (3, 0, 5)
        indices[at(0, 1, 0)] = 2; // dirt at (0, 1, 0)
        fastnbt::to_bytes(&TestChunk {
            sections: vec![TestSection {
                y: 0,
                block_states: TestBlockStates {
                    palette: vec![
                        TestPaletteEntry {
                            name: "minecraft:air".into(),
                        },
                        TestPaletteEntry {
                            name: "minecraft:stone".into(),
                        },
                        TestPaletteEntry {
                            name: "minecraft:dirt".into(),
                        },
                    ],
                    data: fastnbt::LongArray::new(pack(&indices, 4)),
                },
            }],
            x: 4,
            z: -2,
        })
        .unwrap()
    }

    #[test]
    fn decodes_modern_chunk_layout() {
        let chunk = Chunk::from_nbt(&test_chunk_bytes()).unwrap();
        assert_eq!((chunk.x, chunk.z), (4, -2));
        assert_eq!(chunk.block_at(3, 0, 5), Some("minecraft:stone"));
        assert_eq!(chunk.block_at(0, 1, 0), Some("minecraft:dirt"));
        assert_eq!(chunk.block_at(1, 1, 1), Some("minecraft:air"));
        assert_eq!(chunk.block_at(0, 16, 0), None, "outside decoded sections");
    }

    #[test]
    fn unpacks_without_crossing_longs() {
        // 17-entry palette → 5 bits → 12 indices per long, 4 bits unused.
        let indices: Vec<u16> = (0..24).map(|i| (i % 17) as u16).collect();
        let packed = pack(&indices, 5);
        let unpacked = unpack_indices(&packed, 17, false);
        assert_eq!(&unpacked[..24], &indices[..]);
    }

    #[test]
    fn opacity_classification() {
        assert!(!is_opaque_cube("minecraft:air"));
        assert!(!is_opaque_cube("minecraft:water"));
        assert!(!is_opaque_cube("minecraft:oak_leaves"));
        assert!(is_opaque_cube("minecraft:stone"));
        assert!(is_opaque_cube("minecraft:pluto_rock"), "unknown = opaque");
    }
}
