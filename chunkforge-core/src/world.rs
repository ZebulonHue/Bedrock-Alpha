//! ParsedWorld, exterior culling, and output assembly — port of the
//! pipeline half of `src/lib/mc/parseCore.ts`.
//!
//! Two-phase cull: pass 1 scans the section arena and packs every exterior
//! cell as `(local_index << 16) | (name_id + 1)` into one `Vec<u32>` (~4 bytes
//! per exterior block instead of ~80 for the tuples); the big arena is then
//! dropped BEFORE pass 2 emits the pre-sized `[i32; 3]` tuples per type —
//! building tuples while the arena is still live is what used to blow peak RSS.

use crate::arena::{unkey, NameTable, SectionArena, D_SEC_X, D_SEC_Y, D_SEC_Z, SECTION_CELLS};
use crate::chunk::read_section;
use crate::error::ParseError;
use crate::nbt::{parse_nbt, Nbt};
use crate::region::{inflate_chunk, read_region_header, RegionChunkRef, COMPRESSION_LZ4};

/// A fully parsed, exterior-culled world built from one or more .mca files.
#[derive(Debug, Clone)]
pub struct ParsedWorld {
    /// block name (e.g. "minecraft:end_stone") -> `[x, y, z]` world coords
    /// (block min-corner; center at +0.5). Names appear in first-seen order.
    pub blocks_by_type: Vec<(String, Vec<[i32; 3]>)>,
    /// total non-air blocks before culling
    pub total_blocks: u64,
    /// blocks kept after exterior culling
    pub exterior_blocks: u64,
    /// (min, max) exterior block coords — `None` for an empty world
    pub bounds: Option<([i32; 3], [i32; 3])>,
    /// file names that were loaded, e.g. `["r.0.0.mca"]`
    pub regions: Vec<String>,
    /// max chunk DataVersion seen (e.g. 3465 for 1.20.1, 3839 for 1.20.6)
    pub data_version: Option<i32>,
    /// chunks skipped because they use LZ4 compression (type 4, unsupported)
    pub skipped_lz4_chunks: u64,
    /// chunks skipped because inflating/NBT parsing failed or the record was corrupt
    pub corrupt_chunks: u64,
    /// chunks skipped because they use the pre-1.18 (`Level.Sections`) layout
    pub legacy_chunks: u64,
}

impl ParsedWorld {
    fn empty() -> Self {
        ParsedWorld {
            blocks_by_type: Vec::new(),
            total_blocks: 0,
            exterior_blocks: 0,
            bounds: None,
            regions: Vec::new(),
            data_version: None,
            skipped_lz4_chunks: 0,
            corrupt_chunks: 0,
            legacy_chunks: 0,
        }
    }
}

/// Per-file parse state (TS `FileParseState`).
struct FileParseState {
    solid: SectionArena,
    names: NameTable,
    total_blocks: u64,
    skipped_legacy: u64,
    decoded_chunks: u64,
    failed_chunks: u64,
    lz4_skipped: u64,
    data_version: Option<i32>,
}

impl FileParseState {
    fn new() -> Self {
        FileParseState {
            solid: SectionArena::new(),
            names: NameTable::new(),
            total_blocks: 0,
            skipped_legacy: 0,
            decoded_chunks: 0,
            failed_chunks: 0,
            lz4_skipped: 0,
            data_version: None,
        }
    }
}

/// Result of parsing one region buffer (world + counters the caller needs
/// for cross-file fatal checks).
pub(crate) struct FileParseOutcome {
    pub world: ParsedWorld,
    pub chunk_count: u64,
    pub decoded_chunks: u64,
}

/// Inflate + decode one located chunk. Per-chunk failures are counted, never
/// thrown (a single corrupt chunk must not abort the file).
fn process_chunk(
    buf: &[u8],
    r: &RegionChunkRef,
    state: &mut FileParseState,
) -> Result<(), ParseError> {
    if r.compression == COMPRESSION_LZ4 {
        state.lz4_skipped += 1; // LZ4 is unsupported — skip, report via counter
        return Ok(());
    }
    let root: Nbt = match inflate_chunk(buf, r).and_then(|bytes| parse_nbt(&bytes)) {
        Ok(root) => root,
        Err(_) => {
            state.failed_chunks += 1;
            return Ok(());
        }
    };

    if let Some(dv) = root.get("DataVersion").and_then(Nbt::as_i32) {
        state.data_version = Some(state.data_version.map_or(dv, |cur| cur.max(dv)));
    }

    let sections = match root.get("sections").and_then(Nbt::as_list) {
        Some(s) => s,
        None => {
            // pre-1.18 layout (Level.Sections) or unknown layout — skipped, not fatal
            state.skipped_legacy += 1;
            return Ok(());
        }
    };
    state.decoded_chunks += 1;

    // Chunk coords from NBT; in-region index fallback (kept TS behavior).
    let x_pos = root
        .get("xPos")
        .and_then(Nbt::as_i32)
        .unwrap_or(r.cx as i32);
    let z_pos = root
        .get("zPos")
        .and_then(Nbt::as_i32)
        .unwrap_or(r.cz as i32);
    for section in sections {
        state.total_blocks +=
            read_section(section, x_pos, z_pos, &mut state.solid, &mut state.names)?;
    }
    Ok(())
}

/// Result of the culling pass.
struct CullResult {
    blocks_by_type: Vec<(String, Vec<[i32; 3]>)>,
    exterior_blocks: u64,
    bounds: Option<([i32; 3], [i32; 3])>,
}

/// Keep blocks with >= 1 absent or non-opaque neighbor (edges count as absent).
fn cull_world(solid: &mut SectionArena, names: &NameTable) -> CullResult {
    let n_names = names.names.len();
    let opaque = &names.opaque;
    let mut id_counts = vec![0u64; n_names];
    let mut min = [i32::MAX; 3];
    let mut max = [i32::MIN; 3];

    // A neighbor "blocks" a face only if it exists AND is an opaque cube.
    // Cell values are nameId + 1, so 0 = air/absent.
    let blocked = |v: u16| v != 0 && opaque[(v - 1) as usize];

    solid.shrink(); // exact-fit the arena before the packed buffer grows
    let arena = solid.cells();

    // Pass 1 — visibility scan.
    let mut packed: Vec<u32> = Vec::new();
    let mut sec_keys: Vec<i64> = Vec::new();
    let mut sec_ends: Vec<usize> = Vec::new(); // cumulative packed end per recorded section

    for (key, slot) in solid.entries() {
        let off = slot as usize * SECTION_CELLS;
        // Neighbor section offsets — None when the neighbor section is absent.
        let off_xm = solid.get(key - D_SEC_X).map(|s| s as usize * SECTION_CELLS);
        let off_xp = solid.get(key + D_SEC_X).map(|s| s as usize * SECTION_CELLS);
        let off_ym = solid.get(key - D_SEC_Y).map(|s| s as usize * SECTION_CELLS);
        let off_yp = solid.get(key + D_SEC_Y).map(|s| s as usize * SECTION_CELLS);
        let off_zm = solid.get(key - D_SEC_Z).map(|s| s as usize * SECTION_CELLS);
        let off_zp = solid.get(key + D_SEC_Z).map(|s| s as usize * SECTION_CELLS);

        let mut found = false;
        for i in 0..SECTION_CELLS {
            let v = arena[off + i];
            if v == 0 {
                continue;
            }
            let lx = i & 15;
            let lz = (i >> 4) & 15;
            let ly = i >> 8;
            // 6-neighbor check via pure index arithmetic. A block is visible
            // the moment any neighbor is missing or non-opaque.
            let ym = if ly > 0 {
                arena[off + i - 256]
            } else {
                off_ym.map_or(0, |o| arena[o + i + 3840])
            };
            let yp = if ly < 15 {
                arena[off + i + 256]
            } else {
                off_yp.map_or(0, |o| arena[o + i - 3840])
            };
            let zm = if lz > 0 {
                arena[off + i - 16]
            } else {
                off_zm.map_or(0, |o| arena[o + i + 240])
            };
            let zp = if lz < 15 {
                arena[off + i + 16]
            } else {
                off_zp.map_or(0, |o| arena[o + i - 240])
            };
            let xm = if lx > 0 {
                arena[off + i - 1]
            } else {
                off_xm.map_or(0, |o| arena[o + i + 15])
            };
            let xp = if lx < 15 {
                arena[off + i + 1]
            } else {
                off_xp.map_or(0, |o| arena[o + i - 15])
            };
            let visible = !(blocked(ym)
                && blocked(yp)
                && blocked(zm)
                && blocked(zp)
                && blocked(xm)
                && blocked(xp));
            if !visible {
                continue;
            }
            if !found {
                sec_keys.push(key);
                found = true;
            }
            packed.push(((i as u32) << 16) | v as u32);
            id_counts[(v - 1) as usize] += 1;
        }
        if found {
            sec_ends.push(packed.len());
        }
    }

    // Drop the section arena before the tuple vectors grow the heap.
    solid.clear();

    // Pre-allocate each type's vector at its exact size.
    let mut by_id: Vec<Option<Vec<[i32; 3]>>> = (0..n_names).map(|_| None).collect();
    for (id, &count) in id_counts.iter().enumerate() {
        if count > 0 {
            by_id[id] = Some(vec![[0i32; 3]; count as usize]);
        }
    }
    let mut cursors = vec![0usize; n_names];

    // Pass 2 — unpack into the per-type position vectors.
    let mut exterior_blocks = 0u64;
    let mut start = 0usize;
    for (s, &key) in sec_keys.iter().enumerate() {
        let (sx, sz, sy) = unkey(key);
        let base_x = sx * 16;
        let base_y = sy * 16;
        let base_z = sz * 16;

        let end = sec_ends[s];
        for &p in &packed[start..end] {
            let i = (p >> 16) as usize;
            let id = ((p & 0xffff) - 1) as usize;
            let x = base_x + (i & 15) as i32;
            let y = base_y + (i >> 8) as i32;
            let z = base_z + ((i >> 4) & 15) as i32;
            exterior_blocks += 1;
            let dst = by_id[id].as_mut().unwrap();
            dst[cursors[id]] = [x, y, z];
            cursors[id] += 1;
            min[0] = min[0].min(x);
            max[0] = max[0].max(x);
            min[1] = min[1].min(y);
            max[1] = max[1].max(y);
            min[2] = min[2].min(z);
            max[2] = max[2].max(z);
        }
        start = end;
    }

    // Names appear in intern (first-seen) order — deterministic output.
    let mut blocks_by_type = Vec::new();
    for (id, arr) in by_id.into_iter().enumerate() {
        if let Some(arr) = arr {
            blocks_by_type.push((names.names[id].clone(), arr));
        }
    }
    let bounds = if exterior_blocks > 0 {
        Some((min, max))
    } else {
        None
    };
    CullResult {
        blocks_by_type,
        exterior_blocks,
        bounds,
    }
}

/// Parse one .mca buffer into an exterior-culled world.
///
/// # Errors
/// * [`ParseError::Truncated`] — buffer smaller than the 8 KiB header.
/// * [`ParseError::Pre118Format`] — the file contains chunks and EVERY one
///   uses the pre-1.18 layout (exact TS behavior for a legacy-only region).
/// * [`ParseError::CorruptNbt`] — more than 65,534 distinct block names
///   (hostile file guard).
///
/// NOT thrown here (deferred to callers, per the port's documented judgment
/// calls): [`ParseError::NoChunksDecoded`].
pub(crate) fn parse_region_buffer(
    buf: &[u8],
    file_name: &str,
) -> Result<FileParseOutcome, ParseError> {
    let header = read_region_header(buf)?;
    let mut state = FileParseState::new();
    state.failed_chunks += header.corrupt;

    for r in &header.refs {
        process_chunk(buf, r, &mut state)?;
    }

    let chunk_count = header.refs.len() as u64;
    if chunk_count > 0 && state.skipped_legacy == chunk_count {
        return Err(ParseError::Pre118Format);
    }

    let culled = cull_world(&mut state.solid, &state.names);
    let world = ParsedWorld {
        blocks_by_type: culled.blocks_by_type,
        total_blocks: state.total_blocks,
        exterior_blocks: culled.exterior_blocks,
        bounds: culled.bounds,
        regions: vec![file_name.to_string()],
        data_version: state.data_version,
        skipped_lz4_chunks: state.lz4_skipped,
        corrupt_chunks: state.failed_chunks,
        legacy_chunks: state.skipped_legacy,
    };
    Ok(FileParseOutcome {
        world,
        chunk_count,
        decoded_chunks: state.decoded_chunks,
    })
}

/// Merge several single-region results into one `ParsedWorld` (TS `mergeWorlds`,
/// plus summed counters). Names are concatenated in first-seen order across
/// files; `bounds` is `None` only when every part is empty.
pub(crate) fn merge_worlds(parts: Vec<ParsedWorld>) -> ParsedWorld {
    let mut out = ParsedWorld::empty();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    for w in parts {
        out.total_blocks += w.total_blocks;
        out.exterior_blocks += w.exterior_blocks;
        out.regions.extend(w.regions);
        out.skipped_lz4_chunks += w.skipped_lz4_chunks;
        out.corrupt_chunks += w.corrupt_chunks;
        out.legacy_chunks += w.legacy_chunks;
        if let Some(dv) = w.data_version {
            out.data_version = Some(out.data_version.map_or(dv, |cur| cur.max(dv)));
        }
        for (name, arr) in w.blocks_by_type {
            let slot = *index.entry(name.clone()).or_insert_with(|| {
                out.blocks_by_type.push((name, Vec::new()));
                out.blocks_by_type.len() - 1
            });
            out.blocks_by_type[slot].1.extend(arr);
        }
        if let Some((pmin, pmax)) = w.bounds {
            match &mut out.bounds {
                Some((omin, omax)) => {
                    for i in 0..3 {
                        omin[i] = omin[i].min(pmin[i]);
                        omax[i] = omax[i].max(pmax[i]);
                    }
                }
                None => out.bounds = Some((pmin, pmax)),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::section_key;

    /// Build an arena + name table with given (name, x, y, z) world blocks in
    /// chunk (0,0). Local coords must be 0..=15 and section 0.
    fn arena_with(blocks: &[(&str, usize)]) -> (SectionArena, NameTable) {
        let mut arena = SectionArena::new();
        let mut names = NameTable::new();
        for &(name, i) in blocks {
            let id = names.intern(name).unwrap();
            let key = section_key(0, 0, 0);
            let slot = arena.alloc(key) as usize * SECTION_CELLS;
            arena.cells_mut()[slot + i] = (id as u16) + 1;
        }
        (arena, names)
    }

    #[test]
    fn single_block_is_exterior() {
        let (mut arena, names) = arena_with(&[("minecraft:stone", (5 << 8) | (3 << 4) | 2)]);
        let r = cull_world(&mut arena, &names);
        assert_eq!(r.exterior_blocks, 1);
        assert_eq!(r.bounds, Some(([2, 5, 3], [2, 5, 3])));
        assert_eq!(r.blocks_by_type.len(), 1);
        assert_eq!(r.blocks_by_type[0].0, "minecraft:stone");
        assert_eq!(r.blocks_by_type[0].1, vec![[2, 5, 3]]);
    }

    #[test]
    fn buried_center_culled_but_torch_neighbor_saves_face() {
        // 3x3x3 stone cube at local (2..=4)^2... use (1..=3) on each axis:
        // center (2,2,2) has all 6 neighbors stone -> culled; 26 others kept.
        let mut blocks = Vec::new();
        for ly in 1..=3usize {
            for lz in 1..=3usize {
                for lx in 1..=3usize {
                    blocks.push(("minecraft:stone", (ly << 8) | (lz << 4) | lx));
                }
            }
        }
        let (mut arena, names) = arena_with(&blocks);
        let r = cull_world(&mut arena, &names);
        assert_eq!(r.exterior_blocks, 26);
        assert_eq!(r.bounds, Some(([1, 1, 1], [3, 3, 3])));

        // Replace one face neighbor of the center with a torch: the center's
        // face toward the torch is no longer blocked -> center survives.
        let mut blocks: Vec<(&str, usize)> = blocks
            .into_iter()
            .filter(|&(_, i)| i != ((2 << 8) | (3 << 4) | 2)) // remove z+1 neighbor of center
            .collect();
        blocks.push(("minecraft:torch", (2 << 8) | (3 << 4) | 2));
        let (mut arena, names) = arena_with(&blocks);
        let r = cull_world(&mut arena, &names);
        let stone = &r
            .blocks_by_type
            .iter()
            .find(|(n, _)| n == "minecraft:stone")
            .unwrap()
            .1;
        assert!(
            stone.contains(&[2, 2, 2]),
            "center visible through the non-opaque torch"
        );
        let torch = &r
            .blocks_by_type
            .iter()
            .find(|(n, _)| n == "minecraft:torch")
            .unwrap()
            .1;
        assert_eq!(torch.as_slice(), &[[2, 2, 3]], "torch itself is kept");
    }

    #[test]
    fn empty_world_has_no_bounds() {
        let (mut arena, names) = arena_with(&[]);
        let r = cull_world(&mut arena, &names);
        assert_eq!(r.exterior_blocks, 0);
        assert_eq!(r.bounds, None);
        assert!(r.blocks_by_type.is_empty());
    }

    #[test]
    fn merge_keeps_order_and_none_bounds() {
        let a = ParsedWorld {
            blocks_by_type: vec![("minecraft:stone".into(), vec![[1, 2, 3]])],
            total_blocks: 5,
            exterior_blocks: 1,
            bounds: Some(([1, 2, 3], [1, 2, 3])),
            regions: vec!["a.mca".into()],
            data_version: Some(3465),
            skipped_lz4_chunks: 1,
            corrupt_chunks: 2,
            legacy_chunks: 3,
        };
        let b = ParsedWorld {
            blocks_by_type: vec![
                ("minecraft:dirt".into(), vec![[9, 9, 9]]),
                ("minecraft:stone".into(), vec![[-4, 0, 8]]),
            ],
            total_blocks: 7,
            exterior_blocks: 2,
            bounds: Some(([-4, 0, 8], [9, 9, 9])),
            regions: vec!["b.mca".into()],
            data_version: Some(3839),
            skipped_lz4_chunks: 0,
            corrupt_chunks: 0,
            legacy_chunks: 0,
        };
        let m = merge_worlds(vec![a, b]);
        assert_eq!(m.total_blocks, 12);
        assert_eq!(m.exterior_blocks, 3);
        assert_eq!(m.regions, vec!["a.mca".to_string(), "b.mca".to_string()]);
        assert_eq!(m.data_version, Some(3839));
        assert_eq!(m.skipped_lz4_chunks, 1);
        assert_eq!(m.corrupt_chunks, 2);
        assert_eq!(m.legacy_chunks, 3);
        assert_eq!(m.bounds, Some(([-4, 0, 3], [9, 9, 9])));
        let names: Vec<&str> = m.blocks_by_type.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            ["minecraft:stone", "minecraft:dirt"],
            "first-seen order across files"
        );
        assert_eq!(m.blocks_by_type[0].1, vec![[1, 2, 3], [-4, 0, 8]]);
        // merging two empty worlds -> None bounds
        let m2 = merge_worlds(vec![ParsedWorld::empty(), ParsedWorld::empty()]);
        assert_eq!(m2.bounds, None);
    }
}
