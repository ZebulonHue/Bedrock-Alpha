use chunkforge_core::parse_region_paths;

#[test]
fn debug_real_world() {
    let path = r"C:\Users\zebby\AppData\Local\Temp\opencode\r.0.0.mca";
    match parse_region_paths(&[path]) {
        Ok(world) => {
            println!("OK: {} total blocks", world.total_blocks);
            println!("OK: {} exterior blocks", world.exterior_blocks);
            println!("OK: {} region files", world.regions.len());
            println!("DataVersion: {:?}", world.data_version);
            println!("Legacy chunks skipped: {}", world.legacy_chunks);
            println!("Corrupt chunks: {}", world.corrupt_chunks);
            println!("LZ4 skipped: {}", world.skipped_lz4_chunks);
            println!("Block types:");
            for (name, pos) in &world.blocks_by_type {
                println!("  {}: {} blocks", name, pos.len());
            }
        }
        Err(e) => {
            println!("ERROR: {:?}", e);
        }
    }
}
