//! Minimal reader for Java Edition `level.dat` (gzip-compressed NBT).
//!
//! Only the fields the World Browser needs are extracted; full NBT access
//! for chunks arrives with the Anvil reader in Phase 3.

use serde::Deserialize;
use std::fmt;
use std::io::Read;
use std::path::Path;

/// World metadata extracted from `level.dat`.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelMeta {
    /// The world's display name.
    pub level_name: String,
    /// Last played timestamp, milliseconds since the Unix epoch.
    pub last_played_ms: i64,
    /// Numeric data version (e.g. 4189), if present.
    pub data_version: Option<i32>,
    /// The player's last known position `(x, y, z)`, if present.
    pub player_pos: Option<[f64; 3]>,
}

#[derive(Debug, Deserialize)]
struct LevelDat {
    #[serde(rename = "Data")]
    data: Data,
}

#[derive(Debug, Deserialize)]
struct Data {
    #[serde(rename = "LevelName")]
    level_name: String,
    #[serde(rename = "LastPlayed", default)]
    last_played: i64,
    #[serde(rename = "Version", default)]
    version: Option<VersionTag>,
    #[serde(rename = "Player", default)]
    player: Option<PlayerTag>,
}

#[derive(Debug, Deserialize)]
struct VersionTag {
    #[serde(rename = "Id")]
    id: i32,
}

#[derive(Debug, Deserialize)]
struct PlayerTag {
    #[serde(rename = "Pos")]
    pos: Vec<f64>,
}

/// Why reading a `level.dat` failed.
#[derive(Debug)]
pub enum LevelDatError {
    /// File could not be read.
    Io(std::io::Error),
    /// The NBT payload was malformed.
    Nbt(fastnbt::error::Error),
}

impl fmt::Display for LevelDatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LevelDatError::Io(err) => write!(f, "I/O error: {err}"),
            LevelDatError::Nbt(err) => write!(f, "invalid NBT: {err}"),
        }
    }
}

impl std::error::Error for LevelDatError {}

impl From<std::io::Error> for LevelDatError {
    fn from(err: std::io::Error) -> Self {
        LevelDatError::Io(err)
    }
}

impl From<fastnbt::error::Error> for LevelDatError {
    fn from(err: fastnbt::error::Error) -> Self {
        LevelDatError::Nbt(err)
    }
}

/// Read and decompress a Java `level.dat`, returning its metadata.
pub fn read_level_dat(path: &Path) -> Result<LevelMeta, LevelDatError> {
    let compressed = std::fs::read(path)?;
    let mut nbt = Vec::new();
    flate2::read::GzDecoder::new(compressed.as_slice()).read_to_end(&mut nbt)?;
    let level: LevelDat = fastnbt::from_bytes(&nbt)?;
    Ok(LevelMeta {
        level_name: level.data.level_name,
        last_played_ms: level.data.last_played,
        data_version: level.data.version.map(|v| v.id),
        player_pos: level
            .data
            .player
            .and_then(|p| <[f64; 3]>::try_from(p.pos).ok()),
    })
}
