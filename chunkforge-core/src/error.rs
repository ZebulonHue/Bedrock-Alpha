//! Error type for the parser (hand-rolled `Display`/`Error`, no thiserror dep).

use std::fmt;

/// Everything that can go wrong while parsing region files.
#[derive(Debug)]
pub enum ParseError {
    /// Filesystem-level failure (only from [`crate::parse_region_paths`]).
    Io(std::io::Error),
    /// The buffer is smaller than the 8 KiB region header, or otherwise cut short.
    Truncated,
    /// NBT that failed to decode (message mirrors the TS reader's diagnostics).
    CorruptNbt(String),
    /// Every chunk across all inputs failed to decode (corrupt data or
    /// unsupported compression) — nothing could be produced.
    NoChunksDecoded,
    /// All chunks use the pre-1.18 (`Level.Sections`) layout.
    Pre118Format,
}

/// Exact message required by the TS contract (`LEGACY_FORMAT_ERROR`).
pub const LEGACY_FORMAT_MESSAGE: &str =
    "This save uses the pre-1.18 chunk format, which is not supported yet.";

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Io(e) => write!(f, "io error: {e}"),
            ParseError::Truncated => {
                write!(f, "region file too small (need at least 8192 bytes)")
            }
            ParseError::CorruptNbt(msg) => write!(f, "corrupt chunk NBT: {msg}"),
            ParseError::NoChunksDecoded => write!(
                f,
                "ChunkForge: no chunks could be decoded (corrupt data or unsupported compression)."
            ),
            ParseError::Pre118Format => write!(f, "{LEGACY_FORMAT_MESSAGE}"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ParseError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ParseError {
    fn from(e: std::io::Error) -> Self {
        ParseError::Io(e)
    }
}
