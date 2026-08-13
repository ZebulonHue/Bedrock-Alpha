//! Measure how much geometry a real world meshes into, with and without the
//! memory budget.
//!
//! Usage:
//!     cargo run --release -p bedrock-render --example mesh_budget -- <world dir> [radius]

use bedrock_parser::chunk::Chunk;
use bedrock_parser::jar_textures::JarTextureLoader;
use bedrock_parser::region::RegionFile;
use bedrock_parser::texture::FaceAwareTileSet;
use bedrock_parser::world::World;
use bedrock_render::mesh::{mesh_within_budget, MAX_MESH_BYTES};

fn main() {
    let mut args = std::env::args().skip(1);
    let dir = args.next().expect("world directory");
    let radius: i32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(48);

    let world = World::open(std::path::PathBuf::from(&dir));
    let (cx, cz) = world
        .level_meta()
        .ok()
        .and_then(|m| m.player_pos)
        .map(|p| ((p[0] / 16.0).floor() as i32, (p[2] / 16.0).floor() as i32))
        .unwrap_or((0, 0));

    let mut chunks: Vec<Chunk> = Vec::new();
    for (rx, rz, path) in world.regions() {
        if rx < (cx - radius).div_euclid(32)
            || rx > (cx + radius).div_euclid(32)
            || rz < (cz - radius).div_euclid(32)
            || rz > (cz + radius).div_euclid(32)
        {
            continue;
        }
        let Ok(mut region) = RegionFile::open(&path) else { continue };
        for (lx, lz) in region.present_chunks() {
            let (wx, wz) = (rx * 32 + i32::from(lx), rz * 32 + i32::from(lz));
            if (wx - cx).abs() > radius || (wz - cz).abs() > radius {
                continue;
            }
            if let Some(Ok(nbt)) = region.chunk_nbt(lx, lz) {
                if let Ok(chunk) = Chunk::from_nbt(&nbt) {
                    chunks.push(chunk);
                }
            }
        }
    }
    chunks.sort_by_key(|c| {
        let (dx, dz) = ((c.x - cx) as i64, (c.z - cz) as i64);
        dx * dx + dz * dz
    });
    println!("centre ({cx}, {cz}) radius {radius}: {} chunks", chunks.len());

    let names: Vec<String> = chunks
        .iter()
        .flat_map(|c| c.block_names().into_iter().map(str::to_owned))
        .collect();
    let tiles = FaceAwareTileSet::build(names, &JarTextureLoader::empty());

    for (label, budget) in [("unbounded", u64::MAX), ("with budget", MAX_MESH_BYTES)] {
        let start = std::time::Instant::now();
        let (meshes, skipped) = mesh_within_budget(&chunks, &tiles, budget);
        let bytes: u64 = meshes
            .iter()
            .map(|m| (m.vertices.len() * 32 + m.indices.len() * 4) as u64)
            .sum();
        let verts: usize = meshes.iter().map(|m| m.vertices.len()).sum();
        println!(
            "  {label:<12} {:>6.2} GB  {:>12} verts  {:>5} meshed  {:>5} skipped  {:>5.1}s",
            bytes as f64 / 1e9,
            verts,
            meshes.len(),
            skipped,
            start.elapsed().as_secs_f64()
        );
    }
}
