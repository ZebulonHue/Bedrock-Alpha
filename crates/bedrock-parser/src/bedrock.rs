//! Bedrock Edition world reading: LevelDB storage plus SubChunkPrefix
//! payload decoding (palette versions 8 and 9, the formats all current
//! worlds use).
//!
//! Key layout and payload format follow the public Bedrock level format
//! documentation (see <https://minecraft.wiki/w/Bedrock_Edition_level_format>).

use crate::chunk::{Chunk, SectionData};
use crate::nbt_le::{NbtCursor, NbtValue};
use bedrock_leveldb::{BedrockKey, ChunkKey, ChunkRecordTag, Db, OpenOptions, ReadOptions};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

/// Subchunk payload versions we can decode.
const STORAGE_V8: u8 = 8;
const STORAGE_V9: u8 = 9;

/// Why reading a Bedrock world failed.
#[derive(Debug)]
pub enum BedrockError {
    /// The LevelDB database could not be read.
    Db(bedrock_leveldb::LevelDbError),
    /// A subchunk payload used an unsupported format.
    Unsupported(String),
    /// A payload was malformed.
    Malformed(String),
}

impl fmt::Display for BedrockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BedrockError::Db(err) => write!(f, "LevelDB error: {err}"),
            BedrockError::Unsupported(what) => write!(f, "unsupported Bedrock data: {what}"),
            BedrockError::Malformed(what) => write!(f, "malformed Bedrock data: {what}"),
        }
    }
}

impl std::error::Error for BedrockError {}

impl From<bedrock_leveldb::LevelDbError> for BedrockError {
    fn from(err: bedrock_leveldb::LevelDbError) -> Self {
        BedrockError::Db(err)
    }
}

/// A Bedrock world on disk: its LevelDB handle plus the world folder.
pub struct BedrockWorld {
    db: Db,
    folder: PathBuf,
}

impl BedrockWorld {
    /// Open the world's `db` folder read-only.
    pub fn open(folder: impl Into<PathBuf>) -> Result<Self, BedrockError> {
        let folder = folder.into();
        let db = Db::open(
            folder.join("db"),
            OpenOptions {
                read_only: true,
                create_if_missing: false,
                ..OpenOptions::default()
            },
        )?;
        Ok(Self { db, folder })
    }

    /// The world folder on disk.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The local player's last position `(x, y, z)`, if stored.
    pub fn player_pos(&self) -> Option<[f64; 3]> {
        let value = self.db.get(b"~local_player").ok().flatten()?;
        let mut cursor = NbtCursor::new(&value);
        let root = cursor.read_root().ok()?;
        let compound = root.as_compound()?;
        let pos = compound.get("Pos")?;
        let NbtValue::List(items) = pos else {
            return None;
        };
        let mut coords = [0.0f64; 3];
        for (i, item) in items.iter().take(3).enumerate() {
            coords[i] = match item {
                NbtValue::Float(v) => f64::from(*v),
                NbtValue::Double(v) => *v,
                _ => return None,
            };
        }
        Some(coords)
    }

    /// Decode every overworld chunk within `radius` chunks of `(cx, cz)`.
    /// Chunks that fail to decode are logged and skipped.
    pub fn chunks_near(&self, cx: i32, cz: i32, radius: i32) -> Result<Vec<Chunk>, BedrockError> {
        // Index SubChunkPrefix records per chunk (keys are cheap to scan).
        let mut subchunks: BTreeMap<(i32, i32), Vec<(i8, bytes::Bytes)>> = BTreeMap::new();
        let mut keys: Vec<(ChunkKey, bytes::Bytes)> = Vec::new();
        for key in self.db.collect_keys_owned(ReadOptions::default())? {
            if let BedrockKey::Chunk(chunk_key) = BedrockKey::parse(&key) {
                if chunk_key.tag == ChunkRecordTag::SubChunkPrefix {
                    keys.push((chunk_key, key));
                }
            }
        }
        let wanted: Vec<(ChunkKey, bytes::Bytes)> = keys
            .into_iter()
            .filter(|(ck, _)| {
                (ck.coordinates.x - cx).abs() <= radius && (ck.coordinates.z - cz).abs() <= radius
            })
            .collect();
        let raw_keys: Vec<bytes::Bytes> = wanted.iter().map(|(_, k)| k.clone()).collect();
        let values = self.db.get_many_owned(raw_keys, ReadOptions::default())?;
        for ((chunk_key, _), value) in wanted.iter().zip(values) {
            let Some(value) = value else { continue };
            let Some(sub_y) = chunk_key.subchunk.map(|s| s.raw()) else {
                continue;
            };
            subchunks
                .entry((chunk_key.coordinates.x, chunk_key.coordinates.z))
                .or_default()
                .push((sub_y, value));
        }

        let mut chunks = Vec::new();
        for ((x, z), mut records) in subchunks {
            records.sort_by_key(|(y, _)| *y);
            let mut sections = Vec::new();
            for (sub_y, value) in records {
                match decode_subchunk(&value, sub_y) {
                    Ok(section) => sections.push(section),
                    Err(err) => {
                        tracing::warn!("Skipping subchunk ({x}, {sub_y}, {z}): {err}");
                    }
                }
            }
            if !sections.is_empty() {
                chunks.push(Chunk::from_sections(x, z, sections));
            }
        }
        Ok(chunks)
    }
}

/// Decode one SubChunkPrefix payload (v8/v9) into a section.
fn decode_subchunk(bytes: &[u8], key_sub_y: i8) -> Result<SectionData, BedrockError> {
    if bytes.len() < 2 {
        return Err(BedrockError::Malformed("subchunk payload too short".into()));
    }
    let version = bytes[0];
    if version != STORAGE_V8 && version != STORAGE_V9 {
        return Err(BedrockError::Unsupported(format!(
            "subchunk storage version {version}"
        )));
    }
    let layers = bytes[1] as usize;
    let mut offset = 2;
    // v9 repeats the subchunk Y in the payload; v8 relies on the key.
    if version == STORAGE_V9 {
        offset += 1;
    }

    // Only the first layer holds terrain (extra layers are waterlogged
    // blocks); decode layer 0 and skip the rest.
    let mut result = None;
    for layer in 0..layers {
        let (palette, indices, consumed) = decode_block_storage(&bytes[offset..])?;
        offset += consumed;
        if layer == 0 {
            result = Some((palette, indices));
        }
    }
    let (palette, indices) =
        result.ok_or_else(|| BedrockError::Malformed("subchunk has no layers".into()))?;
    Ok(SectionData {
        y: key_sub_y,
        palette,
        indices,
    })
}

/// Decode one paletted block storage: header byte, bit-packed indices, then
/// an LE NBT palette. Returns `(palette, indices, bytes_consumed)`.
fn decode_block_storage(
    bytes: &[u8],
) -> Result<(Vec<crate::chunk::BlockState>, Vec<u16>, usize), BedrockError> {
    let Some((&header, mut rest)) = bytes.split_first() else {
        return Err(BedrockError::Malformed("empty block storage".into()));
    };
    let mut consumed = 1;
    let bits = (header >> 1) as usize;
    if header & 1 == 1 {
        return Err(BedrockError::Unsupported(
            "runtime-id palettes (need the game's runtime table)".into(),
        ));
    }
    if bits == 0 {
        // Uniform subchunk: no index array, palette follows immediately.
    } else {
        if !(1..=16).contains(&bits) {
            return Err(BedrockError::Unsupported(format!("{bits} bits per block")));
        }
        let per_word = 32 / bits;
        let words = 4096_usize.div_ceil(per_word);
        let byte_len = words * 4;
        if rest.len() < byte_len {
            return Err(BedrockError::Malformed("truncated block indices".into()));
        }
        rest = &rest[byte_len..];
        consumed += byte_len;
    }

    // Palette: u32 LE count, then that many consecutive LE NBT roots.
    if rest.len() < 4 {
        return Err(BedrockError::Malformed("truncated palette".into()));
    }
    let count = u32::from_le_bytes(rest[0..4].try_into().expect("4 bytes")) as usize;
    let mut cursor = NbtCursor::new(&rest[4..]);
    let mut palette = Vec::with_capacity(count);
    for _ in 0..count {
        let entry = cursor
            .read_root()
            .map_err(|err| BedrockError::Malformed(err.to_string()))?;
        let compound = entry
            .as_compound()
            .ok_or_else(|| BedrockError::Malformed("palette entry not compound".into()))?;
        let name = compound
            .get("name")
            .and_then(NbtValue::as_str)
            .ok_or_else(|| BedrockError::Malformed("palette entry has no name".into()))?;

        let mut properties = std::collections::BTreeMap::new();
        if let Some(NbtValue::Compound(states)) = compound.get("states") {
            for (k, v) in states {
                let val_str = match v {
                    NbtValue::Byte(b) => b.to_string(),
                    NbtValue::Short(s) => s.to_string(),
                    NbtValue::Int(i) => i.to_string(),
                    NbtValue::Long(l) => l.to_string(),
                    NbtValue::String(s) => s.clone(),
                    _ => continue,
                };
                properties.insert(k.clone(), val_str);
            }
        }

        let mut bedrock_state_str = format!("{name}[");
        let mut first = true;
        for (k, v) in &properties {
            if !first {
                bedrock_state_str.push(',');
            }
            bedrock_state_str.push_str(&format!("{k}={v}"));
            first = false;
        }
        bedrock_state_str.push(']');

        let map = get_b2j_map();
        let java_state_str = map
            .get(&bedrock_state_str)
            .map(|s| s.as_str())
            .unwrap_or(&bedrock_state_str);

        let java_name;
        let mut java_props = std::collections::HashMap::new();

        if let Some(bracket_idx) = java_state_str.find('[') {
            java_name = java_state_str[0..bracket_idx].to_string();
            let props_str = &java_state_str[bracket_idx + 1..java_state_str.len() - 1];
            if !props_str.is_empty() {
                for pair in props_str.split(',') {
                    let parts: Vec<&str> = pair.split('=').collect();
                    if parts.len() == 2 {
                        java_props.insert(parts[0].to_string(), parts[1].to_string());
                    }
                }
            }
        } else {
            java_name = java_state_str.to_string();
        }

        palette.push(crate::chunk::BlockState {
            name: java_name,
            properties: java_props,
        });
    }

    consumed += 4 + cursor.position();

    let indices = if bits == 0 {
        Vec::new()
    } else {
        unpack_words(bytes, bits)
    };

    Ok((palette, indices, consumed))
}

static B2J_MAP: std::sync::OnceLock<std::collections::HashMap<String, String>> =
    std::sync::OnceLock::new();

fn get_b2j_map() -> &'static std::collections::HashMap<String, String> {
    B2J_MAP.get_or_init(|| {
        let json_str =
            include_str!("../data/bedrock_blocks_b2j.json");
        serde_json::from_str(json_str).unwrap_or_default()
    })
}

/// Unpack the 4096 bit-packed palette indices of a block storage
/// (`bits` must be non-zero).
///
/// Bedrock stores blocks in XZY order (y fastest): file index
/// `i = x*256 + z*16 + y`. Our canonical section layout is
/// `y*256 + z*16 + x`, so each value is written to its permuted slot.
fn unpack_words(bytes: &[u8], bits: usize) -> Vec<u16> {
    let per_word = 32 / bits;
    let words = 4096_usize.div_ceil(per_word);
    let mask = (1u32 << bits) - 1;
    let mut indices = vec![0u16; 4096];
    let mut file_index = 0usize;
    'words: for w in 0..words {
        let start = 1 + w * 4;
        let word = u32::from_le_bytes(bytes[start..start + 4].try_into().expect("4 bytes"));
        for slot in 0..per_word {
            if file_index == 4096 {
                break 'words;
            }
            let y = file_index & 15;
            let z = (file_index >> 4) & 15;
            let x = file_index >> 8;
            indices[(y << 8) | (z << 4) | x] = ((word >> (slot * bits)) & mask) as u16;
            file_index += 1;
        }
    }
    indices
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a v8 subchunk payload with a 2-entry palette (air, stone),
    /// 4 bits per block, stone at file index 306 (= x=1, y=2, z=3 in the
    /// file's XZY order: `1*256 + 3*16 + 2`).
    fn sample_subchunk() -> Vec<u8> {
        let mut payload = vec![STORAGE_V8, 1]; // version, layers
        payload.push(4 << 1); // 4 bits per block, paletted
                              // 4096 indices at 4 bits = 512 u32 words, 8 indices per word;
                              // file index 306 lives in word 38, slot 2.
        for w in 0..512u32 {
            let word = if w == 38 { 1u32 << (2 * 4) } else { 0 };
            payload.extend_from_slice(&word.to_le_bytes());
        }
        payload.extend_from_slice(&2u32.to_le_bytes()); // palette count
        for name in ["minecraft:air", "minecraft:stone"] {
            payload.push(10); // TAG_Compound, empty name
            payload.extend_from_slice(&0u16.to_le_bytes());
            payload.push(8); // TAG_String "name"
            payload.extend_from_slice(&4u16.to_le_bytes());
            payload.extend_from_slice(b"name");
            payload.extend_from_slice(&(name.len() as u16).to_le_bytes());
            payload.extend_from_slice(name.as_bytes());
            payload.push(0); // TAG_End
        }
        payload
    }

    #[test]
    fn decodes_a_v8_subchunk() {
        let section = decode_subchunk(&sample_subchunk(), -4).unwrap();
        assert_eq!(section.y, -4);
        assert_eq!(section.palette.len(), 2);
        assert_eq!(section.palette[0].name, "minecraft:air");
        assert_eq!(section.palette[1].name, "minecraft:stone");
        assert_eq!(section.indices.len(), 4096);
        // XZY file order is permuted into the canonical y<<8|z<<4|x layout.
        assert_eq!(section.indices[(2 << 8) | (3 << 4) | 1], 1);
        assert_eq!(section.indices[306], 0);
    }

    #[test]
    fn rejects_unknown_storage_version() {
        assert!(matches!(
            decode_subchunk(&[7, 1, 0], 0),
            Err(BedrockError::Unsupported(_))
        ));
    }
}
