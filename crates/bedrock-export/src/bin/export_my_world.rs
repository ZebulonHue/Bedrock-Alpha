use bedrock_export::obj::{export_obj_with_options, ExportOptions, ExportRegion};
use bedrock_parser::bedrock::BedrockWorld;
use bedrock_parser::detect::Edition;
use bedrock_parser::mineways::build_mineways_tileset;
use bedrock_parser::world::World;
use std::path::PathBuf;

fn main() {
    // Surface the exporter's own warnings (unmapped blocks, JAR fallbacks)
    // instead of swallowing them.
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let worlds = bedrock_parser::detect::detect_worlds();
    if worlds.is_empty() {
        eprintln!("No worlds found!");
        return;
    }
    let target_name = std::env::args().nth(1);
    // A named world that does not match must be an error, never a fallback.
    // Silently exporting a different save than the one asked for produces an
    // export that looks plausible and is wrong, which is far more expensive to
    // notice than a failed command. Matching is case-insensitive and ignores
    // spaces vs underscores so the name shown in-game works as typed.
    let normalise = |s: &str| {
        s.to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .collect::<String>()
    };
    let world_info = match target_name.as_deref() {
        Some(name) => match worlds.iter().find(|w| normalise(&w.name) == normalise(name)) {
            Some(world) => world,
            None => {
                eprintln!("No world named {name:?}. Available worlds:");
                for world in &worlds {
                    eprintln!("  {} ({:?})", world.name, world.edition);
                }
                std::process::exit(1);
            }
        },
        None => &worlds[0],
    };
    println!(
        "Exporting world: {} ({:?}) at {:?}",
        world_info.name, world_info.edition, world_info.folder
    );

    let chunks = match world_info.edition {
        Edition::Java => {
            let world = World::open(&world_info.folder);
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
        Edition::Bedrock => {
            let world = BedrockWorld::open(world_info.folder.clone()).unwrap();
            world.chunks_near(0, 0, 16).unwrap()
        }
    };

    println!("Decoded {} chunks", chunks.len());
    if chunks.is_empty() {
        return;
    }

    let mut keys_set = std::collections::HashSet::new();
    for c in &chunks {
        for k in c.texture_keys() {
            keys_set.insert(k);
        }
    }
    let texture_keys: Vec<String> = keys_set.into_iter().collect();
    let tiles = build_mineways_tileset(&texture_keys);

    // Optional second arg: half-width in blocks of the exported square.
    let radius: i32 = std::env::args()
        .nth(2)
        .and_then(|a| a.parse().ok())
        .unwrap_or(500);

    // Centre on the content, not the world origin. A save's interesting area
    // is wherever the player actually played; a box around (0, 0) just yields
    // whatever terrain sits at spawn. Prefer the player position, fall back
    // to the median of the generated chunks (robust to a few stray chunks far
    // from everything else), and allow an explicit override.
    let explicit: Option<(i32, i32)> = match (std::env::args().nth(3), std::env::args().nth(4)) {
        (Some(x), Some(z)) => x.parse().ok().zip(z.parse().ok()),
        _ => None,
    };
    let (cx, cz) = if let Some(centre) = explicit {
        centre
    } else if let Some([x, _, z]) = info_player_pos(world_info) {
        println!("Centring on the player position");
        (x.round() as i32, z.round() as i32)
    } else {
        let mut xs: Vec<i32> = chunks.iter().map(|c| c.x).collect();
        let mut zs: Vec<i32> = chunks.iter().map(|c| c.z).collect();
        xs.sort_unstable();
        zs.sort_unstable();
        let median = |v: &[i32]| v.get(v.len() / 2).copied().unwrap_or(0) * 16;
        println!("No player position in this save — centring on the median generated chunk");
        (median(&xs), median(&zs))
    };
    let export_region = ExportRegion {
        min: [cx - radius, -64, cz - radius],
        max: [cx + radius, 320, cz + radius],
    };
    println!("Export region: +/-{radius} blocks around ({cx}, {cz}), y -64..320");

    // Exports run to hundreds of megabytes plus an atlas and a prototype tree,
    // so they go to a dedicated drive rather than a synced folder. Override
    // with PB_EXPORT_DIR.
    let dest_dir = std::env::var("PB_EXPORT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(r"D:\Bedrock exports"));
    if let Err(err) = std::fs::create_dir_all(&dest_dir) {
        eprintln!("could not create {}: {err}", dest_dir.display());
        return;
    }
    // Name the output after the world so two exports can't overwrite each
    // other, and so an OBJ is always paired with its own manifest.
    let safe_name: String = world_info
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let dest = dest_dir.join(format!("{safe_name}.obj"));
    let t0 = std::time::Instant::now();
    let options = ExportOptions {
        write_block_manifest: true,
        write_prototypes: true,
    };
    match export_obj_with_options(&chunks, &export_region, &dest, &tiles, &options) {
        Ok(stats) => println!(
            "Successfully exported {} blocks, {} faces, {} materials to {:?} in {:.1}s",
            stats.blocks,
            stats.faces,
            stats.materials,
            stats.obj_path,
            t0.elapsed().as_secs_f64(),
        ),
        Err(e) => eprintln!("Export error: {e}"),
    }
}

/// Player position from a world's `level.dat`, if it has one.
fn info_player_pos(info: &bedrock_parser::detect::WorldSummary) -> Option<[f64; 3]> {
    match info.edition {
        Edition::Java => World::open(info.folder.clone())
            .level_meta()
            .ok()
            .and_then(|meta| meta.player_pos),
        Edition::Bedrock => BedrockWorld::open(info.folder.clone())
            .ok()
            .and_then(|w| w.player_pos()),
    }
}
