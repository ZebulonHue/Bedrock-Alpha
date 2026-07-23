//! Integration tests against the fixtures copied from the ChunkForge app
//! repo (`test/fixture.mca`, `test/fixture-legacy.mca`, `test/fixture-big.mca`,
//! `test/real/r.0.0.mca` — MC 1.20.6). Expected numbers are the exact values
//! the TypeScript parser produces (see SPEC §Validation).

use chunkforge_core::{
    appearance, parse_region_bytes, parse_region_paths, ParseError, ParsedWorld,
};

const FIXTURE: &[u8] = include_bytes!("fixtures/fixture.mca");
const FIXTURE_LEGACY: &[u8] = include_bytes!("fixtures/fixture-legacy.mca");
const FIXTURE_BIG: &[u8] = include_bytes!("fixtures/fixture-big.mca");
const REAL: &[u8] = include_bytes!("fixtures/real/r.0.0.mca");

fn blocks<'w>(world: &'w ParsedWorld, name: &str) -> Option<&'w Vec<[i32; 3]>> {
    world
        .blocks_by_type
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v)
}

fn has(arr: &Option<&Vec<[i32; 3]>>, x: i32, y: i32, z: i32) -> bool {
    arr.is_some_and(|v| v.contains(&[x, y, z]))
}

// ---------- fixture.mca ----------

#[test]
fn fixture_mca_exact_numbers() {
    let world = parse_region_bytes(FIXTURE, "fixture.mca").expect("parse fixture.mca");

    assert_eq!(world.total_blocks, 74, "totalBlocks");
    assert_eq!(world.exterior_blocks, 66, "exteriorBlocks");
    assert_eq!(world.legacy_chunks, 1, "1 legacy chunk skipped");
    assert_eq!(world.corrupt_chunks, 0);
    assert_eq!(world.skipped_lz4_chunks, 0);
    assert_eq!(world.data_version, Some(3465), "dataVersion 3465 (1.20.1)");
    assert_eq!(world.regions, vec!["fixture.mca".to_string()]);

    let stone = blocks(&world, "minecraft:stone");
    assert_eq!(stone.map(Vec::len), Some(56), "stone exterior count");
    assert!(has(&stone, 2, 2, 2), "surface stone (2,2,2) present");
    assert!(has(&stone, 3, 5, 3), "stone under torch (3,5,3) present");
    assert!(!has(&stone, 3, 3, 3), "fully buried stone (3,3,3) culled");
    assert!(!has(&stone, 4, 4, 4), "fully buried stone (4,4,4) culled");

    let torch = blocks(&world, "minecraft:torch");
    assert_eq!(torch.map(Vec::len), Some(1));
    assert!(has(&torch, 3, 6, 3), "torch at (3,6,3), never culled");

    let sapling = blocks(&world, "minecraft:oak_sapling");
    assert_eq!(sapling.map(Vec::len), Some(1));
    assert!(has(&sapling, 7, 2, 7), "oak_sapling at (7,2,7)");

    let end_stone = blocks(&world, "minecraft:end_stone");
    assert_eq!(end_stone.map(Vec::len), Some(8), "end_stone count");
    assert!(has(&end_stone, 16, 0, 0) && has(&end_stone, 17, 1, 1));

    assert_eq!(world.bounds, Some(([2, 0, 0], [17, 6, 7])), "bounds");
}

// ---------- fixture-legacy.mca ----------

#[test]
fn legacy_only_region_is_pre118_error() {
    let err = parse_region_bytes(FIXTURE_LEGACY, "fixture-legacy.mca")
        .expect_err("legacy-only region must fail");
    assert!(matches!(err, ParseError::Pre118Format));
    assert_eq!(
        err.to_string(),
        "This save uses the pre-1.18 chunk format, which is not supported yet.",
        "exact TS LEGACY_FORMAT_ERROR message"
    );
}

// ---------- fixture-big.mca ----------

#[test]
fn fixture_big_exact_numbers() {
    let world = parse_region_bytes(FIXTURE_BIG, "fixture-big.mca").expect("parse fixture-big.mca");
    assert_eq!(world.total_blocks, 2_097_152, "totalBlocks");
    assert_eq!(world.exterior_blocks, 218_448, "exteriorBlocks");
    assert_eq!(world.data_version, Some(3465));
}

// ---------- real/r.0.0.mca (MC 1.20.6) ----------

#[test]
fn real_region_matches_ts_reference() {
    // TS reference (measured via test/reference-dump.ts in the app repo):
    // totalBlocks=1,062,650, exteriorBlocks=123,956, dataVersion=3839,
    // 35 block types, bounds min [0,-64,0] max [95,81,95].
    let world = parse_region_bytes(REAL, "r.0.0.mca").expect("parse r.0.0.mca");
    assert_eq!(world.total_blocks, 1_062_650, "totalBlocks");
    assert_eq!(
        world.exterior_blocks, 123_956,
        "exteriorBlocks (TS reference)"
    );
    assert_eq!(world.data_version, Some(3839), "dataVersion 3839 (1.20.6)");
    assert_eq!(world.blocks_by_type.len(), 35, "35 block types");
    assert_eq!(world.bounds, Some(([0, -64, 0], [95, 81, 95])), "bounds");
}

// ---------- multi-file API ----------

#[test]
fn paths_api_merges_and_matches_bytes_api() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let single = parse_region_paths(&[format!("{dir}/fixture.mca")]).expect("parse paths");
    assert_eq!(single.total_blocks, 74);
    assert_eq!(single.exterior_blocks, 66);
    assert_eq!(
        single.regions,
        vec!["fixture.mca".to_string()],
        "bare file name"
    );

    let merged = parse_region_paths(&[
        format!("{dir}/fixture.mca"),
        format!("{dir}/fixture-big.mca"),
    ])
    .expect("merge two fixtures");
    assert_eq!(merged.total_blocks, 74 + 2_097_152);
    assert_eq!(merged.exterior_blocks, 66 + 218_448);
    assert_eq!(
        merged.regions,
        vec!["fixture.mca".to_string(), "fixture-big.mca".to_string()]
    );
    assert_eq!(merged.data_version, Some(3465));
    // stone appears once, with fixture.mca's 56 entries first
    let stone = blocks(&merged, "minecraft:stone").expect("stone present");
    assert!(
        stone.contains(&[3, 5, 3]),
        "first file's blocks keep their coords"
    );
    assert!(merged.bounds.is_some());
}

/// A region whose only chunk record inflates to garbage: every chunk corrupt.
fn corrupt_only_region() -> Vec<u8> {
    let mut buf = vec![0u8; 8192 + 4096];
    // header entry 0: sector offset 2, count 1
    buf[0..4].copy_from_slice(&((2u32 << 8) | 1).to_be_bytes());
    let pos = 2 * 4096;
    let payload = b"this is not zlib data at all";
    let length = (1 + payload.len()) as u32;
    buf[pos..pos + 4].copy_from_slice(&length.to_be_bytes());
    buf[pos + 4] = 2; // zlib
    buf[pos + 5..pos + 5 + payload.len()].copy_from_slice(payload);
    buf
}

#[test]
fn corrupt_only_buffer_is_no_chunks_decoded() {
    let buf = corrupt_only_region();
    let err = parse_region_bytes(&buf, "corrupt.mca").expect_err("must fail");
    assert!(matches!(err, ParseError::NoChunksDecoded), "got: {err}");
}

#[test]
fn multi_file_tolerates_corrupt_sibling() {
    // Judgment call (documented in PARSING.md): a corrupt-only sibling is
    // tolerated when another file decodes; the NoChunksDecoded check applies
    // to the merged totals.
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");
    let good = format!("{dir}/fixture.mca");
    // Write the corrupt-only region to a temp file next to nothing shared.
    let tmp = std::env::temp_dir().join("chunkforge-corrupt-only-test.mca");
    std::fs::write(&tmp, corrupt_only_region()).unwrap();
    let world = parse_region_paths(&[&good, &tmp.to_string_lossy().into_owned()])
        .expect("corrupt sibling tolerated");
    assert_eq!(world.total_blocks, 74);
    assert_eq!(world.exterior_blocks, 66);
    assert_eq!(world.corrupt_chunks, 1, "corrupt chunk counted");
    assert_eq!(world.regions.len(), 2);
    std::fs::remove_file(&tmp).ok();

    // ...but a legacy-only sibling still fails the whole parse (TS behavior).
    let legacy = format!("{dir}/fixture-legacy.mca");
    let err = parse_region_paths(&[&good, &legacy]).expect_err("legacy sibling fails");
    assert!(matches!(err, ParseError::Pre118Format));
}

// ---------- misc API behavior ----------

#[test]
fn truncated_buffer_rejected() {
    let err = parse_region_bytes(&[0u8; 100], "tiny.mca").expect_err("too small");
    assert!(matches!(err, ParseError::Truncated));
}

#[test]
fn empty_region_is_empty_world_not_error() {
    // Header only, no chunks: not an error, and bounds stay None (the TS
    // merge produces fake [0,0,0] bounds here — a latent bug we deliberately
    // do NOT reproduce).
    let buf = vec![0u8; 8192];
    let world = parse_region_bytes(&buf, "empty.mca").expect("empty region parses");
    assert_eq!(world.total_blocks, 0);
    assert_eq!(world.exterior_blocks, 0);
    assert_eq!(world.bounds, None);
    assert!(world.blocks_by_type.is_empty());
    assert_eq!(world.data_version, None);
}

#[test]
fn appearance_public_contract() {
    let a = appearance("minecraft:torch");
    assert!(!a.opaque, "torches never occlude");
    assert_eq!(a.color, [0xff, 0xc8, 0x47]);
    assert_eq!(
        a.texture_path,
        Some("assets/minecraft/textures/block/torch.png")
    );
    assert!(appearance("minecraft:stone").opaque);
    assert!(!appearance("minecraft:oak_stairs").opaque, "hint fallback");
}
