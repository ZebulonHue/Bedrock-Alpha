use bedrock_export::obj::{export_obj, ExportRegion};
use bedrock_parser::bedrock::BedrockWorld;
use bedrock_parser::detect::Edition;
use bedrock_parser::mineways::build_mineways_tileset;
use bedrock_parser::world::World;
use std::path::PathBuf;

fn main() {
    let worlds = bedrock_parser::detect::detect_worlds();
    if worlds.is_empty() {
        eprintln!("No worlds found!");
        return;
    }
    let world_info = &worlds[0];
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

    let export_region = ExportRegion {
        min: [-500, -64, -500],
        max: [500, 320, 500],
    };

    let dest = PathBuf::from(r"C:\Users\zebby\OneDrive\Documents\Project Bedrock\My_World.obj");
    match export_obj(&chunks, &export_region, &dest, &tiles) {
        Ok(stats) => println!(
            "Successfully exported {} blocks, {} faces to {:?}",
            stats.blocks, stats.faces, stats.obj_path
        ),
        Err(e) => eprintln!("Export error: {e}"),
    }
}
