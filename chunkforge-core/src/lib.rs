//! # chunkforge-core
//!
//! A dependency-light Rust port of the ChunkForge webapp's Minecraft save
//! parser (`src/lib/mc/`). It reads Minecraft Java Edition **Anvil region
//! files** (`.mca`, 1.18+ chunk format), decodes chunks, culls interior
//! blocks, and returns exterior block positions grouped by block type.
//!
//! ## Quick start
//!
//! ```no_run
//! use chunkforge_core::{parse_region_paths, parse_region_bytes};
//!
//! // From file paths (merges every file into one world):
//! let world = parse_region_paths(&["r.0.0.mca", "r.0.-1.mca"])?;
//! println!("{} blocks, {} exterior", world.total_blocks, world.exterior_blocks);
//! for (name, positions) in &world.blocks_by_type {
//!     println!("{name}: {} blocks", positions.len());
//! }
//!
//! // From an in-memory buffer:
//! let bytes = std::fs::read("r.0.0.mca")?;
//! let world = parse_region_bytes(&bytes, "r.0.0.mca")?;
//! # Ok::<(), chunkforge_core::ParseError>(())
//! ```
//!
//! ## Semantics (same as the TypeScript parser)
//!
//! * Supports the 1.18+ chunk layout (`sections[]` + `block_states`); chunks
//!   in the pre-1.18 `Level.Sections` layout are counted as legacy and
//!   skipped. If every chunk is legacy, [`ParseError::Pre118Format`] is
//!   returned. LZ4-compressed chunks (compression type 4) are skipped and
//!   counted. Corrupt chunks are skipped and counted — never fatal.
//! * A block survives culling iff at least one of its 6 neighbors is absent
//!   or non-opaque per [`appearance`]. Region edges count as absent, so edge
//!   blocks are always kept. Air (`minecraft:air`, `cave_air`, `void_air`) is
//!   dropped before culling.
//! * Coordinates: Y-up, 1 block = 1 unit, positions are block **min-corners**
//!   (block center at +0.5 on each axis).
//!
//! ## Progress callbacks
//!
//! Not built in — desktop apps wire their own: read the file(s) yourself,
//! call [`parse_region_bytes`] per buffer, and merge with your own progress
//! accounting (or run the parse on a worker thread and report completion).
//!
//! See `PARSING.md` in the repository for the full format walkthrough.

mod appearance;
mod arena;
mod chunk;
mod error;
mod nbt;
mod region;
mod world;

pub use appearance::{appearance, BlockAppearance};
pub use error::{ParseError, LEGACY_FORMAT_MESSAGE};
pub use world::ParsedWorld;

use std::path::Path;

/// Parse one `.mca` buffer into an exterior-culled world.
///
/// `file_name` is only used for the [`ParsedWorld::regions`] list (and error
/// context) — pass the bare file name, e.g. `"r.0.0.mca"`.
///
/// # Errors
/// * [`ParseError::Truncated`] — buffer smaller than the 8 KiB region header.
/// * [`ParseError::Pre118Format`] — the file contains chunks and every one of
///   them uses the pre-1.18 chunk layout.
/// * [`ParseError::NoChunksDecoded`] — chunks were found but none could be
///   decoded (all corrupt or LZ4-compressed). Applied to single-buffer parses
///   too, per the port's documented judgment calls.
pub fn parse_region_bytes(buf: &[u8], file_name: &str) -> Result<ParsedWorld, ParseError> {
    let out = world::parse_region_buffer(buf, file_name)?;
    if out.chunk_count > 0 && out.decoded_chunks == 0 {
        return Err(ParseError::NoChunksDecoded);
    }
    Ok(out.world)
}

/// Parse several `.mca` files and merge them into one world.
///
/// Files are read and parsed sequentially (peak memory stays bounded to
/// roughly one region). Per-file results are merged in path order; block
/// types appear in first-seen order. A sibling file whose chunks are all
/// corrupt is tolerated as long as another file decodes chunks — the
/// [`ParseError::NoChunksDecoded`] check applies to the merged totals.
/// A file that is entirely pre-1.18 still fails with
/// [`ParseError::Pre118Format`], exactly like the TS parser.
///
/// # Errors
/// * [`ParseError::Io`] — a file cannot be read.
/// * plus the errors of [`parse_region_bytes`].
pub fn parse_region_paths<P: AsRef<Path>>(paths: &[P]) -> Result<ParsedWorld, ParseError> {
    let mut parts = Vec::with_capacity(paths.len());
    let mut chunk_count = 0u64;
    let mut decoded_chunks = 0u64;

    for path in paths {
        let path = path.as_ref();
        let buf = std::fs::read(path)?; // ParseError::Io via From
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        let out = world::parse_region_buffer(&buf, &file_name)?;
        chunk_count += out.chunk_count;
        decoded_chunks += out.decoded_chunks;
        parts.push(out.world);
    }

    if chunk_count > 0 && decoded_chunks == 0 {
        return Err(ParseError::NoChunksDecoded);
    }
    Ok(world::merge_worlds(parts))
}
