//! Which atlas swatch a named block resolves to, and what that tile is.
//!
//! Blocks whose true shape is a full cube never reach the prototype path --
//! they are drawn from the shared atlas instead, so when one renders in the
//! wrong colour the cause is its swatch mapping, not its geometry. This
//! prints the resolved swatch per face alongside the tile name each index
//! actually holds, so a block pointing at someone else's picture is visible
//! rather than inferred.
//!
//! Run: `cargo run -p bedrock-parser --example audit_atlas_blocks -- cherry_leaves cherry_log`

use bedrock_parser::mineways::get_swatch_for_block;
use bedrock_parser::mineways_data::TILE_TABLE;

fn main() {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let names = if names.is_empty() {
        vec![
            "cherry_leaves".to_owned(),
            "cherry_log".to_owned(),
            "acacia_log".to_owned(),
            "oak_log".to_owned(),
            "oak_leaves".to_owned(),
        ]
    } else {
        names
    };

    let tile_name = |index: usize| -> String {
        TILE_TABLE
            .get(index)
            .map(|t| format!("{} (#{index})", t.2))
            .unwrap_or_else(|| format!("<out of range> (#{index})"))
    };

    for name in names {
        match get_swatch_for_block(&name, None) {
            Some(faces) => {
                println!("{name}:");
                // Face order is CUBE_FACES order, which starts with the
                // vertical pair: +Y, -Y, then the four sides.
                for (label, index) in ["up", "down", "side", "side", "side", "side"]
                    .iter()
                    .zip(faces.iter())
                {
                    println!("    {label:<6} -> {}", tile_name(*index));
                }
            }
            None => println!("{name}: NO SWATCH — drawn as the neutral fallback"),
        }
    }
}
