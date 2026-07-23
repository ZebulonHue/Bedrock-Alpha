//! Timing harness: parse fixture-big.mca (2,097,152 blocks) and print the
//! wall time. Run: `cargo run --example time_big [--release]`

use std::time::Instant;

fn main() {
    let buf = include_bytes!("../tests/fixtures/fixture-big.mca");
    let t0 = Instant::now();
    let world = chunkforge_core::parse_region_bytes(buf, "fixture-big.mca").expect("parse");
    let elapsed = t0.elapsed();
    println!(
        "fixture-big.mca: {} total / {} exterior blocks in {:.1} ms",
        world.total_blocks,
        world.exterior_blocks,
        elapsed.as_secs_f64() * 1000.0
    );
}
