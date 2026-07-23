//! Debug helper: opens a Bedrock world folder and prints what the reader
//! finds. Run with:
//! `cargo run --example inspect_bedrock -p bedrock-parser -- <world-folder>`

use bedrock_parser::bedrock::BedrockWorld;

fn main() {
    let folder = std::env::args()
        .nth(1)
        .expect("usage: inspect_bedrock <world-folder>");
    let world = BedrockWorld::open(&folder).expect("open world");
    println!("player_pos: {:?}", world.player_pos());
    let center = world
        .player_pos()
        .map(|p| ((p[0] / 16.0).floor() as i32, (p[2] / 16.0).floor() as i32))
        .unwrap_or((0, 0));
    let chunks = world
        .chunks_near(center.0, center.1, 10)
        .expect("read chunks");
    println!("chunks near {center:?}: {}", chunks.len());
    if let Some(chunk) = chunks.first() {
        println!(
            "first chunk at ({}, {}), y_range {:?}",
            chunk.x,
            chunk.z,
            chunk.y_range()
        );
        let mut names = chunk.block_names();
        names.sort();
        println!("block names (first chunk): {names:?}");
    }
}
