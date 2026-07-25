//! Audit how every block in a real world resolves to a Mineways atlas swatch.
//!
//! `get_swatch_for_block` returns `None` for a block it doesn't know, and the
//! tileset builder has to draw it as *something*. That fallback used to be
//! swatch 0 — `grass_block_top` — so unrecognised blocks rendered as grass,
//! which is indistinguishable from real terrain and easy to miss in a render.
//! It is now a neutral stone swatch, but either way the block is wrong, so
//! this lists the failures explicitly rather than leaving them to be spotted
//! by eye.
//!
//! Run with: `cargo run --release --example audit_swatches -p bedrock-parser -- <WorldName>`

use bedrock_parser::chunk::strip_namespace;
use bedrock_parser::detect::Edition;
use bedrock_parser::mineways::{get_swatch_for_block, split_texture_key};
use bedrock_parser::mineways_data::TILE_TABLE;
use bedrock_parser::world::World;
use std::collections::BTreeSet;

fn main() {
    let worlds = bedrock_parser::detect::detect_worlds();
    let target = std::env::args().nth(1);
    let Some(info) = target
        .as_deref()
        .and_then(|n| worlds.iter().find(|w| w.name == n))
        .or_else(|| worlds.first())
    else {
        eprintln!("No worlds found");
        return;
    };
    println!("Auditing world: {} ({:?})", info.name, info.edition);

    let chunks = match info.edition {
        Edition::Java => {
            let world = World::open(&info.folder);
            let mut chunks = Vec::new();
            for (_rx, _rz, path) in world.regions() {
                let Ok(mut region) = bedrock_parser::region::RegionFile::open(&path) else {
                    continue;
                };
                for (lx, lz) in region.present_chunks() {
                    if let Some(Ok(nbt)) = region.chunk_nbt(lx, lz) {
                        if let Ok(chunk) = bedrock_parser::chunk::Chunk::from_nbt(&nbt) {
                            chunks.push(chunk);
                        }
                    }
                }
            }
            chunks
        }
        Edition::Bedrock => bedrock_parser::bedrock::BedrockWorld::open(info.folder.clone())
            .and_then(|w| w.chunks_near(0, 0, 16))
            .unwrap_or_default(),
    };
    println!("Decoded {} chunks", chunks.len());

    let mut keys = BTreeSet::new();
    for c in &chunks {
        for k in c.texture_keys() {
            keys.insert(k);
        }
    }

    let swatch_name = |idx: usize| TILE_TABLE.get(idx).map(|t| t.2).unwrap_or("<out-of-range>");

    let mut unresolved = Vec::new();
    let mut resolved_to_grass = Vec::new();
    let mut ok = 0usize;

    for key in &keys {
        let (name, color) = split_texture_key(key);
        let short = strip_namespace(name);
        if short == "air" || short == "cave_air" || short == "void_air" {
            continue;
        }
        match get_swatch_for_block(short, color) {
            None => unresolved.push(key.clone()),
            Some(faces) => {
                // Swatch 0 on every face means it landed on grass_block_top,
                // which is almost never a legitimate result.
                if faces.iter().all(|&f| f == 0) && short != "grass_block" {
                    resolved_to_grass.push(key.clone());
                } else {
                    ok += 1;
                }
            }
        }
    }

    println!("\n=== {} distinct block states ===", keys.len());
    println!("resolved OK:                 {ok}");
    println!("UNRESOLVED (-> neutral stone): {}", unresolved.len());
    println!("resolved to all-swatch-0:    {}", resolved_to_grass.len());

    if !unresolved.is_empty() {
        println!("\n--- UNRESOLVED (no swatch in terrainExt.png; drawn as stone) ---");
        for k in &unresolved {
            println!("  {k}");
        }
    }
    if !resolved_to_grass.is_empty() {
        println!("\n--- ALL FACES = swatch 0 ---");
        for k in &resolved_to_grass {
            println!("  {k}");
        }
    }

    // Does the tile table actually agree with the atlas image? The table was
    // generated from Mineways' tiles.h; if the vendored terrainExt.png is a
    // different build, indices still resolve but point at the wrong picture.
    // Compare each named swatch's average colour against the independent
    // per-block colour table in `blocks.rs`.
    println!("\n--- table vs. atlas image agreement ---");
    if let Ok((pixels, aw, ah)) = bedrock_parser::mineways::load_terrain_atlas() {
        println!("atlas {}x{} ({} rows of 16)", aw, ah, ah / 16);
        let tile_avg = |col: u32, row: u32| -> [f32; 3] {
            let (mut r, mut g, mut b, mut n) = (0f32, 0f32, 0f32, 0f32);
            for y in row * 16..row * 16 + 16 {
                for x in col * 16..col * 16 + 16 {
                    if x >= aw || y >= ah {
                        continue;
                    }
                    let i = ((y * aw + x) * 4) as usize;
                    if pixels[i + 3] < 128 {
                        continue; // ignore transparent pixels
                    }
                    r += pixels[i] as f32;
                    g += pixels[i + 1] as f32;
                    b += pixels[i + 2] as f32;
                    n += 1.0;
                }
            }
            if n == 0.0 {
                return [0.0; 3];
            }
            [r / n / 255.0, g / n / 255.0, b / n / 255.0]
        };

        let mut mismatches = 0;
        for probe in [
            "stone", "bedrock", "gravel", "sand", "dirt", "deepslate", "tuff", "calcite",
            "andesite", "granite", "diorite", "cobblestone", "obsidian", "clay", "netherrack",
        ] {
            let Some(faces) = get_swatch_for_block(probe, None) else {
                continue;
            };
            let Some(&(col, row, tname)) = TILE_TABLE.get(faces[2]) else {
                continue;
            };
            let got = tile_avg(col, row);
            let want = bedrock_parser::blocks::block_color(probe);
            // Compare hue-ish: large per-channel divergence means wrong tile.
            let d = (got[0] - want[0]).abs() + (got[1] - want[1]).abs() + (got[2] - want[2]).abs();
            let flag = if d > 0.45 { "  <-- MISMATCH" } else { "" };
            if d > 0.45 {
                mismatches += 1;
            }
            println!(
                "  {probe:14} swatch#{:<5} {tname:22} atlas=({:.2},{:.2},{:.2}) expected=({:.2},{:.2},{:.2}) d={d:.2}{flag}",
                faces[2], got[0], got[1], got[2], want[0], want[1], want[2]
            );
        }
        println!("  => {mismatches} mismatched");
    }

    // Spot-check a few well-known blocks so a systemic mapping break is obvious.
    println!("\n--- spot check ---");
    for probe in [
        "grass_block",
        "stone",
        "deepslate",
        "oak_leaves",
        "water",
        "lava",
        "sand",
        "glow_lichen",
        "leaf_litter",
        "bubble_column",
        "amethyst_block",
        "tuff",
    ] {
        match get_swatch_for_block(probe, None) {
            Some(f) => println!(
                "  {probe:16} top={} side={} bottom={}",
                swatch_name(f[0]),
                swatch_name(f[2]),
                swatch_name(f[1])
            ),
            None => println!("  {probe:16} UNRESOLVED"),
        }
    }
}
